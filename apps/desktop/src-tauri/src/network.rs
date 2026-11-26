use crate::{
    identity::{fingerprint, NOISE_PAIR, NOISE_RECONNECT},
    model::*,
    state::Core,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch, Semaphore},
};
use uuid::Uuid;

const MAX_FRAME: usize = 65_535;
const MAX_FILE: u64 = 128 * 1024 * 1024;
const CHUNK: usize = 24 * 1024;
const MAX_CLIPBOARD: usize = 24 * 1024;
const TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const INVITATION_PREFIX: &str = "traplink:v1:";
const MAX_INVITATION_LENGTH: usize = 2048;
const MAX_INVITATION_CANDIDATES: usize = 8;
const INVITATION_LIFETIME: Duration = Duration::from_secs(300);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(12);
const DIRECT_ATTEMPT: Duration = Duration::from_secs(4);
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
const MAX_SCAN_SUBNETS: usize = 4;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EncodedInvitation {
    candidates: Vec<String>,
    key: String,
    public_key: String,
    expires_at: i64,
}
struct ParsedInvitation {
    candidates: Vec<SocketAddr>,
    key: [u8; 32],
    public_key: [u8; 32],
}

fn decode_invitation(invitation: &str, now: i64) -> Result<ParsedInvitation, String> {
    if invitation.len() > MAX_INVITATION_LENGTH {
        return Err("The invitation is too long. Copy a new invitation from TrapLink.".into());
    }
    let invitation = invitation.trim();
    let value = if let Some(value) = invitation.strip_prefix(INVITATION_PREFIX) {
        value
    } else if invitation.starts_with("traplink:") {
        return Err(
            "This invitation version is not supported. Update TrapLink on both computers.".into(),
        );
    } else {
        return Err("Paste the complete TrapLink invitation, starting with traplink:v1:.".into());
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "The invitation is invalid. Copy the complete invitation again.")?;
    let encoded: EncodedInvitation = serde_json::from_slice(&bytes)
        .map_err(|_| "The invitation is invalid. Copy the complete invitation again.")?;
    if encoded.expires_at <= now {
        return Err("This invitation has expired. Create a new one on the other computer.".into());
    }
    if encoded.expires_at > now.saturating_add(INVITATION_LIFETIME.as_secs() as i64) {
        return Err("The invitation expiry is invalid. Check both computer clocks and create a new invitation.".into());
    }
    if encoded.candidates.is_empty() || encoded.candidates.len() > MAX_INVITATION_CANDIDATES {
        return Err("The invitation must contain between 1 and 8 local addresses.".into());
    }
    let mut candidates = Vec::with_capacity(encoded.candidates.len());
    for candidate in encoded.candidates {
        let address = allowed_address(&candidate)?;
        if candidates.contains(&address) {
            return Err("The invitation contains duplicate addresses.".into());
        }
        candidates.push(address);
    }
    let decode_key = |value: &str| -> Result<[u8; 32], String> {
        URL_SAFE_NO_PAD
            .decode(value)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| "The invitation contains an invalid security key.".into())
    };
    Ok(ParsedInvitation {
        candidates,
        key: decode_key(&encoded.key)?,
        public_key: decode_key(&encoded.public_key)?,
    })
}

fn encode_invitation(encoded: &EncodedInvitation) -> Result<String, String> {
    let bytes = serde_json::to_vec(encoded).map_err(|_| "Cannot create invitation.")?;
    let invitation = format!("{INVITATION_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes));
    decode_invitation(&invitation, chrono::Utc::now().timestamp())?;
    Ok(invitation)
}

pub struct Invite {
    pub code: String,
    pub key: [u8; 32],
    pub expires: Instant,
    reserved: bool,
}
struct InviteReservation {
    core: Arc<Core>,
    key: Option<[u8; 32]>,
}
impl Drop for InviteReservation {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            if let Ok(mut guard) = self.core.invite.lock() {
                if let Some(invite) = guard.as_mut().filter(|i| i.key == key) {
                    invite.reserved = false;
                }
            }
        }
    }
}
pub struct Session {
    pub tx: mpsc::Sender<Outgoing>,
    pub cancel: watch::Sender<bool>,
    pub generation: Uuid,
    pub rtt_ms: Arc<AtomicU32>,
    pub farewell: Arc<AtomicBool>,
    pub protocol: u32,
}
impl Session {
    pub fn new(tx: mpsc::Sender<Outgoing>, cancel: watch::Sender<bool>, generation: Uuid) -> Self {
        Self {
            tx,
            cancel,
            generation,
            rtt_ms: Arc::new(AtomicU32::new(0)),
            farewell: Arc::new(AtomicBool::new(false)),
            protocol: crate::link::PROTOCOL,
        }
    }
    fn stop_untagged(&self) {
        self.notify_stop(Wire::ControlStop, true);
    }
    pub(crate) fn notify_input_stop(&self, active: bool) {
        let message = if self.protocol >= crate::link::SCOPED_FLOW {
            Wire::EmergencyStop
        } else {
            Wire::ControlStop
        };
        self.notify_stop(message, active);
    }
    fn notify_stop(&self, message: Wire, active: bool) {
        if self
            .tx
            .try_send(Outgoing {
                message,
                result: None,
            })
            .is_err()
            && active
        {
            let _ = self.cancel.send(true);
        }
    }
}
pub struct Outgoing {
    pub message: Wire,
    pub result: Option<oneshot::Sender<Result<(), String>>>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowPacket {
    Input { event: InputEvent },
    Keepalive,
    Return { entry: f64 },
    Stop,
}

impl FlowPacket {
    fn into_wire(self, session: Option<Uuid>) -> Wire {
        if let Some(session) = session {
            return Wire::Flow {
                session,
                packet: self,
            };
        }
        match self {
            Self::Input { event } => Wire::Input { event },
            Self::Keepalive => Wire::InputKeepalive,
            Self::Return { entry } => Wire::FlowReturn { entry },
            Self::Stop => Wire::ControlStop,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum ClipboardFormat {
    Image { width: usize, height: usize },
    Files(Vec<(String, u64)>),
}

impl ClipboardFormat {
    fn validate(&self, bytes: u64) -> Result<(), String> {
        let valid = match self {
            Self::Image { width, height } => {
                *width > 0
                    && *height > 0
                    && width.checked_mul(*height).and_then(|n| n.checked_mul(4))
                        == usize::try_from(bytes).ok()
            }
            Self::Files(files) => {
                !files.is_empty()
                    && files.len() <= 32
                    && files.iter().all(|(name, _)| safe_file_name(name).is_ok())
                    && files
                        .iter()
                        .try_fold(0u64, |total, (_, n)| total.checked_add(*n))
                        == Some(bytes)
            }
        };
        if valid && bytes <= MAX_FILE {
            Ok(())
        } else {
            Err("Invalid clipboard dimensions, file list or size (maximum 128 MiB).".into())
        }
    }
}

fn permit_transfer(core: &Core, id: &str, format: Option<&ClipboardFormat>) -> Result<(), String> {
    if format.is_some() {
        permit(core, id, false)?;
    }
    if !matches!(format, Some(ClipboardFormat::Image { .. })) {
        permit(core, id, true)?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Wire {
    Hello {
        id: String,
        name: String,
        platform: String,
        port: u16,
        #[serde(default)]
        protocol: u32,
    },
    Clipboard {
        request: String,
        text: String,
    },
    FileStart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clipboard: Option<ClipboardFormat>,
        request: String,
        name: String,
        bytes: u64,
    },
    FileChunk {
        request: String,
        data: String,
    },
    FileEnd {
        request: String,
    },
    FileAbort {
        request: String,
    },
    Ack {
        request: String,
        error: Option<String>,
    },
    ControlStart {
        request: String,
    },
    ControlStop,
    EmergencyStop,
    Input {
        event: InputEvent,
    },
    InputKeepalive,
    FlowStart {
        request: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<Uuid>,
        edge: FlowEdge,
        entry: f64,
        clipboard: bool,
    },
    FlowReturn {
        entry: f64,
    },
    Flow {
        session: Uuid,
        packet: FlowPacket,
    },
    Ping,
    Probe {
        id: u64,
    },
    ProbeReply {
        id: u64,
    },
    Goodbye,
    InputConsent {
        request: String,
        #[serde(default)]
        flow: bool,
    },
}

pub fn create_invite(core: &Arc<Core>) -> Result<PairingInvite, String> {
    create_invite_for_addresses(core, core.addresses())
}

fn create_invite_for_addresses(
    core: &Arc<Core>,
    addresses: Vec<String>,
) -> Result<PairingInvite, String> {
    let identity = core
        .identity
        .as_ref()
        .ok_or("OS credential store is unavailable. Pairing is disabled.")?;
    if !core.listening.load(Ordering::Relaxed) {
        return Err(
            "The local listener is not running. Check the warning and port setting.".into(),
        );
    }
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    let code = URL_SAFE_NO_PAD.encode(key);
    let candidates: Vec<_> = addresses
        .into_iter()
        .take(MAX_INVITATION_CANDIDATES)
        .collect();
    let address = candidates.first().cloned().ok_or(
        "No private network address is available. Connect this computer to your local network and try again.",
    )?;
    let expires_at =
        chrono::Utc::now() + chrono::Duration::seconds(INVITATION_LIFETIME.as_secs() as i64);
    let invitation = encode_invitation(&EncodedInvitation {
        candidates,
        key: code.clone(),
        public_key: URL_SAFE_NO_PAD.encode(&identity.public),
        expires_at: expires_at.timestamp(),
    })?;
    *core
        .invite
        .lock()
        .map_err(|_| "Pairing lock unavailable.")? = Some(Invite {
        code: code.clone(),
        key,
        expires: Instant::now() + INVITATION_LIFETIME,
        reserved: false,
    });
    Ok(PairingInvite {
        invitation,
        code,
        address,
        expires_at: expires_at.to_rfc3339(),
        fingerprint: fingerprint(&identity.public),
    })
}
pub fn cancel_invite(core: &Arc<Core>) {
    if let Ok(mut invite) = core.invite.lock() {
        *invite = None;
    }
}

fn allowed_address(address: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = address
        .parse()
        .map_err(|_| "Use a numeric local address and port, for example 192.168.1.20:48372.")?;
    let allowed = match address.ip() {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    };
    if !allowed || address.port() < 1024 {
        return Err(
            "Pairing is limited to local/private IP addresses and ports above 1023.".into(),
        );
    }
    Ok(address)
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    let len = reader
        .read_u32()
        .await
        .map_err(|_| "Peer disconnected.".to_string())? as usize;
    if len == 0 || len > MAX_FRAME {
        return Err("Invalid network frame length.".into());
    }
    let mut frame = vec![0u8; len];
    reader
        .read_exact(&mut frame)
        .await
        .map_err(|_| "Peer disconnected during a message.".to_string())?;
    Ok(frame)
}
async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &[u8]) -> Result<(), String> {
    if frame.is_empty() || frame.len() > MAX_FRAME {
        return Err("Network message exceeds the frame limit.".into());
    }
    let mut buffer = Vec::with_capacity(4 + frame.len());
    buffer.extend_from_slice(&(frame.len() as u32).to_be_bytes());
    buffer.extend_from_slice(frame);
    write_prefixed(writer, &buffer).await
}
async fn write_prefixed<W: AsyncWrite + Unpin>(
    writer: &mut W,
    buffer: &[u8],
) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(3), writer.write_all(buffer))
        .await
        .map_err(|_| "Peer stopped reading; connection closed.".to_string())?
        .map_err(|_| "Cannot write to peer.".to_string())
}
async fn write_wire<W: AsyncWrite + Unpin>(
    stream: &mut W,
    noise: &mut snow::TransportState,
    message: &Wire,
) -> Result<(), String> {
    let plain = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    let mut buffer = vec![0u8; 4 + plain.len() + 16];
    let len = noise
        .write_message(&plain, &mut buffer[4..])
        .map_err(|_| "Cannot encrypt peer message.".to_string())?;
    if len == 0 || len > MAX_FRAME {
        return Err("Network message exceeds the frame limit.".into());
    }
    buffer[..4].copy_from_slice(&(len as u32).to_be_bytes());
    buffer.truncate(4 + len);
    write_prefixed(stream, &buffer).await
}
fn tune_socket(stream: &TcpStream) -> Result<(), String> {
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(5))
        .with_interval(Duration::from_secs(1));
    let _ = socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive);
    Ok(())
}
fn decrypt(noise: &mut snow::TransportState, cipher: &[u8]) -> Result<Wire, String> {
    let mut plain = vec![0u8; cipher.len()];
    let len = noise
        .read_message(cipher, &mut plain)
        .map_err(|_| "Peer message authentication failed.".to_string())?;
    serde_json::from_slice(&plain[..len]).map_err(|_| "Invalid peer message.".to_string())
}
fn hello(core: &Core) -> Wire {
    let data = core.data.lock().unwrap();
    Wire::Hello {
        id: data.device_id.clone(),
        name: data.settings.device_name.clone(),
        platform: platform(),
        port: data.settings.port,
        protocol: crate::link::PROTOCOL,
    }
}

