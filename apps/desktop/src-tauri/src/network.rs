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
    net::{IpAddr, SocketAddr},
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
}
pub struct Outgoing {
    pub message: Wire,
    pub result: Option<oneshot::Sender<Result<(), String>>>,
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
    Input {
        event: InputEvent,
    },
    InputKeepalive,
    FlowStart {
        request: String,
        edge: FlowEdge,
        entry: f64,
        clipboard: bool,
    },
    FlowReturn {
        entry: f64,
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
                core.network_changed.notified().await;
                continue;
            }
        };
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
    let (rtt, farewell) = (session.rtt_ms.clone(), session.farewell.clone());
    if let Some(old) = core
        .sessions
        .lock()
        .map_err(|_| "Network lock unavailable.")?
        .insert(peer.id.clone(), session)
    {
        crate::input::stop_peer(&core, &peer.id);
        let _ = old.cancel.send(true);
    }
    drop(data);
    tokio::spawn(run_session(
        core, stream, noise, peer, rx, cancel_rx, generation, protocol, rtt, farewell,
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
    if value.is_empty()
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
            crate::input::stop_peer(core, &peer.id);
            None
        }
        Wire::Input { event } => match crate::input::receive(core, &peer.id, event) {
            Err(error) => {
                crate::input::stop_peer(core, &peer.id);
                core.warning(error);
                Some(Wire::ControlStop)
            }
            Ok(Some(info)) => {
                if info.clipboard && clipboard_permitted(core, &peer.id) {
                    let (core, id) = (core.clone(), peer.id.clone());
                    tokio::spawn(async move {
                        let _ = send_clipboard(core, &id).await;
                    });
                }
                Some(Wire::FlowReturn { entry: info.entry })
            }
            Ok(None) => None,
        },
        Wire::FlowStart {
            request,
            edge,
            entry,
            clipboard,
        } => Some(Wire::Ack {
            request,
            error: crate::input::start_flow_receiving(core, &peer.id, edge, entry, clipboard).err(),
        }),
        Wire::FlowReturn { entry } => {
            if let Err(error) = crate::flow::returned(core, &peer.id, entry) {
                core.warning(error);
            }
            None
        }
        Wire::InputConsent { request, flow } => consent_requested(core, peer, request, flow),
        Wire::InputKeepalive => {
            if crate::input::keepalive(core, &peer.id).is_err() {
                return Some(Wire::ControlStop);
            }
            None
        }
        Wire::Clipboard { request, text } => {
            let result = permit(core, &peer.id, false).and_then(|_| {
                if text.len() <= MAX_CLIPBOARD {
                    Ok(())
                } else {
                    Err("Clipboard text exceeds 24 KiB.".into())
                }
            });
            let result = if result.is_ok() {
                tokio::task::spawn_blocking(move || {
                    arboard::Clipboard::new()
                        .and_then(|mut c| c.set_text(text))
                        .map_err(|_| "Cannot write to the system clipboard.".to_string())
                })
                .await
                .unwrap_or_else(|_| Err("Clipboard task failed.".into()))
            } else {
                result
            };
            Some(Wire::Ack {
                request,
                error: result.err(),
            })
        }
        Wire::FileStart {
            request,
            name,
            bytes,
        } => {
            let result: Result<(), String> = async {
                permit(core, &peer.id, true)?;
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
                permit(core, &peer.id, true)?;
                let file = files.get_mut(&request).ok_or("Unknown transfer.")?;
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
                permit(core, &peer.id, true)?;
                let mut file = staged.ok_or("Unknown transfer.")?;
                if file.expected != file.received {
                    return Err("Received file length did not match the declared size.".into());
                }
                let mut handle = file.file.take().ok_or("Received file is closed.")?;
                handle.flush().await.map_err(|e| e.to_string())?;
                handle.sync_all().await.map_err(|e| e.to_string())?;
                drop(handle);
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
    generation: Uuid,
    protocol: u32,
    rtt: Arc<AtomicU32>,
    farewell: Arc<AtomicBool>,
) {
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
            biased;
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
    for (id, _) in &files {
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
fn session(core: &Core, id: &str) -> Result<Arc<Session>, String> {
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
pub async fn send_clipboard(core: Arc<Core>, id: &str) -> Result<(), String> {
    permit(&core, id, false)?;
    let session = session(&core, id)?;
    let text = tokio::task::spawn_blocking(|| {
        arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .map_err(|_| "Cannot read text from the system clipboard.".to_string())
    })
    .await
    .map_err(|_| "Clipboard task failed.".to_string())??;
    if text.len() > MAX_CLIPBOARD {
        return Err("Clipboard text exceeds 24 KiB.".into());
    }
    request(
        &session,
        Wire::Clipboard {
            request: Uuid::new_v4().to_string(),
            text,
        },
    )
    .await
}
pub async fn set_control(core: Arc<Core>, id: &str, enabled: bool) -> Result<(), String> {
    let session = session(&core, id)?;
    if !enabled {
        crate::input::stop_peer(&core, id);
        return tokio::time::timeout(
            Duration::from_secs(3),
            session.tx.send(Outgoing {
                message: Wire::ControlStop,
                result: None,
            }),
        )
        .await
        .map_err(|_| "Peer did not receive the stop request.".to_string())?
        .map_err(|_| "Peer disconnected.".into());
    }
    crate::input::enabled(&core, id)?;
    {
        let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
        if state.busy() {
            return Err("Stop the existing input session first.".into());
        }
        state.sending_to = Some(id.into());
    }
    let result = request(
        &session,
        Wire::ControlStart {
            request: Uuid::new_v4().to_string(),
        },
    )
    .await;
    if result.is_err() {
        crate::input::stop_peer(&core, id);
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
pub async fn flow_start(
    core: &Core,
    id: &str,
    edge: FlowEdge,
    entry: f64,
    clipboard: bool,
) -> Result<(), String> {
    crate::flow::validate_entry(entry)?;
    let session = session(core, id)?;
    request(
        &session,
        Wire::FlowStart {
            request: Uuid::new_v4().to_string(),
            edge,
            entry,
            clipboard,
        },
    )
    .await
}
pub async fn send_message(core: &Core, id: &str, message: Wire) -> Result<(), String> {
    let session = session(core, id)?;
    tokio::time::timeout(
        Duration::from_secs(2),
        session.tx.send(Outgoing {
            message,
            result: None,
        }),
    )
    .await
    .map_err(|_| "Peer stopped receiving input.".to_string())?
    .map_err(|_| "Peer disconnected.".into())
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
    if core
        .input
        .lock()
        .map_err(|_| "Input lock unavailable.")?
        .sending_to
        .as_deref()
        != Some(id)
    {
        return Err("Start focused control first.".into());
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
pub async fn send_file(core: Arc<Core>, id: &str, path: &Path) -> Result<Transfer, String> {
    permit(&core, id, true)?;
    let session = session(&core, id)?;
    let mut file = tokio::fs::File::open(path)
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
        bytes: meta.len(),
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
                name,
                bytes: meta.len(),
            },
        )
        .await?;
        let mut buffer = vec![0u8; CHUNK];
        let mut total = 0u64;
        loop {
            permit(&core, id, true)?;
            let count = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > meta.len() {
                return Err("File changed while sending.".into());
            }
            tokio::time::timeout(
                Duration::from_secs(5),
                session.tx.send(Outgoing {
                    message: Wire::FileChunk {
                        request: transfer.id.clone(),
                        data: STANDARD.encode(&buffer[..count]),
                    },
                    result: None,
                }),
            )
            .await
            .map_err(|_| "Peer stopped receiving the file.".to_string())?
            .map_err(|_| "Peer disconnected.".to_string())?;
        }
        if total != meta.len() {
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
    async fn abort_revocation_and_timeout_remove_partial_transfers() {
        let (_directory, core, peer) = receiving_fixture();
        let mut files = HashMap::new();
        let id = Uuid::new_v4().to_string();
        let reply = incoming(
            &core,
            &peer,
            Wire::FileStart {
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
        let wire: Wire =
            serde_json::from_str(r#"{"kind":"input_consent","request":"r"}"#).unwrap();
        assert!(matches!(wire, Wire::InputConsent { ref request, flow: false } if request == "r"));
        let text = serde_json::to_string(&Wire::InputConsent {
            request: "r".into(),
            flow: true,
        })
        .unwrap();
        assert_eq!(text, r#"{"kind":"input_consent","request":"r","flow":true}"#);
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
        assert!(rx.try_recv().is_err(), "nothing was queued for the old peer");
    }
    #[tokio::test]
    async fn requester_waits_for_the_answer_and_allows_one_request_per_peer() {
        let (_directory, core, peer) = receiving_fixture();
        let (tx, mut rx) = mpsc::channel(4);
        let (cancel, _) = watch::channel(false);
        core.sessions
            .lock()
            .unwrap()
            .insert(peer.id.clone(), Arc::new(Session::new(tx, cancel, Uuid::new_v4())));
        let waiting = tokio::spawn({
            let (core, id) = (core.clone(), peer.id.clone());
            async move { request_input_consent(core, &id, false).await }
        });
        let sent = rx.recv().await.unwrap();
        assert!(matches!(sent.message, Wire::InputConsent { flow: false, .. }));
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
        assert!(core.consent.lock().unwrap().begin_outgoing(&peer.id, Instant::now()).is_ok());
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