async fn handshake(
    core: &Arc<Core>,
    stream: &mut TcpStream,
    initiator: bool,
    key: Option<&[u8; 32]>,
    expected: Option<&[u8]>,
    pair_commit_started: &mut bool,
) -> Result<(snow::TransportState, Vec<u8>), String> {
    let identity = core
        .identity
        .as_ref()
        .ok_or("Secure identity is unavailable.")?;
    let protocol = if key.is_some() {
        NOISE_PAIR
    } else {
        NOISE_RECONNECT
    };
    let mut builder = snow::Builder::new(protocol.parse().map_err(|_| "Invalid secure protocol.")?)
        .prologue(b"TrapLink LAN v1")
        .map_err(|e| e.to_string())?
        .local_private_key(&identity.private)
        .map_err(|e| e.to_string())?;
    if let Some(key) = key {
        builder = builder.psk(3, key).map_err(|e| e.to_string())?;
    }
    let mut noise = if initiator {
        builder.build_initiator()
    } else {
        builder.build_responder()
    }
    .map_err(|e| e.to_string())?;
    let mut buffer = vec![0u8; MAX_FRAME];
    for step in 0..3 {
        if (step % 2 == 0) == initiator {
            if initiator && step == 2 {
                validate_remote_identity(core, &noise, expected, key.is_some())?;
                if key.is_some() {
                    *pair_commit_started = true;
                }
            }
            let len = noise
                .write_message(&[], &mut buffer)
                .map_err(|_| "Secure pairing authentication failed.".to_string())?;
            write_frame(stream, &buffer[..len]).await?;
        } else {
            let message = read_frame(stream).await?;
            noise.read_message(&message, &mut buffer).map_err(|_| {
                "Secure pairing authentication failed. Check the invitation code.".to_string()
            })?;
        }
    }
    let remote = noise
        .get_remote_static()
        .ok_or("Peer did not provide an identity.")?
        .to_vec();
    validate_remote_identity(core, &noise, expected, key.is_some())?;
    let transport = noise.into_transport_mode().map_err(|e| e.to_string())?;
    Ok((transport, remote))
}

fn validate_remote_identity(
    core: &Core,
    noise: &snow::HandshakeState,
    expected: Option<&[u8]>,
    paired: bool,
) -> Result<(), String> {
    let remote = noise
        .get_remote_static()
        .ok_or("Peer did not provide an identity.")?;
    if let Some(expected) = expected {
        if expected != remote {
            return Err(
                "Peer identity changed. Forget the device and pair again after verifying it."
                    .into(),
            );
        }
    }
    if core
        .identity
        .as_ref()
        .is_some_and(|identity| remote == identity.public)
    {
        return Err("Cannot pair this computer with itself.".into());
    }
    if !paired && expected.is_none() {
        let remote_b64 = STANDARD.encode(remote);
        if !core
            .data
            .lock()
            .map_err(|_| "Settings lock unavailable.")?
            .peers
            .iter()
            .any(|p| p.public_key == remote_b64)
        {
            return Err("Untrusted peer. Create a new pairing invitation first.".into());
        }
    }
    Ok(())
}

async fn exchange_hello(
    core: &Arc<Core>,
    stream: &mut TcpStream,
    noise: &mut snow::TransportState,
    remote_key: &[u8],
    ip: IpAddr,
    paired: bool,
) -> Result<(Peer, u32), String> {
    write_wire(stream, noise, &hello(core)).await?;
    let frame = read_frame(stream).await?;
    let Wire::Hello {
        id,
        name,
        platform,
        port,
        protocol,
    } = decrypt(noise, &frame)?
    else {
        return Err("Expected peer identity message.".into());
    };
    Uuid::parse_str(&id).map_err(|_| "Invalid remote device ID.")?;
    if name.is_empty()
        || name.chars().count() > 80
        || port < 1024
        || !["windows", "macos", "linux", "unknown"].contains(&platform.as_str())
    {
        return Err("Invalid remote device details.".into());
    }
    let public_key = STANDARD.encode(remote_key);
    let mut data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    if !paired
        && !data
            .peers
            .iter()
            .any(|p| p.public_key == public_key && p.peer.id == id)
    {
        return Err("Peer trust was revoked during connection.".into());
    }
    if id == data.device_id {
        return Err("Cannot connect a computer to itself.".into());
    }
    if let Some(existing) = data.peers.iter().find(|p| p.peer.id == id) {
        if existing.public_key != public_key {
            return Err("Peer ID is already associated with a different key.".into());
        }
    }
    if data
        .peers
        .iter()
        .any(|p| p.public_key == public_key && p.peer.id != id)
    {
        return Err("Peer key is already associated with another device ID.".into());
    }
    let permissions = data
        .peers
        .iter()
        .find(|p| p.peer.id == id)
        .map(|p| p.peer.permissions.clone())
        .unwrap_or_default();
    let peer = Peer {
        flow_clipboard: data
            .peers
            .iter()
            .find(|p| p.peer.id == id)
            .is_some_and(|p| p.peer.flow_clipboard),
        id: id.clone(),
        name,
        address: SocketAddr::new(ip, port).to_string(),
        platform,
        fingerprint: fingerprint(remote_key),
        connected: true,
        trusted: true,
        last_seen: Some(now()),
        permissions,
    };
    let mut updated = data.clone();
    updated.peers.retain(|p| p.peer.id != id);
    updated.peers.push(TrustedPeer {
        peer: peer.clone(),
        public_key,
    });
    crate::storage::save(&core.directory, &updated)?;
    *data = updated;
    Ok((peer, protocol))
}

pub async fn connect(
    core: Arc<Core>,
    address: String,
    code: Option<String>,
    id: Option<String>,
) -> Result<Peer, String> {
    let address = allowed_address(&address)?;
    let key: Option<[u8; 32]> = code
        .map(|code| {
            URL_SAFE_NO_PAD
                .decode(code.trim())
                .map_err(|_| "Invalid invitation code.")
                .and_then(|v| {
                    v.try_into()
                        .map_err(|_| "Invitation code must contain 32 random bytes.")
                })
        })
        .transpose()?;
    let expected = if let Some(id) = id {
        Some(
            STANDARD
                .decode(
                    core.data
                        .lock()
                        .map_err(|_| "Settings lock unavailable.")?
                        .peers
                        .iter()
                        .find(|p| p.peer.id == id)
                        .ok_or("Unknown peer.")?
                        .public_key
                        .clone(),
                )
                .map_err(|_| "Invalid stored peer key.")?,
        )
    } else {
        None
    };
    if key.is_none() && expected.is_none() {
        return Err("A pairing code or a trusted peer is required.".into());
    }
    if let (None, Some(expected)) = (key.as_ref(), expected.as_deref()) {
        return reconnect_trusted(&core, address, expected).await;
    }
    let mut pair_commit_started = false;
    tokio::time::timeout(
        CONNECTION_TIMEOUT,
        connect_address(
            &core,
            address,
            key.as_ref(),
            expected.as_deref(),
            &mut pair_commit_started,
        ),
    )
    .await
    .map_err(|_| connection_timeout())?
}

async fn reconnect_trusted(
    core: &Arc<Core>,
    address: SocketAddr,
    expected: &[u8],
) -> Result<Peer, String> {
    let deadline = tokio::time::Instant::now() + CONNECTION_TIMEOUT;
    let mut pair_commit_started = false;
    let mut last_error = match tokio::time::timeout(
        DIRECT_ATTEMPT,
        connect_address(
            core,
            address,
            None,
            Some(expected),
            &mut pair_commit_started,
        ),
    )
    .await
    {
        Ok(Ok(peer)) => return Ok(peer),
        Ok(Err(error)) => error,
        Err(_) => connection_timeout(),
    };
    if address.ip().is_loopback() {
        return Err(last_error);
    }
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let candidates = tokio::time::timeout(remaining, discover(address))
        .await
        .unwrap_or_default();
    if candidates.is_empty() {
        return Err(format!(
            "Couldn't find the other computer at {address} or anywhere on this network. Make sure it has TrapLink open and is on the same network as this computer."
        ));
    }
    for candidate in candidates {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut pair_commit_started = false;
        match tokio::time::timeout(
            remaining.min(DIRECT_ATTEMPT),
            connect_address(
                core,
                candidate,
                None,
                Some(expected),
                &mut pair_commit_started,
            ),
        )
        .await
        {
            Ok(Ok(peer)) => return Ok(peer),
            Ok(Err(error)) => last_error = error,
            Err(_) => last_error = connection_timeout(),
        }
    }
    Err(last_error)
}

fn local_ipv4() -> Vec<Ipv4Addr> {
    let mut addresses = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(_, ip)| match ip {
            IpAddr::V4(ip) if ip.is_private() => Some(ip),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    addresses.sort_by_key(|ip| {
        let octets = ip.octets();
        let priority = match octets[0] {
            192 => 0,
            172 => 1,
            _ => 2,
        };
        (priority, octets)
    });
    addresses
}

fn scan_targets(own: &[Ipv4Addr], stale: SocketAddr) -> Vec<SocketAddr> {
    let mut subnets: Vec<[u8; 3]> = Vec::new();
    for ip in own {
        let [a, b, c, _] = ip.octets();
        if !subnets.contains(&[a, b, c]) && subnets.len() < MAX_SCAN_SUBNETS {
            subnets.push([a, b, c]);
        }
    }
    subnets
        .iter()
        .flat_map(|&[a, b, c]| (1..=254u8).map(move |d| Ipv4Addr::new(a, b, c, d)))
        .filter(|ip| !own.contains(ip))
        .map(|ip| SocketAddr::new(IpAddr::V4(ip), stale.port()))
        .filter(|candidate| *candidate != stale)
        .collect()
}

async fn discover(stale: SocketAddr) -> Vec<SocketAddr> {
    let targets = scan_targets(&local_ipv4(), stale);
    let limit = Arc::new(Semaphore::new(256));
    let mut probes = tokio::task::JoinSet::new();
    for target in targets {
        let limit = limit.clone();
        probes.spawn(async move {
            let _permit = limit.acquire_owned().await.ok()?;
            let stream = tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(target))
                .await
                .ok()?
                .ok()?;
            drop(stream);
            Some(target)
        });
    }
    let mut found = Vec::new();
    while let Some(result) = probes.join_next().await {
        if let Ok(Some(target)) = result {
            found.push(target);
        }
    }
    found.sort();
    found
}

fn connection_timeout() -> String {
    "Connection timed out. Confirm both computers are on the same network and the firewall allows TrapLink.".into()
}

async fn connect_address(
    core: &Arc<Core>,
    address: SocketAddr,
    key: Option<&[u8; 32]>,
    expected: Option<&[u8]>,
    pair_commit_started: &mut bool,
) -> Result<Peer, String> {
    let mut stream = TcpStream::connect(address)
        .await
        .map_err(|e| format!("Cannot reach peer: {e}"))?;
    tune_socket(&stream)?;
    stream
        .write_u8(if key.is_some() { 1 } else { 2 })
        .await
        .map_err(|e| e.to_string())?;
    let (mut noise, remote) =
        handshake(core, &mut stream, true, key, expected, pair_commit_started).await?;
    let (peer, protocol) = exchange_hello(
        core,
        &mut stream,
        &mut noise,
        &remote,
        address.ip(),
        key.is_some(),
    )
    .await?;
    install_session(core.clone(), stream, noise, peer.clone(), protocol)?;
    Ok(peer)
}

pub async fn connect_invitation(core: Arc<Core>, invitation: &str) -> Result<Peer, String> {
    let invitation = decode_invitation(invitation, chrono::Utc::now().timestamp())?;
    if core
        .identity
        .as_ref()
        .is_some_and(|identity| identity.public == invitation.public_key)
    {
        return Err("Cannot pair this computer with itself.".into());
    }
    let deadline = tokio::time::Instant::now() + CONNECTION_TIMEOUT;
    let mut last_error = connection_timeout();
    for (index, address) in invitation.candidates.iter().enumerate() {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(connection_timeout());
        }
        let attempts_left = invitation.candidates.len() - index;
        let budget = if attempts_left == 1 {
            remaining
        } else {
            (remaining / attempts_left as u32).min(Duration::from_secs(4))
        };
        let mut pair_commit_started = false;
        let result = tokio::time::timeout(
            budget,
            connect_address(
                &core,
                *address,
                Some(&invitation.key),
                Some(&invitation.public_key),
                &mut pair_commit_started,
            ),
        )
        .await;
        match result {
            Ok(Ok(peer)) => return Ok(peer),
            Ok(Err(error)) => last_error = error,
            Err(_) => last_error = connection_timeout(),
        }
        if pair_commit_started {
            return Err(last_error);
        }
    }
    Err(last_error)
}

async fn accept(core: Arc<Core>, mut stream: TcpStream, address: SocketAddr) -> Result<(), String> {
    tune_socket(&stream)?;
    let mode = stream
        .read_u8()
        .await
        .map_err(|_| "Peer disconnected.".to_string())?;
    if mode != 1 && mode != 2 {
        return Err("Unsupported LAN protocol.".into());
    }
    let key = if mode == 1 {
        let mut guard = core
            .invite
            .lock()
            .map_err(|_| "Pairing lock unavailable.")?;
        let invite = guard
            .as_mut()
            .filter(|invite| invite.expires > Instant::now() && !invite.reserved)
            .ok_or("There is no available pairing invitation.")?;
        invite.reserved = true;
        Some(invite.key)
    } else {
        None
    };
    let _reservation = InviteReservation {
        core: core.clone(),
        key,
    };
    let result = async {
        let mut pair_commit_started = false;
        let (mut noise, remote) = handshake(
            &core,
            &mut stream,
            false,
            key.as_ref(),
            None,
            &mut pair_commit_started,
        )
        .await?;
        if let Some(key) = key {
            let mut guard = core
                .invite
                .lock()
                .map_err(|_| "Pairing lock unavailable.")?;
            if !guard
                .as_ref()
                .is_some_and(|i| i.key == key && i.expires > Instant::now())
            {
                return Err("Pairing invitation was cancelled or expired.".into());
            }
            *guard = None;
        }
        let (peer, protocol) = exchange_hello(
            &core,
            &mut stream,
            &mut noise,
            &remote,
            address.ip(),
            key.is_some(),
        )
        .await?;
        install_session(core.clone(), stream, noise, peer, protocol)
    }
    .await;
    if result.is_err() {
        if let Some(key) = key {
            if let Ok(mut guard) = core.invite.lock() {
                if let Some(invite) = guard.as_mut().filter(|i| i.key == key) {
                    invite.reserved = false;
                }
            }
        }
    }
    result
}

pub async fn listen(core: Arc<Core>) {
    if core.identity.is_none() {
        return;
    }
    let permits = Arc::new(Semaphore::new(8));
    loop {
        let port = core.data.lock().map(|d| d.settings.port).unwrap_or(48372);
        let listener = match TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => listener,
            Err(e) => {
                core.warning(format!("Cannot listen on LAN port {port}: {e}"));
                core.listening.store(false, Ordering::Relaxed);
                tokio::select! {
                    _ = core.network_changed.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                }
                continue;
            }
        };
        core.clear_warnings(|w| {
            w.starts_with("Cannot listen on LAN port") || w.starts_with("LAN listener failed")
        });
        core.listening.store(true, Ordering::Relaxed);
        loop {
            tokio::select! {
                _ = core.network_changed.notified() => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, address)) => {
                        if let Ok(permit) = permits.clone().try_acquire_owned() {
                            let core = core.clone();
                            tokio::spawn(async move { let _permit = permit; let _ = tokio::time::timeout(Duration::from_secs(12), accept(core, stream, address)).await; });
                        }
                    },
                    Err(e) => { core.warning(format!("LAN listener failed: {e}")); break; }
                }
            }
        }
        core.listening.store(false, Ordering::Relaxed);
    }
}

fn install_session(
    core: Arc<Core>,
    stream: TcpStream,
    noise: snow::TransportState,
    peer: Peer,
    protocol: u32,
) -> Result<(), String> {
    let data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    if !data
        .peers
        .iter()
        .any(|p| p.peer.id == peer.id && p.peer.fingerprint == peer.fingerprint)
    {
        return Err("Peer trust was revoked before connection completed.".into());
    }
    let (tx, rx) = mpsc::channel(32);
    let (cancel, cancel_rx) = watch::channel(false);
    let generation = Uuid::new_v4();
    let session = Arc::new(Session {
        protocol,
        ..Session::new(tx, cancel, generation)
    });
    if let Some(old) = core
        .sessions
        .lock()
        .map_err(|_| "Network lock unavailable.")?
        .insert(peer.id.clone(), session.clone())
    {
        crate::input::stop_peer(&core, &peer.id);
        let _ = old.cancel.send(true);
    }
    drop(data);
    tokio::spawn(run_session(
        core, stream, noise, peer, rx, cancel_rx, session,
    ));
    Ok(())
}

pub fn disconnect(core: &Core, id: &str) -> Result<(), String> {
    crate::flow::user_disconnected(core, id);
    crate::input::stop_peer(core, id);
    let mut sessions = core
        .sessions
        .lock()
        .map_err(|_| "Network lock unavailable.")?;
    if let Some(session) = sessions.remove(id) {
        session.farewell.store(true, Ordering::SeqCst);
        let _ = session.cancel.send(true);
    }
    Ok(())
}
pub fn drop_session(core: &Core, id: &str) {
    crate::input::stop_peer(core, id);
    if let Ok(mut sessions) = core.sessions.lock() {
        if let Some(session) = sessions.remove(id) {
            let _ = session.cancel.send(true);
        }
    }
}
pub fn rtt(core: &Core, id: &str) -> Option<u32> {
    session(core, id)
        .ok()
        .map(|s| s.rtt_ms.load(Ordering::Relaxed))
        .filter(|ms| *ms > 0)
}
pub fn peer_address(core: &Core, id: &str) -> Option<String> {
    let data = core.data.lock().ok()?;
    data.peers
        .iter()
        .find(|p| p.peer.id == id)
        .map(|p| p.peer.address.clone())
        .filter(|a| !a.is_empty())
}
fn permit(core: &Core, id: &str, files: bool) -> Result<(), String> {
    let data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    let peer = data
        .peers
        .iter()
        .find(|p| p.peer.id == id)
        .ok_or("Peer is not trusted.")?;
    if files {
        if !data.settings.file_sharing_enabled || !peer.peer.permissions.files {
            return Err(
                "File sharing requires the global switch and this peer's file permission.".into(),
            );
        }
    } else if !data.settings.clipboard_enabled || !peer.peer.permissions.clipboard {
        return Err(
            "Clipboard sharing requires the global switch and this peer's clipboard permission."
                .into(),
        );
    }
    Ok(())
}
struct IncomingFile {
    clipboard: Option<ClipboardFormat>,
    file: Option<tokio::fs::File>,
    temporary: PathBuf,
    destination: PathBuf,
    received: u64,
    expected: u64,
    last_activity: Instant,
}
impl Drop for IncomingFile {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.temporary);
    }
}

struct OutgoingTransferGuard {
    core: Arc<Core>,
    session: Arc<Session>,
    id: String,
    complete: bool,
}
impl Drop for OutgoingTransferGuard {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        self.core.transfer_status(&self.id, "failed");
        let _ = self.session.tx.try_send(Outgoing {
            message: Wire::FileAbort {
                request: self.id.clone(),
            },
            result: None,
        });
    }
}

fn expire_incoming(core: &Core, files: &mut HashMap<String, IncomingFile>, now: Instant) {
    let expired: Vec<String> = files
        .iter()
        .filter(|(_, file)| {
            now.saturating_duration_since(file.last_activity) >= TRANSFER_IDLE_TIMEOUT
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in expired {
        files.remove(&id);
        core.transfer_status(&id, "failed");
    }
}

pub fn safe_file_name(value: &str) -> Result<String, String> {
    let stem = value
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|s| {
            matches!(
                s,
                "1" | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "\u{b9}"
                    | "\u{b2}"
                    | "\u{b3}"
            )
        })
    });
    if reserved
        || value.is_empty()
        || value.chars().count() > 180
        || value == "."
        || value == ".."
        || value
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        || value.ends_with(['.', ' '])
    {
        return Err("Unsafe file name rejected.".into());
    }
    Ok(value.to_owned())
}
async fn incoming(
    core: &Arc<Core>,
    peer: &Peer,
    wire: Wire,
    files: &mut HashMap<String, IncomingFile>,
) -> Option<Wire> {
    match wire {
        Wire::FileAbort { request } => {
            if files.remove(&request).is_some() {
                core.transfer_status(&request, "failed");
            }
            None
        }
        Wire::ControlStart { request } => Some(Wire::Ack {
            request,
            error: crate::input::start_receiving(core, &peer.id).err(),
        }),
        Wire::ControlStop => {
            // Modern Flow stops carry a visit ID. An untagged stop on that
            // connection belongs to focused control. Older peers share it.
            if session(core, &peer.id).is_ok_and(|link| link.protocol >= crate::link::SCOPED_FLOW) {
                crate::input::stop_focused(core, &peer.id);
            } else {
                crate::input::stop_peer(core, &peer.id);
            }
            None
        }
        Wire::EmergencyStop => {
            crate::input::stop_peer(core, &peer.id);
            None
        }
        Wire::Input { event } => incoming_input(core, peer, None, FlowPacket::Input { event }),
        Wire::Flow { session, packet } => incoming_input(core, peer, Some(session), packet),
        Wire::FlowStart {
            request,
            session,
            edge,
            entry,
            clipboard,
        } => Some(Wire::Ack {
            request,
            error: crate::input::start_flow_receiving(
                core, &peer.id, session, edge, entry, clipboard,
            )
            .err(),
        }),
        Wire::FlowReturn { entry } => {
            incoming_input(core, peer, None, FlowPacket::Return { entry })
        }
        Wire::InputConsent { request, flow } => consent_requested(core, peer, request, flow),
        Wire::InputKeepalive => incoming_input(core, peer, None, FlowPacket::Keepalive),
        Wire::Clipboard { request, text } => {
            let result = permit(core, &peer.id, false).and_then(|_| {
                if text.len() <= MAX_CLIPBOARD {
                    Ok(())
                } else {
                    Err("Clipboard text exceeds 24 KiB.".into())
                }
            });
            let result = if result.is_ok() {
                core.clipboard
                    .write(crate::clipboard::Content::Text(text))
                    .await
            } else {
                result
            };
            Some(Wire::Ack {
                request,
                error: result.err(),
            })
        }
        Wire::FileStart {
            clipboard,
            request,
            name,
            bytes,
        } => {
            let result: Result<(), String> = async {
                permit_transfer(core, &peer.id, clipboard.as_ref())?;
                if let Some(format) = &clipboard {
                    format.validate(bytes)?;
                }
                Uuid::parse_str(&request).map_err(|_| "Invalid transfer ID.")?;
                let name = safe_file_name(&name)?;
                if bytes > MAX_FILE || files.len() >= 2 || files.contains_key(&request) {
                    return Err("File exceeds 128 MiB or too many transfers are active.".into());
                }
                let directory = PathBuf::from(
                    core.data
                        .lock()
                        .map_err(|_| "Settings lock unavailable.")?
                        .settings
                        .download_directory
                        .clone(),
                )
                .join("TrapLink");
                tokio::fs::create_dir_all(&directory)
                    .await
                    .map_err(|e| format!("Cannot create receive folder: {e}"))?;
                let temporary = directory.join(format!(".{}.part", Uuid::new_v4()));
                let destination = directory.join(format!("{}-{name}", Uuid::new_v4()));
                let file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)
                    .await
                    .map_err(|e| format!("Cannot create received file: {e}"))?;
                files.insert(
                    request.clone(),
                    IncomingFile {
                        clipboard,
                        file: Some(file),
                        temporary,
                        destination,
                        received: 0,
                        expected: bytes,
                        last_activity: Instant::now(),
                    },
                );
                core.push_transfer(Transfer {
                    id: request.clone(),
                    file_name: name,
                    peer_name: peer.name.clone(),
                    direction: "incoming".into(),
                    bytes,
                    status: "receiving".into(),
                    created_at: now(),
                });
                Ok(())
            }
            .await;
            Some(Wire::Ack {
                request,
                error: result.err(),
            })
        }
        Wire::FileChunk { request, data } => {
            let result: Result<(), String> = async {
                let file = files.get_mut(&request).ok_or("Unknown transfer.")?;
                permit_transfer(core, &peer.id, file.clipboard.as_ref())?;
                let bytes = STANDARD.decode(data).map_err(|_| "Invalid file block.")?;
                if bytes.len() > CHUNK || file.received + bytes.len() as u64 > file.expected {
                    return Err("File exceeds the declared size.".into());
                }
                file.file
                    .as_mut()
                    .ok_or("Received file is closed.")?
                    .write_all(&bytes)
                    .await
                    .map_err(|e| format!("Cannot write received file: {e}"))?;
                file.received += bytes.len() as u64;
                file.last_activity = Instant::now();
                Ok(())
            }
            .await;
            if let Err(error) = result {
                files.remove(&request);
                core.transfer_status(&request, "failed");
                return Some(Wire::Ack {
                    request,
                    error: Some(error),
                });
            }
            None
        }
        Wire::FileEnd { request } => {
            let staged = files.remove(&request);
            let result: Result<(), String> = async {
                let mut file = staged.ok_or("Unknown transfer.")?;
                permit_transfer(core, &peer.id, file.clipboard.as_ref())?;
                if file.expected != file.received {
                    return Err("Received file length did not match the declared size.".into());
                }
                let mut handle = file.file.take().ok_or("Received file is closed.")?;
                handle.flush().await.map_err(|e| e.to_string())?;
                handle.sync_all().await.map_err(|e| e.to_string())?;
                drop(handle);
                if let Some(format) = &file.clipboard {
                    return finish_clipboard(core, &peer.id, &file.temporary, format).await;
                }
                tokio::fs::hard_link(&file.temporary, &file.destination)
                    .await
                    .map_err(|e| format!("Cannot finalize received file: {e}"))?;
                let _ = tokio::fs::remove_file(&file.temporary).await;
                Ok(())
            }
            .await;
            core.transfer_status(&request, if result.is_ok() { "complete" } else { "failed" });
            Some(Wire::Ack {
                request,
                error: result.err(),
            })
        }
        Wire::Ping => None,
        _ => None,
    }
}

fn incoming_input(
    core: &Arc<Core>,
    peer: &Peer,
    session: Option<Uuid>,
    packet: FlowPacket,
) -> Option<Wire> {
    let result = match packet {
        FlowPacket::Stop => {
            if let Some(token) = session {
                if let Ok(mut state) = core.input.lock() {
                    state.acknowledge_stop(&peer.id, token);
                }
            }
            return None;
        }
        FlowPacket::Return { entry } => {
            match crate::flow::returned(core, &peer.id, session, entry) {
                Ok(true) => return Some(FlowPacket::Stop.into_wire(session)),
                Ok(false) => {}
                Err(error) => core.warning(error),
            }
            return None;
        }
        FlowPacket::Input { event } => crate::input::receive(core, &peer.id, session, event),
        FlowPacket::Keepalive => crate::input::keepalive(core, &peer.id, session).map(|_| None),
    };
    match result {
        Ok(Some(info)) => {
            if info.clipboard && clipboard_permitted(core, &peer.id) {
                share_clipboard_on_switch(core.clone(), peer.id.clone());
            }
            Some(FlowPacket::Return { entry: info.entry }.into_wire(session))
        }
        Ok(None) => None,
        Err(error) => {
            crate::input::stop_receiving(core, &peer.id, session);
            if error != crate::input::INACTIVE {
                core.warning(error);
            }
            Some(FlowPacket::Stop.into_wire(session))
        }
    }
}

fn consent_requested(core: &Core, peer: &Peer, request: String, flow: bool) -> Option<Wire> {
    let permitted = crate::input::enabled(core, &peer.id).is_ok();
    let name = core
        .data
        .lock()
        .ok()
        .and_then(|data| {
            data.peers
                .iter()
                .find(|p| p.peer.id == peer.id)
                .map(|p| p.peer.name.clone())
        })
        .unwrap_or_else(|| peer.name.clone());
    let asked = crate::consent::ConsentRequest {
        peer_id: peer.id.clone(),
        peer_name: name,
        request: request.clone(),
        flow,
    };
    let decision = match core.consent.lock() {
        Ok(mut state) => state.receive(asked, permitted, Instant::now()),
        Err(_) => crate::consent::Decision::Refuse("Consent lock unavailable."),
    };
    match decision {
        crate::consent::Decision::Granted => Some(Wire::Ack {
            request,
            error: None,
        }),
        crate::consent::Decision::Refuse(error) => Some(Wire::Ack {
            request,
            error: Some(error.into()),
        }),
        crate::consent::Decision::Prompt(open) => {
            crate::consent::notify(&open);
            None
        }
    }
}
pub fn answer_consent(core: &Core, peer: &str, request: String, error: Option<String>) {
    try_send(core, peer, Wire::Ack { request, error });
}
fn is_current_session(core: &Core, id: &str, generation: Uuid) -> bool {
    core.sessions
        .lock()
        .is_ok_and(|sessions| sessions.get(id).is_some_and(|s| s.generation == generation))
}

async fn run_session(
    core: Arc<Core>,
    stream: TcpStream,
    mut noise: snow::TransportState,
    peer: Peer,
    mut outgoing: mpsc::Receiver<Outgoing>,
    mut cancel: watch::Receiver<bool>,
    session: Arc<Session>,
) {
    let (generation, protocol) = (session.generation, session.protocol);
    let (rtt, farewell) = (session.rtt_ms.clone(), session.farewell.clone());
    // Let closing the last owning session also close its outgoing channel.
    drop(session);
    let probing = protocol >= crate::link::PROBING;
    let (mut reader, mut writer) = stream.into_split();
    let (frames_tx, mut frames) = mpsc::channel(8);
    let reader_task = tokio::spawn(async move {
        loop {
            let frame = read_frame(&mut reader).await;
            let finished = frame.is_err();
            if frames_tx.send(frame).await.is_err() || finished {
                break;
            }
        }
    });
    let mut pending: HashMap<String, oneshot::Sender<Result<(), String>>> = HashMap::new();
    let mut files: HashMap<String, IncomingFile> = HashMap::new();
    let mut clipboard_commit = tokio::task::JoinSet::new();
    let mut heartbeat = tokio::time::interval(if probing {
        crate::link::PROBE_INTERVAL
    } else {
        Duration::from_secs(15)
    });
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut transfer_cleanup = tokio::time::interval(Duration::from_secs(5));
    let mut last_message = Instant::now();
    let mut probe: Option<(u64, Instant)> = None;
    let mut probe_id = 0u64;
    loop {
        if !is_current_session(&core, &peer.id, generation) {
            break;
        }
        pending.retain(|_, result| !result.is_closed());
        tokio::select! {
            _ = cancel.changed() => break,
            _ = transfer_cleanup.tick() => expire_incoming(&core, &mut files, Instant::now()),
            _ = heartbeat.tick() => {
                let limit = crate::link::liveness_timeout(protocol, crate::input::active_with(&core, &peer.id));
                if last_message.elapsed() > limit { break; }
                let message = if probing {
                    probe_id = probe_id.wrapping_add(1);
                    probe = Some((probe_id, Instant::now()));
                    Wire::Probe { id: probe_id }
                } else {
                    Wire::Ping
                };
                if write_wire(&mut writer, &mut noise, &message).await.is_err() { break; }
            },
            Some(reply) = clipboard_commit.join_next(), if !clipboard_commit.is_empty() => {
                match reply {
                    Ok(Some(ack)) => if write_wire(&mut writer, &mut noise, &ack).await.is_err() { break; },
                    _ => break,
                }
            }
            next = frames.recv() => {
                if !is_current_session(&core, &peer.id, generation) { break; }
                let Some(Ok(frame)) = next else { break; };
                let Ok(wire) = decrypt(&mut noise, &frame) else { break; }; last_message = Instant::now();
                match wire {
                    Wire::Ack { request, error } => { if let Some(sender) = pending.remove(&request) { let _ = sender.send(error.map_or(Ok(()), Err)); } },
                    Wire::Probe { id } => { if write_wire(&mut writer, &mut noise, &Wire::ProbeReply { id }).await.is_err() { break; } },
                    Wire::ProbeReply { id } => {
                        if let Some((_, sent_at)) = probe.filter(|(sent, _)| *sent == id) {
                            probe = None;
                            let previous = Some(rtt.load(Ordering::Relaxed)).filter(|ms| *ms > 0);
                            rtt.store(crate::link::smooth_rtt(previous, sent_at.elapsed()), Ordering::Relaxed);
                        }
                    },
                    Wire::Goodbye => {
                        crate::flow::remote_closed(&core, &peer.id);
                        break;
                    },
                    Wire::FileEnd { request } if files.get(&request).is_some_and(|file| file.clipboard.is_some()) => {
                        // Encoding a large native bitmap or unpacking a file list can take
                        // longer than the input lease. Keep serving motion and probes.
                        if let Some(file) = files.remove(&request) {
                            if clipboard_commit.is_empty() {
                                let (core, peer) = (core.clone(), peer.clone());
                                clipboard_commit.spawn(async move {
                                    let mut staged = HashMap::from([(request.clone(), file)]);
                                    incoming(&core, &peer, Wire::FileEnd { request }, &mut staged).await
                                });
                            } else {
                                core.transfer_status(&request, "failed");
                                let reply = Wire::Ack { request, error: Some("Another clipboard transfer is finishing.".into()) };
                                if write_wire(&mut writer, &mut noise, &reply).await.is_err() { break; }
                            }
                        }
                    }
                    wire => { if let Some(reply) = incoming(&core, &peer, wire, &mut files).await { if write_wire(&mut writer, &mut noise, &reply).await.is_err() { break; } } }
                }
            },
            next = outgoing.recv() => {
                if !is_current_session(&core, &peer.id, generation) { break; }
                let Some(message) = next else { break; };
                if let Some(result) = message.result { match &message.message {
                    Wire::Clipboard { request, .. } | Wire::FileStart { request, .. } | Wire::FileEnd { request } | Wire::ControlStart { request } | Wire::FlowStart { request, .. } | Wire::InputConsent { request, .. } => { if pending.len() >= 16 { let _ = result.send(Err("Too many requests pending.".into())); continue; } pending.insert(request.clone(), result); },
                    _ => { let _ = result.send(Ok(())); }
                } }
                if write_wire(&mut writer, &mut noise, &message.message).await.is_err() { break; }
            }
        }
    }
    if farewell.load(Ordering::SeqCst) && probing {
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            write_wire(&mut writer, &mut noise, &Wire::Goodbye),
        )
        .await;
    }
    reader_task.abort();
    for id in files.keys() {
        core.transfer_status(id, "failed");
    }
    for (_, result) in pending {
        let _ = result.send(Err("Peer disconnected.".into()));
    }
    if let Ok(mut sessions) = core.sessions.lock() {
        if sessions
            .get(&peer.id)
            .is_some_and(|s| s.generation == generation)
        {
            crate::input::stop_peer(&core, &peer.id);
            sessions.remove(&peer.id);
            if let Ok(mut consent) = core.consent.lock() {
                consent.forget(&peer.id);
            }
        }
    }
}
pub(super) fn session(core: &Core, id: &str) -> Result<Arc<Session>, String> {
    core.sessions
        .lock()
        .map_err(|_| "Network lock unavailable.")?
        .get(id)
        .cloned()
        .ok_or("Peer is not connected.".into())
}
async fn request(session: &Session, message: Wire) -> Result<(), String> {
    request_within(session, message, Duration::from_secs(30)).await
}
async fn request_within(session: &Session, message: Wire, limit: Duration) -> Result<(), String> {
    let (result, rx) = oneshot::channel();
    let operation = async {
        session
            .tx
            .send(Outgoing {
                message,
                result: Some(result),
            })
            .await
            .map_err(|_| "Peer disconnected.".to_string())?;
        rx.await.map_err(|_| "Peer disconnected.".to_string())?
    };
    tokio::time::timeout(limit, operation)
        .await
        .map_err(|_| "Peer did not acknowledge the request.".to_string())?
}
pub(crate) fn share_clipboard_on_switch(core: Arc<Core>, id: String) {
    tokio::spawn(async move {
        if let Err(error) = send_clipboard(core.clone(), &id).await {
            if error != "Clipboard has no supported content." {
                core.warning(
                    "Flow could not transfer the clipboard. Use Send clipboard to see the reason."
                        .into(),
                );
            }
        }
    });
}

pub async fn send_clipboard(core: Arc<Core>, id: &str) -> Result<(), String> {
    permit(&core, id, false)?;
    let session = session(&core, id)?;
    use crate::clipboard::Content;
    let content = core
        .clipboard
        .read()
        .await?
        .ok_or("Clipboard has no supported content.")?;
    let text = match content {
        Content::Text(text) => text,
        content => {
            if session.protocol < crate::link::RICH_CLIPBOARD {
                return Err(
                    "Update TrapLink on both computers to share copied files and images.".into(),
                );
            }
            let (format, reader, bytes): (_, Box<dyn AsyncRead + Unpin + Send>, u64) = match content
            {
                Content::Files(paths) => {
                    permit(&core, id, true)?;
                    if paths.len() > 32 {
                        return Err("Copy up to 32 regular files at a time.".into());
                    }
                    let mut entries = Vec::new();
                    let mut total = 0u64;
                    let mut reader: Box<dyn AsyncRead + Unpin + Send> =
                        Box::new(tokio::io::empty());
                    for path in paths {
                        let name = safe_file_name(
                            path.file_name()
                                .and_then(|n| n.to_str())
                                .ok_or("Invalid file name.")?,
                        )?;
                        let file = tokio::fs::File::open(&path)
                            .await
                            .map_err(|_| "Cannot open copied file.")?;
                        let meta = file
                            .metadata()
                            .await
                            .map_err(|_| "Cannot inspect copied file.")?;
                        if !meta.is_file() || meta.len() > MAX_FILE - total {
                            return Err("Copy regular files totaling at most 128 MiB.".into());
                        }
                        total += meta.len();
                        entries.push((name, meta.len()));
                        reader = Box::new(reader.chain(file.take(meta.len())));
                    }
                    (ClipboardFormat::Files(entries), reader, total)
                }
                Content::Image {
                    width,
                    height,
                    rgba,
                } => {
                    let bytes = rgba.len() as u64;
                    (
                        ClipboardFormat::Image { width, height },
                        Box::new(std::io::Cursor::new(rgba)),
                        bytes,
                    )
                }
                Content::Text(_) => unreachable!(),
            };
            format.validate(bytes)?;
            return send_stream(
                core,
                id,
                session,
                reader,
                "Clipboard".into(),
                bytes,
                Some(format),
            )
            .await
            .map(|_| ());
        }
    };
    if text.len() > MAX_CLIPBOARD {
        return Err("Clipboard text exceeds 24 KiB.".into());
    }
    let wire = Wire::Clipboard {
        request: Uuid::new_v4().to_string(),
        text,
    };
    if serde_json::to_vec(&wire)
        .map_err(|_| "Invalid clipboard text.")?
        .len()
        + 16
        > MAX_FRAME
    {
        return Err("Encoded clipboard text exceeds the message limit.".into());
    }
    permit(&core, id, false)?;
    request(&session, wire).await
}
pub async fn set_control(core: Arc<Core>, id: &str, enabled: bool) -> Result<(), String> {
    let session = session(&core, id)?;
    if !enabled {
        let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
        if state.sending_to.as_deref() == Some(id) && state.flow.switch.peer().is_none() {
            state.sending_to = None;
            state.pending_control = None;
            // Stop locally even if the peer no longer drains its queue. In
            // that case socket teardown releases the peer's receiver lease.
            session.stop_untagged();
        }
        return Ok(());
    }
    crate::input::enabled(&core, id)?;
    let token = Uuid::new_v4();
    let (answer, response) = oneshot::channel();
    {
        // A legacy peer treats every stop as untagged. Queue its new focused
        // start only after the previous Flow transition has sent its last stop.
        let _transition = if session.protocol < crate::link::SCOPED_FLOW {
            Some(crate::flow::transition(&core).await?)
        } else {
            None
        };
        let slot = tokio::time::timeout(Duration::from_secs(3), session.tx.reserve())
            .await
            .map_err(|_| "Peer is not responding.")?
            .map_err(|_| "Peer disconnected.")?;
        let mut state = crate::input::reserve_input(&core, id)?;
        if state.busy() {
            return Err("Stop the existing input session first.".into());
        }
        state.sending_to = Some(id.into());
        state.pending_control = Some(token);
        slot.send(Outgoing {
            message: Wire::ControlStart {
                request: token.to_string(),
            },
            result: Some(answer),
        });
    }
    let result = tokio::time::timeout(Duration::from_secs(30), response).await;
    let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
    if state.pending_control != Some(token) {
        return Err("Focused control ended before the peer confirmed.".into());
    }
    state.pending_control = None;
    // A timed-out request may already have started a receiver. Queue the
    // matching stop before making room for another local session.
    if result.is_err() {
        session.stop_untagged();
    }
    let result = result
        .map_err(|_| "Peer did not acknowledge the request.".to_string())
        .and_then(|answer| answer.map_err(|_| "Peer disconnected.".to_string()))
        .and_then(|answer| answer);
    if result.is_err() {
        state.sending_to = None;
    }
    result
}
pub async fn request_input_consent(core: Arc<Core>, id: &str, flow: bool) -> Result<(), String> {
    let session = session(&core, id)?;
    if session.protocol < crate::consent::PROTOCOL {
        return Err(crate::consent::OUTDATED.into());
    }
    core.consent
        .lock()
        .map_err(|_| "Consent lock unavailable.")?
        .begin_outgoing(id, Instant::now())?;
    let result = request_within(
        &session,
        Wire::InputConsent {
            request: Uuid::new_v4().to_string(),
            flow,
        },
        crate::consent::REQUEST_TIMEOUT,
    )
    .await
    .map_err(|error| {
        if error == "Peer did not acknowledge the request." {
            crate::consent::EXPIRED.to_string()
        } else {
            error
        }
    });
    if let Ok(mut consent) = core.consent.lock() {
        consent.end_outgoing(id);
    }
    result
}
#[cfg(all(test, windows))]
async fn flow_start(
    core: &Core,
    id: &str,
    token: Uuid,
    edge: FlowEdge,
    entry: f64,
    clipboard: bool,
) -> Result<(), String> {
    let session = session(core, id)?;
    flow_start_on(&session, token, edge, entry, clipboard).await
}
pub(super) async fn flow_start_on(
    session: &Session,
    token: Uuid,
    edge: FlowEdge,
    entry: f64,
    clipboard: bool,
) -> Result<(), String> {
    crate::flow::validate_entry(entry)?;
    request(
        session,
        Wire::FlowStart {
            request: token.to_string(),
            session: (session.protocol >= crate::link::SCOPED_FLOW).then_some(token),
            edge,
            entry,
            clipboard,
        },
    )
    .await
}
#[cfg(all(test, windows))]
async fn send_flow(core: &Core, id: &str, token: Uuid, packet: FlowPacket) -> Result<(), String> {
    let link = session(core, id)?;
    send_flow_on(core, id, &link, token, packet).await
}
pub(super) async fn send_flow_on(
    core: &Core,
    id: &str,
    session: &Session,
    token: Uuid,
    packet: FlowPacket,
) -> Result<(), String> {
    let scoped = (session.protocol >= crate::link::SCOPED_FLOW).then_some(token);
    let slot = tokio::time::timeout(Duration::from_secs(2), session.tx.reserve())
        .await
        .map_err(|_| "Peer stopped receiving input.".to_string())?
        .map_err(|_| "Peer disconnected.")?;
    if scoped.is_none() && matches!(packet, FlowPacket::Input { .. } | FlowPacket::Keepalive) {
        // A legacy receiver cannot reject late input. Check our lease after
        // any wait for channel capacity and enqueue while it is still active.
        let state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
        if state.flow.forwarding() == Some(id) && state.flow.switch_id() == Some(token) {
            slot.send(Outgoing {
                message: packet.into_wire(None),
                result: None,
            });
        }
    } else {
        slot.send(Outgoing {
            message: packet.into_wire(scoped),
            result: None,
        });
    }
    Ok(())
}
pub async fn release_flow_control(core: &Core, peer: &str, token: Uuid) -> Result<(), String> {
    let link = session(core, peer);
    let pending = {
        let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
        if !state.stop_flow(peer, token) {
            return Err("Control has already moved. Check the current computer.".into());
        }
        let Ok(link) = link else {
            return Ok(());
        };
        if link.protocol < crate::link::SCOPED_FLOW {
            // Enqueue the untagged stop before a new legacy start. If the
            // queue is full, closing this connection releases its receiver.
            link.stop_untagged();
            return Ok(());
        }
        let stop = Outgoing {
            message: FlowPacket::Stop.into_wire(Some(token)),
            result: None,
        };
        match link.tx.try_send(stop) {
            Err(mpsc::error::TrySendError::Full(stop)) => (link, stop),
            _ => return Ok(()),
        }
    };
    // Local capture has already been released. A full queue may delay only
    // the notification for this visit, never the user's physical pointer.
    let (link, stop) = pending;
    if tokio::time::timeout(Duration::from_secs(2), link.tx.send(stop))
        .await
        .is_err()
    {
        let _ = link.cancel.send(true);
    }
    Ok(())
}
pub fn try_send(core: &Core, id: &str, message: Wire) {
    if let Ok(session) = session(core, id) {
        let _ = session.tx.try_send(Outgoing {
            message,
            result: None,
        });
    }
}
pub fn is_connected(core: &Core, id: &str) -> bool {
    session(core, id).is_ok()
}
pub fn clipboard_permitted(core: &Core, id: &str) -> bool {
    permit(core, id, false).is_ok()
}
pub async fn send_input(
    core: Arc<Core>,
    id: &str,
    event: Option<InputEvent>,
) -> Result<(), String> {
    crate::input::enabled(&core, id)?;
    {
        let state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
        if state.sending_to.as_deref() != Some(id)
            || state.pending_control.is_some()
            || state.flow.switch.peer().is_some()
        {
            return Err("Start focused control first.".into());
        }
    }
    if let Some(event) = &event {
        crate::input::validate(event)?;
    }
    let session = session(&core, id)?;
    let message = event
        .map(|event| Wire::Input { event })
        .unwrap_or(Wire::InputKeepalive);
    tokio::time::timeout(
        Duration::from_secs(3),
        session.tx.send(Outgoing {
            message,
            result: None,
        }),
    )
    .await
    .map_err(|_| "Peer stopped receiving input.".to_string())?
    .map_err(|_| "Peer disconnected.".into())
}
async fn finish_clipboard(
    core: &Core,
    id: &str,
    staged: &Path,
    format: &ClipboardFormat,
) -> Result<(), String> {
    use crate::clipboard::Content;
    match format {
        ClipboardFormat::Image { width, height } => {
            let rgba = tokio::fs::read(staged)
                .await
                .map_err(|_| "Cannot read received image.")?;
            format.validate(rgba.len() as u64)?;
            permit_transfer(core, id, Some(format))?;
            core.clipboard
                .write(Content::Image {
                    width: *width,
                    height: *height,
                    rgba,
                })
                .await
        }
        ClipboardFormat::Files(entries) => {
            let directory = tempfile::Builder::new()
                .prefix("Clipboard-")
                .tempdir_in(staged.parent().ok_or("Missing receive folder.")?)
                .map_err(|_| "Cannot create clipboard folder.")?;
            let mut source = tokio::fs::File::open(staged)
                .await
                .map_err(|_| "Cannot open received files.")?;
            let mut paths = Vec::new();
            for (name, size) in entries {
                let path = directory.path().join(safe_file_name(name)?);
                let mut out = tokio::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                    .await
                    .map_err(|_| "Cannot create copied file (check for duplicate names).")?;
                let n = tokio::io::copy(&mut (&mut source).take(*size), &mut out)
                    .await
                    .map_err(|_| "Cannot unpack copied files.")?;
                if n != *size {
                    return Err("Incomplete copied file.".into());
                }
                out.flush()
                    .await
                    .map_err(|_| "Cannot finish copied file.")?;
                paths.push(path);
            }
            permit_transfer(core, id, Some(format))?;
            core.clipboard.write(Content::Files(paths)).await?;
            let _ = directory.keep();
            Ok(())
        }
    }
}

pub async fn send_file(core: Arc<Core>, id: &str, path: &Path) -> Result<Transfer, String> {
    permit(&core, id, true)?;
    let session = session(&core, id)?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("Cannot open chosen file: {e}"))?;
    let meta = file.metadata().await.map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return Err("Choose a regular file of at most 128 MiB.".into());
    }
    let name = safe_file_name(
        path.file_name()
            .and_then(|p| p.to_str())
            .ok_or("File name is not UTF-8.")?,
    )?;
    send_stream(core, id, session, file, name, meta.len(), None).await
}

async fn send_stream(
    core: Arc<Core>,
    id: &str,
    session: Arc<Session>,
    mut file: impl AsyncRead + Unpin,
    name: String,
    bytes: u64,
    clipboard: Option<ClipboardFormat>,
) -> Result<Transfer, String> {
    let peer_name = core
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?
        .peers
        .iter()
        .find(|p| p.peer.id == id)
        .map(|p| p.peer.name.clone())
        .ok_or("Unknown peer.")?;
    let mut transfer = Transfer {
        id: Uuid::new_v4().to_string(),
        file_name: name.clone(),
        peer_name,
        direction: "outgoing".into(),
        bytes,
        status: "sending".into(),
        created_at: now(),
    };
    core.push_transfer(transfer.clone());
    let mut transfer_guard = OutgoingTransferGuard {
        core: core.clone(),
        session: session.clone(),
        id: transfer.id.clone(),
        complete: false,
    };
    let result: Result<(), String> = async {
        request(
            &session,
            Wire::FileStart {
                request: transfer.id.clone(),
                clipboard: clipboard.clone(),
                name,
                bytes,
            },
        )
        .await?;
        let mut buffer = vec![0u8; CHUNK];
        let mut total = 0u64;
        loop {
            permit_transfer(&core, id, clipboard.as_ref())?;
            let count = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > bytes {
                return Err("File changed while sending.".into());
            }
            // Allow the reader to prepare the next chunk as the writer sends
            // this one. Filling the shared queue with bulk data makes cursor
            // updates and handoffs wait behind up to a megabyte per transfer.
            let (dequeued, ready) = oneshot::channel();
            tokio::time::timeout(Duration::from_secs(5), async {
                session
                    .tx
                    .send(Outgoing {
                        message: Wire::FileChunk {
                            request: transfer.id.clone(),
                            data: STANDARD.encode(&buffer[..count]),
                        },
                        result: Some(dequeued),
                    })
                    .await
                    .map_err(|_| "Peer disconnected.".to_string())?;
                ready.await.map_err(|_| "Peer disconnected.".to_string())?
            })
            .await
            .map_err(|_| "Peer stopped receiving the file.".to_string())??;
        }
        if total != bytes {
            return Err("File changed while sending.".into());
        }
        request(
            &session,
            Wire::FileEnd {
                request: transfer.id.clone(),
            },
        )
        .await
    }
    .await;
    transfer.status = if result.is_ok() { "complete" } else { "failed" }.into();
    core.transfer_status(&transfer.id, &transfer.status);
    transfer_guard.complete = result.is_ok();
    result.map(|_| transfer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rediscovery_scans_current_subnets_without_self_or_stale_address() {
        let own = [Ipv4Addr::new(192, 168, 18, 7), Ipv4Addr::new(10, 5, 0, 2)];
        let stale: SocketAddr = "192.168.18.71:48372".parse().unwrap();
        let targets = scan_targets(&own, stale);
        assert_eq!(targets.len(), 254 * 2 - 3);
        assert!(targets.iter().all(|t| t.port() == 48372));
        assert!(!targets.contains(&"192.168.18.7:48372".parse().unwrap()));
        assert!(!targets.contains(&stale));
        assert!(targets.contains(&"192.168.18.20:48372".parse().unwrap()));
        assert!(targets.contains(&"10.5.0.9:48372".parse().unwrap()));
        let many: Vec<_> = (0..8).map(|n| Ipv4Addr::new(192, 168, n, 1)).collect();
        let limited = scan_targets(&many, stale);
        assert_eq!(limited.len(), MAX_SCAN_SUBNETS * 253);
    }

    fn encoded_test_invitation() -> EncodedInvitation {
        EncodedInvitation {
            candidates: vec!["192.168.1.20:48372".into(), "10.5.0.2:48372".into()],
            key: URL_SAFE_NO_PAD.encode([21u8; 32]),
            public_key: URL_SAFE_NO_PAD.encode([42u8; 32]),
            expires_at: 1_700_000_300,
        }
    }

    fn raw_invitation(value: &serde_json::Value) -> String {
        format!(
            "{INVITATION_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap())
        )
    }

    fn local_invite(core: &Arc<Core>, port: u16) -> PairingInvite {
        create_invite_for_addresses(core, vec![format!("127.0.0.1:{port}")]).unwrap()
    }

    #[test]
    fn invitation_round_trip_contains_all_addresses_and_pins_identity() {
        let encoded = encoded_test_invitation();
        let code = raw_invitation(&serde_json::to_value(&encoded).unwrap());
        let parsed = decode_invitation(&format!("  {code}\n"), 1_700_000_000).unwrap();
        assert_eq!(parsed.candidates.len(), 2);
        assert_eq!(parsed.candidates[0].to_string(), encoded.candidates[0]);
        assert_eq!(parsed.candidates[1].to_string(), encoded.candidates[1]);
        assert_eq!(parsed.key, [21u8; 32]);
        assert_eq!(parsed.public_key, [42u8; 32]);
    }

    #[test]
    fn invitation_rejects_expiry_invalid_schema_and_nonlocal_destinations() {
        let valid = serde_json::to_value(encoded_test_invitation()).unwrap();
        let code = raw_invitation(&valid);
        assert!(decode_invitation(&code, 1_700_000_300)
            .err()
            .unwrap()
            .contains("expired"));
        assert!(decode_invitation(&code, 1_699_999_999).is_err());
        assert!(
            decode_invitation(&code.replacen("v1:", "v2:", 1), 1_700_000_000)
                .err()
                .unwrap()
                .contains("version")
        );
        for malformed in [
            "",
            "abc",
            "traplink:v1:%",
            "traplink:v1:e30",
            "https://example.com/",
        ] {
            assert!(decode_invitation(malformed, 1_700_000_000).is_err());
        }
        assert!(decode_invitation(&"x".repeat(MAX_INVITATION_LENGTH + 1), 1_700_000_000).is_err());
        for address in [
            "8.8.8.8:48372",
            "example.com:48372",
            "192.168.1.20:80",
            "0.0.0.0:48372",
            "[2001:4860:4860::8888]:48372",
            "https://192.168.1.20:48372",
        ] {
            let mut invalid = valid.clone();
            invalid["candidates"] = serde_json::json!([address]);
            assert!(decode_invitation(&raw_invitation(&invalid), 1_700_000_000).is_err());
        }
        for candidates in [
            vec![],
            vec!["192.168.1.20:48372"; 2],
            vec!["192.168.1.20:48372"; 9],
        ] {
            let mut invalid = valid.clone();
            invalid["candidates"] = serde_json::json!(candidates);
            assert!(decode_invitation(&raw_invitation(&invalid), 1_700_000_000).is_err());
        }
        for field in ["key", "publicKey"] {
            for invalid_key in [
                "".to_string(),
                "!".to_string(),
                URL_SAFE_NO_PAD.encode([1u8; 31]),
            ] {
                let mut invalid = valid.clone();
                invalid[field] = serde_json::json!(invalid_key);
                assert!(decode_invitation(&raw_invitation(&invalid), 1_700_000_000).is_err());
            }
        }
        let mut invalid = valid.clone();
        invalid["extra"] = serde_json::json!(true);
        assert!(decode_invitation(&raw_invitation(&invalid), 1_700_000_000).is_err());
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("publicKey");
        assert!(decode_invitation(&raw_invitation(&missing), 1_700_000_000).is_err());
    }

    fn receiving_fixture() -> (tempfile::TempDir, Arc<Core>, Peer) {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let peer = Peer {
            flow_clipboard: false,
            id: Uuid::new_v4().to_string(),
            name: "Test peer".into(),
            address: "127.0.0.1:48373".into(),
            platform: "windows".into(),
            fingerprint: "test".into(),
            connected: true,
            trusted: true,
            last_seen: None,
            permissions: PeerPermissions {
                files: true,
                clipboard: false,
                input: false,
            },
        };
        core.data.lock().unwrap().peers.push(TrustedPeer {
            peer: peer.clone(),
            public_key: String::new(),
        });
        (directory, core, peer)
    }
    #[test]
    fn emergency_release_disconnects_only_the_active_peer_if_queues_are_full() {
        let (_directory, core, peer) = receiving_fixture();
        let mut receivers = Vec::new();
        for (id, busy) in [
            (&peer.id[..], true),
            ("busy-file-peer", true),
            ("idle-peer", false),
        ] {
            let (tx, messages) = mpsc::channel(1);
            let (cancel, cancelled) = watch::channel(false);
            let link = Arc::new(Session::new(tx, cancel, Uuid::new_v4()));
            if busy {
                link.tx
                    .try_send(Outgoing {
                        message: Wire::Ping,
                        result: None,
                    })
                    .unwrap();
            }
            core.sessions.lock().unwrap().insert(id.to_owned(), link);
            receivers.push((messages, cancelled));
        }
        core.data.lock().unwrap().settings.input_enabled = true;
        core.input.lock().unwrap().sending_to = Some(peer.id);
        crate::stop_all_input(&core, true);
        assert!(!core.data.lock().unwrap().settings.input_enabled);
        assert!(core.input.lock().unwrap().sending_to.is_none());
        assert!(*receivers[0].1.borrow());
        assert!(!*receivers[1].1.borrow());
        assert!(matches!(
            receivers[2].0.try_recv().unwrap().message,
            Wire::EmergencyStop
        ));
    }
    #[test]
    fn disconnected_or_replaced_sessions_cannot_process_queued_messages() {
        let (_directory, core, peer) = receiving_fixture();
        let (tx, _) = mpsc::channel(1);
        let (cancel, _) = watch::channel(false);
        let old = Uuid::new_v4();
        let current = Uuid::new_v4();
        core.sessions
            .lock()
            .unwrap()
            .insert(peer.id.clone(), Arc::new(Session::new(tx, cancel, current)));
        assert!(!is_current_session(&core, &peer.id, old));
        assert!(is_current_session(&core, &peer.id, current));
        disconnect(&core, &peer.id).unwrap();
        assert!(!is_current_session(&core, &peer.id, current));
    }

    #[tokio::test]
    async fn flow_starts_use_the_peer_protocol_and_keep_the_legacy_wire_shape() {
        for protocol in [crate::consent::PROTOCOL, crate::link::SCOPED_FLOW] {
            let (tx, mut rx) = mpsc::channel(1);
            let (cancel, _) = watch::channel(false);
            let link = Session {
                protocol,
                ..Session::new(tx, cancel, Uuid::new_v4())
            };
            let token = Uuid::new_v4();
            let start = flow_start_on(&link, token, FlowEdge::Right, 0.5, false);
            let ack = async {
                let outgoing = rx.recv().await.unwrap();
                let json = serde_json::to_value(&outgoing.message).unwrap();
                assert_eq!(json["kind"], "flow_start");
                assert_eq!(json["request"], token.to_string());
                if protocol == crate::link::SCOPED_FLOW {
                    assert_eq!(json["session"], token.to_string());
                } else {
                    assert!(json.get("session").is_none());
                    let decoded: Wire = serde_json::from_value(json).unwrap();
                    assert!(matches!(decoded, Wire::FlowStart { session: None, .. }));
                }
                outgoing.result.unwrap().send(Ok(())).unwrap();
            };
            tokio::join!(start, ack).0.unwrap();
        }
        assert!(matches!(
            FlowPacket::Stop.into_wire(None),
            Wire::ControlStop
        ));
        assert!(matches!(
            FlowPacket::Keepalive.into_wire(None),
            Wire::InputKeepalive
        ));
        assert!(matches!(
            FlowPacket::Return { entry: 0.5 }.into_wire(None),
            Wire::FlowReturn { .. }
        ));
    }

    #[tokio::test]
    async fn a_legacy_focused_start_waits_for_the_previous_flow_stop() {
        let (_directory, core, peer) = receiving_fixture();
        {
            let mut data = core.data.lock().unwrap();
            data.settings.input_enabled = true;
            data.peers[0].peer.permissions.input = true;
        }
        let (tx, mut rx) = mpsc::channel(2);
        let (cancel, _) = watch::channel(false);
        let link = Arc::new(Session {
            protocol: crate::consent::PROTOCOL,
            ..Session::new(tx, cancel, Uuid::new_v4())
        });
        core.sessions
            .lock()
            .unwrap()
            .insert(peer.id.clone(), link.clone());

        // A canceled Flow start still owns its transition until a delayed
        // acknowledgement has been answered with the final untagged stop.
        let previous = crate::flow::transition(&core).await.unwrap();
        let focused = set_control(core.clone(), &peer.id, true);
        tokio::pin!(focused);
        tokio::select! {
            biased;
            result = &mut focused => panic!("focused start completed early: {result:?}"),
            _ = std::future::ready(()) => {}
        }
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        send_flow_on(&core, &peer.id, &link, Uuid::new_v4(), FlowPacket::Stop)
            .await
            .unwrap();
        drop(previous);

        let acknowledge = async {
            assert!(matches!(
                rx.recv().await.unwrap().message,
                Wire::ControlStop
            ));
            let start = rx.recv().await.unwrap();
            assert!(matches!(start.message, Wire::ControlStart { .. }));
            start.result.unwrap().send(Ok(())).unwrap();
        };
        tokio::join!(focused, acknowledge).0.unwrap();
        assert!(crate::input::active_with(&core, &peer.id));
    }

    #[tokio::test]
    async fn returning_control_releases_local_capture_before_a_full_queue_drains() {
        let (_directory, core, peer) = receiving_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        let (cancel, _) = watch::channel(false);
        let link = Arc::new(Session::new(tx, cancel, Uuid::new_v4()));
        core.sessions
            .lock()
            .unwrap()
            .insert(peer.id.clone(), link.clone());
        link.tx
            .try_send(Outgoing {
                message: Wire::Ping,
                result: None,
            })
            .unwrap();
        let start = || {
            let mut input = core.input.lock().unwrap();
            input.sending_to = Some(peer.id.clone());
            let token = input
                .flow
                .begin_switch(&peer.id, FlowEdge::Left, Instant::now(), link.clone())
                .unwrap();
            assert!(input.flow.switch.apply(crate::flow::SwitchEvent::Started {
                peer: peer.id.clone(),
                ok: true
            }));
            token
        };
        let first = start();
        let release = tokio::spawn({
            let (core, id) = (core.clone(), peer.id.clone());
            async move { release_flow_control(&core, &id, first).await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while crate::input::active_with(&core, &peer.id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the occupied slot must not prevent a local release");
        let second = start();
        assert!(matches!(rx.recv().await.unwrap().message, Wire::Ping));
        release.await.unwrap().unwrap();
        assert!(
            matches!(rx.recv().await.unwrap().message, Wire::Flow { session, packet: FlowPacket::Stop } if session == first)
        );
        assert_eq!(core.input.lock().unwrap().flow.switch_id(), Some(second));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_returned_receiver_can_accept_a_new_visit_before_the_old_stop() {
        let (_dir, core, peer) = receiving_fixture();
        {
            let mut data = core.data.lock().unwrap();
            data.settings.input_enabled = true;
            data.peers[0].peer.permissions.input = true;
        }
        let mut files = HashMap::new();
        let start = |token: Uuid| Wire::FlowStart {
            request: token.to_string(),
            session: Some(token),
            edge: FlowEdge::Left,
            entry: 1.0,
            clipboard: false,
        };
        let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(matches!(
            incoming(&core, &peer, start(first), &mut files).await,
            Some(Wire::Ack { error: None, .. })
        ));
        assert!(matches!(
            incoming(&core, &peer, start(second), &mut files).await,
            Some(Wire::Ack { error: Some(_), .. })
        ));
        let movement = FlowPacket::Input {
            event: InputEvent::MoveBy { dx: 100, dy: 0 },
        };
        assert!(
            matches!(incoming_input(&core, &peer, Some(first), movement),
            Some(Wire::Flow { session, packet: FlowPacket::Return { .. } }) if session == first)
        );
        assert!(matches!(
            incoming(&core, &peer, start(second), &mut files).await,
            Some(Wire::Ack { error: None, .. })
        ));
        incoming_input(&core, &peer, Some(first), FlowPacket::Stop);
        assert!(incoming_input(&core, &peer, Some(second), FlowPacket::Keepalive).is_none());
        assert!(core.input.lock().unwrap().last_return.is_none());
        let movement = FlowPacket::Input {
            event: InputEvent::MoveBy { dx: 100, dy: 0 },
        };
        assert!(
            matches!(incoming_input(&core, &peer, Some(second), movement),
            Some(Wire::Flow { session, packet: FlowPacket::Return { .. } }) if session == second)
        );
        incoming_input(&core, &peer, Some(second), FlowPacket::Stop);
        assert!(!crate::input::active_with(&core, &peer.id));
        assert_eq!(
            core.input
                .lock()
                .unwrap()
                .last_return
                .as_ref()
                .unwrap()
                .peer,
            peer.id
        );
    }
    #[tokio::test]
    async fn abort_revocation_and_timeout_remove_partial_transfers() {
        let (_directory, core, peer) = receiving_fixture();
        let mut files = HashMap::new();
        let id = Uuid::new_v4().to_string();
        let reply = incoming(
            &core,
            &peer,
            Wire::FileStart {
                clipboard: None,
                request: id.clone(),
                name: "partial.bin".into(),
                bytes: 10,
            },
            &mut files,
        )
        .await;
        assert!(matches!(reply, Some(Wire::Ack { error: None, .. })));
        let path = files[&id].temporary.clone();
        assert!(path.exists());
        incoming(
            &core,
            &peer,
            Wire::FileAbort {
                request: id.clone(),
            },
            &mut files,
        )
        .await;
        assert!(files.is_empty());
        assert!(!path.exists());
        assert_eq!(core.transfers.lock().unwrap()[0].status, "failed");

        let id = Uuid::new_v4().to_string();
        incoming(
            &core,
            &peer,
            Wire::FileStart {
                clipboard: None,
                request: id.clone(),
                name: "revoked.bin".into(),
                bytes: 0,
            },
            &mut files,
        )
        .await;
        let path = files[&id].temporary.clone();
        core.data.lock().unwrap().settings.file_sharing_enabled = false;
        let reply = incoming(
            &core,
            &peer,
            Wire::FileEnd {
                request: id.clone(),
            },
            &mut files,
        )
        .await;
        assert!(matches!(reply, Some(Wire::Ack { error: Some(_), .. })));
        assert!(files.is_empty());
        assert!(!path.exists());

        core.data.lock().unwrap().settings.file_sharing_enabled = true;
        let id = Uuid::new_v4().to_string();
        incoming(
            &core,
            &peer,
            Wire::FileStart {
                clipboard: None,
                request: id.clone(),
                name: "stale.bin".into(),
                bytes: 10,
            },
            &mut files,
        )
        .await;
        let path = files[&id].temporary.clone();
        expire_incoming(&core, &mut files, Instant::now() + TRANSFER_IDLE_TIMEOUT);
        assert!(files.is_empty());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cancelled_sender_notifies_receiver_without_blocking() {
        let (_directory, core, _peer) = receiving_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        let (cancel, _) = watch::channel(false);
        let id = Uuid::new_v4().to_string();
        let session = Arc::new(Session::new(tx, cancel, Uuid::new_v4()));
        let guard = OutgoingTransferGuard {
            core,
            session,
            id: id.clone(),
            complete: false,
        };
        drop(guard);
        assert!(
            matches!(rx.recv().await.unwrap().message, Wire::FileAbort { request } if request == id)
        );
    }
    #[tokio::test]
    async fn consent_requests_prompt_once_and_never_grant_by_themselves() {
        let (_directory, core, peer) = receiving_fixture();
        let mut files = HashMap::new();
        let ask = |request: &str| Wire::InputConsent {
            request: request.into(),
            flow: true,
        };
        assert!(incoming(&core, &peer, ask("1"), &mut files).await.is_none());
        let pending = core.consent.lock().unwrap().pending(Instant::now());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].peer_name, "Test peer");
        assert!(pending[0].flow);
        assert!(crate::input::enabled(&core, &peer.id).is_err());
        assert!(!core.data.lock().unwrap().settings.input_enabled);
        let reply = incoming(&core, &peer, ask("2"), &mut files).await;
        assert!(
            matches!(reply, Some(Wire::Ack { request, error: Some(e) }) if request == "2" && e == crate::consent::BUSY)
        );
        {
            let mut data = core.data.lock().unwrap();
            data.settings.input_enabled = true;
            data.peers[0].peer.permissions.input = true;
        }
        let reply = incoming(&core, &peer, ask("3"), &mut files).await;
        assert!(matches!(reply, Some(Wire::Ack { request, error: None }) if request == "3"));
    }
    #[test]
    fn consent_message_round_trips_and_defaults_flow() {
        let wire: Wire = serde_json::from_str(r#"{"kind":"input_consent","request":"r"}"#).unwrap();
        assert!(matches!(wire, Wire::InputConsent { ref request, flow: false } if request == "r"));
        let text = serde_json::to_string(&Wire::InputConsent {
            request: "r".into(),
            flow: true,
        })
        .unwrap();
        assert_eq!(
            text,
            r#"{"kind":"input_consent","request":"r","flow":true}"#
        );
    }
    #[tokio::test]
    async fn consent_is_never_sent_to_older_peers() {
        let (_directory, core, peer) = receiving_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        let (cancel, _) = watch::channel(false);
        let session = Session {
            protocol: crate::link::PROBING,
            ..Session::new(tx, cancel, Uuid::new_v4())
        };
        core.sessions
            .lock()
            .unwrap()
            .insert(peer.id.clone(), Arc::new(session));
        let result = request_input_consent(core.clone(), &peer.id, true).await;
        assert_eq!(result, Err(crate::consent::OUTDATED.to_string()));
        assert!(
            rx.try_recv().is_err(),
            "nothing was queued for the old peer"
        );
    }
    #[tokio::test]
    async fn requester_waits_for_the_answer_and_allows_one_request_per_peer() {
        let (_directory, core, peer) = receiving_fixture();
        let (tx, mut rx) = mpsc::channel(4);
        let (cancel, _) = watch::channel(false);
        core.sessions.lock().unwrap().insert(
            peer.id.clone(),
            Arc::new(Session::new(tx, cancel, Uuid::new_v4())),
        );
        let waiting = tokio::spawn({
            let (core, id) = (core.clone(), peer.id.clone());
            async move { request_input_consent(core, &id, false).await }
        });
        let sent = rx.recv().await.unwrap();
        assert!(matches!(
            sent.message,
            Wire::InputConsent { flow: false, .. }
        ));
        assert_eq!(
            request_input_consent(core.clone(), &peer.id, false).await,
            Err(crate::consent::BUSY.to_string())
        );
        sent.result
            .unwrap()
            .send(Err(crate::consent::DECLINED.into()))
            .unwrap();
        assert_eq!(
            waiting.await.unwrap(),
            Err(crate::consent::DECLINED.to_string())
        );
        assert!(core
            .consent
            .lock()
            .unwrap()
            .begin_outgoing(&peer.id, Instant::now())
            .is_ok());
    }
    #[test]
    fn path_and_address_boundary() {
        for path in [
            "../secret",
            "..\\secret",
            "file:stream",
            "foo.",
            "a/b",
            "",
            "..",
        ] {
            assert!(safe_file_name(path).is_err());
        }
        assert_eq!(safe_file_name("example.txt").unwrap(), "example.txt");
        assert!(allowed_address("8.8.8.8:48372").is_err());
        assert!(allowed_address("192.168.1.5:48372").is_ok());
    }
    #[tokio::test]
    async fn rejects_oversized_frames_before_allocation() {
        let (mut sender, mut receiver) = tokio::io::duplex(16);
        sender.write_u32(1_000_000).await.unwrap();
        assert!(read_frame(&mut receiver).await.is_err());
    }
    #[test]
    fn noise_requires_matching_pairing_key_and_rejects_tampering() {
        let key = [42u8; 32];
        let builder = || snow::Builder::new(NOISE_PAIR.parse().unwrap());
        let a = builder().generate_keypair().unwrap();
        let b = builder().generate_keypair().unwrap();
        let mut initiator = builder()
            .local_private_key(&a.private)
            .unwrap()
            .psk(3, &key)
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut responder = builder()
            .local_private_key(&b.private)
            .unwrap()
            .psk(3, &key)
            .unwrap()
            .build_responder()
            .unwrap();
        let mut out = [0u8; 1024];
        let mut plain = [0u8; 1024];
        let n = initiator.write_message(&[], &mut out).unwrap();
        responder.read_message(&out[..n], &mut plain).unwrap();
        let n = responder.write_message(&[], &mut out).unwrap();
        initiator.read_message(&out[..n], &mut plain).unwrap();
        let n = initiator.write_message(&[], &mut out).unwrap();
        responder.read_message(&out[..n], &mut plain).unwrap();
        let mut a = initiator.into_transport_mode().unwrap();
        let mut b = responder.into_transport_mode().unwrap();
        let n = a.write_message(b"test", &mut out).unwrap();
        out[0] ^= 1;
        assert!(b.read_message(&out[..n], &mut plain).is_err());
    }

    #[tokio::test]
    async fn invitation_falls_back_after_connection_and_identity_failure() {
        let first_dir = tempfile::tempdir().unwrap();
        let wrong_dir = tempfile::tempdir().unwrap();
        let client_dir = tempfile::tempdir().unwrap();
        let correct_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let wrong_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let correct_address = correct_listener.local_addr().unwrap();
        let wrong_address = wrong_listener.local_addr().unwrap();
        let closed_address = closed_listener.local_addr().unwrap();
        drop(closed_listener);
        let first = Core::for_test(first_dir.path().to_path_buf(), correct_address.port());
        let wrong = Core::for_test(wrong_dir.path().to_path_buf(), wrong_address.port());
        let client = Core::for_test(client_dir.path().to_path_buf(), 48372);
        first.listening.store(true, Ordering::Relaxed);
        wrong.listening.store(true, Ordering::Relaxed);
        let invite = create_invite_for_addresses(
            &first,
            vec![
                closed_address.to_string(),
                wrong_address.to_string(),
                correct_address.to_string(),
            ],
        )
        .unwrap();
        local_invite(&wrong, wrong_address.port());
        wrong.invite.lock().unwrap().as_mut().unwrap().key =
            first.invite.lock().unwrap().as_ref().unwrap().key;
        let correct_core = first.clone();
        let correct_task = tokio::spawn(async move {
            let (stream, address) = correct_listener.accept().await.unwrap();
            accept(correct_core, stream, address).await
        });
        let wrong_core = wrong.clone();
        let wrong_task = tokio::spawn(async move {
            let (stream, address) = wrong_listener.accept().await.unwrap();
            accept(wrong_core, stream, address).await
        });
        let peer = tokio::time::timeout(
            Duration::from_secs(3),
            connect_invitation(client.clone(), &invite.invitation),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            peer.fingerprint,
            fingerprint(&first.identity.as_ref().unwrap().public)
        );
        assert_eq!(peer.address, correct_address.to_string());
        assert!(correct_task.await.unwrap().is_ok());
        assert!(wrong_task.await.unwrap().is_err());
        assert!(wrong.data.lock().unwrap().peers.is_empty());
        assert!(!wrong.invite.lock().unwrap().as_ref().unwrap().reserved);
        assert!(first.invite.lock().unwrap().is_none());
        disconnect(&client, &peer.id).unwrap();
    }

    #[tokio::test]
    async fn two_instances_pair_transfer_and_reconnect() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let mut attempt = None;
        for _ in 0..5 {
            let socket = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
            let port = socket.local_addr().unwrap().port();
            drop(socket);
            let first = Core::for_test(first_dir.path().to_path_buf(), port);
            let task = tokio::spawn(listen(first.clone()));
            for _ in 0..300 {
                if first.listening.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if first.listening.load(Ordering::Relaxed) {
                attempt = Some((first, port, task));
                break;
            }
            task.abort();
        }
        let (first, port, listener_task) = attempt.expect("could not start the LAN listener");
        let second = Core::for_test(
            second_dir.path().to_path_buf(),
            if port == u16::MAX { port - 1 } else { port + 1 },
        );
        let invite = local_invite(&first, port);
        let connected = connect_invitation(second.clone(), &invite.invitation)
            .await
            .unwrap();
        let second_id = second.data.lock().unwrap().device_id.clone();
        for _ in 0..100 {
            if first.sessions.lock().unwrap().contains_key(&second_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(first.sessions.lock().unwrap().contains_key(&second_id));
        assert!(first.invite.lock().unwrap().is_none());
        assert!(connect_invitation(second.clone(), &invite.invitation)
            .await
            .is_err());
        for core in [&first, &second] {
            let data = core.data.lock().unwrap();
            let permissions = &data.peers[0].peer.permissions;
            assert!(!permissions.files && !permissions.clipboard && !permissions.input);
        }
        assert!(
            send_file(second.clone(), &connected.id, Path::new("unused"))
                .await
                .is_err()
        );
        first.data.lock().unwrap().peers[0].peer.permissions.files = true;
        second.data.lock().unwrap().peers[0].peer.permissions.files = true;
        let payload: Vec<u8> = (0..180_000).map(|i| (i % 251) as u8).collect();
        let chosen = second_dir.path().join("chosen.bin");
        std::fs::write(&chosen, &payload).unwrap();
        let transfer = send_file(second.clone(), &connected.id, &chosen)
            .await
            .unwrap();
        assert_eq!(transfer.status, "complete");
        let received_dir = first_dir.path().join("downloads").join("TrapLink");
        let received = std::fs::read_dir(received_dir)
            .unwrap()
            .filter_map(Result::ok)
            .find(|f| f.file_name().to_string_lossy().ends_with("chosen.bin"))
            .unwrap();
        assert_eq!(std::fs::read(received.path()).unwrap(), payload);
        use crate::clipboard::Content;
        for core in [&first, &second] {
            let mut data = core.data.lock().unwrap();
            data.settings.clipboard_enabled = true;
            data.peers[0].peer.permissions.clipboard = true;
        }
        first
            .clipboard
            .write(Content::Image {
                width: 300,
                height: 150,
                rgba: payload.clone(),
            })
            .await
            .unwrap();
        send_clipboard(first.clone(), &second_id).await.unwrap();
        assert!(
            matches!(second.clipboard.read().await.unwrap(), Some(Content::Image { width: 300, height: 150, rgba }) if rgba == payload)
        );
        let empty = second_dir.path().join("empty.txt");
        std::fs::write(&empty, []).unwrap();
        second
            .clipboard
            .write(Content::Files(vec![empty, chosen]))
            .await
            .unwrap();
        first.data.lock().unwrap().peers[0].peer.permissions.files = false;
        assert!(send_clipboard(second.clone(), &connected.id).await.is_err());
        first.data.lock().unwrap().peers[0].peer.permissions.files = true;
        send_clipboard(second.clone(), &connected.id).await.unwrap();
        let Some(Content::Files(paths)) = first.clipboard.read().await.unwrap() else {
            panic!("expected copied files");
        };
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("empty.txt") && std::fs::read(&paths[0]).unwrap().is_empty());
        assert!(paths[1].ends_with("chosen.bin"));
        assert_eq!(std::fs::read(&paths[1]).unwrap(), payload);
        disconnect(&second, &connected.id).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let reconnected = connect(
            second.clone(),
            format!("127.0.0.1:{port}"),
            None,
            Some(connected.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(reconnected.id, connected.id);
        assert_eq!(reconnected.fingerprint, connected.fingerprint);
        disconnect(&second, &connected.id).unwrap();
        listener_task.abort();
    }
    #[tokio::test]
    async fn cancelling_pairing_releases_invite_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let core = Core::for_test(directory.path().to_path_buf(), port);
        core.listening.store(true, Ordering::Relaxed);
        local_invite(&core, port);
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let (stream, address) = listener.accept().await.unwrap();
        client.write_u8(1).await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(30),
            accept(core.clone(), stream, address),
        )
        .await;
        assert!(result.is_err());
        assert!(!core.invite.lock().unwrap().as_ref().unwrap().reserved);
    }
}

#[cfg(all(test, windows))]
#[path = "network/handoff_tests.rs"]
mod handoff_tests;
