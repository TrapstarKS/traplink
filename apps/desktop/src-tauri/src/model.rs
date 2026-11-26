use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn now() -> String {
    Utc::now().to_rfc3339()
}
pub fn platform() -> String {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
    .into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub device_name: String,
    pub auth_server_url: String,
    pub clipboard_enabled: bool,
    pub file_sharing_enabled: bool,
    pub input_enabled: bool,
    pub start_on_login: bool,
    pub theme: String,
    pub download_directory: String,
    pub port: u16,
    #[serde(default)]
    pub remap_enabled: bool,
}
impl Settings {
    pub fn defaults(downloads: String) -> Self {
        Self {
            device_name: std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "My computer".into()),
            auth_server_url: "https://traplink.traphub.xyz".into(),
            clipboard_enabled: false,
            file_sharing_enabled: false,
            input_enabled: false,
            start_on_login: false,
            theme: "dark".into(),
            download_directory: downloads,
            port: 48372,
            remap_enabled: false,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.device_name.trim().is_empty() || self.device_name.chars().count() > 80 {
            return Err("Device name must contain 1–80 characters.".into());
        }
        validate_server_url(&self.auth_server_url)?;
        if self.input_enabled && !cfg!(any(target_os = "windows", target_os = "macos")) {
            return Err("Input sharing is available only on Windows and macOS.".into());
        }
        if self.remap_enabled && !cfg!(target_os = "windows") {
            return Err("Remapping is available only on Windows.".into());
        }
        if !["dark", "light", "system"].contains(&self.theme.as_str()) {
            return Err("Unknown theme.".into());
        }
        if self.port < 1024 {
            return Err("Choose a LAN port between 1024 and 65535.".into());
        }
        if self.download_directory.trim().is_empty()
            || !std::path::Path::new(&self.download_directory).is_absolute()
        {
            return Err("Choose an absolute downloads folder.".into());
        }
        Ok(())
    }
}
pub fn validate_server_url(value: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(value).map_err(|_| "Enter a valid authentication server URL.")?;
    let local = matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if !(parsed.scheme() == "https" || (parsed.scheme() == "http" && local))
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Use HTTPS for authentication, or HTTP on localhost for development. URL credentials, queries and fragments are not allowed.".into());
    }
    Ok(parsed)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub transport: String,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub is_logitech: bool,
    pub capabilities: Vec<String>,
    pub input_kinds: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PeerPermissions {
    pub clipboard: bool,
    pub files: bool,
    #[serde(default)]
    pub input: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    Key {
        key: String,
        down: bool,
    },
    Move {
        x: f64,
        y: f64,
    },
    Button {
        button: u8,
        down: bool,
    },
    Scroll {
        x: i32,
        y: i32,
    },
    MoveBy {
        dx: i32,
        dy: i32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowEdge {
    Left,
    Right,
    Top,
    Bottom,
}
impl FlowEdge {
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }
    pub fn perpendicular(self) -> [Self; 2] {
        match self {
            Self::Left | Self::Right => [Self::Top, Self::Bottom],
            Self::Top | Self::Bottom => [Self::Left, Self::Right],
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowMode {
    #[default]
    Edge,
    CtrlEdge,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    #[serde(default)]
    pub flow_clipboard: bool,
    pub id: String,
    pub name: String,
    pub address: String,
    pub platform: String,
    pub fingerprint: String,
    pub connected: bool,
    pub trusted: bool,
    pub last_seen: Option<String>,
    #[serde(default)]
    pub permissions: PeerPermissions,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedPeer {
    pub peer: Peer,
    pub public_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyMapping {
    pub from: String,
    pub to: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub application: Option<String>,
    pub device_id: Option<String>,
    pub mappings: Vec<KeyMapping>,
    pub created_at: String,
    pub updated_at: String,
}
impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        Uuid::parse_str(&self.id).map_err(|_| "Invalid profile ID.")?;
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return Err("Profile name must contain 1–80 characters.".into());
        }
        if self.mappings.len() > 128 {
            return Err("A profile can contain at most 128 mappings.".into());
        }
        if self.application.as_ref().is_some_and(|s| s.len() > 1024)
            || self.device_id.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err("Profile identifier is too long.".into());
        }
        if self
            .mappings
            .iter()
            .any(|m| m.from.is_empty() || m.to.is_empty() || m.from.len() > 80 || m.to.len() > 80)
        {
            return Err("Each mapping needs source and target keys of up to 80 characters.".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub profile_count: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub file_name: String,
    pub peer_name: String,
    pub direction: String,
    pub bytes: u64,
    pub status: String,
    pub created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub email: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub version: String,
    pub settings: Settings,
    pub devices: Vec<Device>,
    pub peers: Vec<Peer>,
    pub profiles: Vec<Profile>,
    pub backups: Vec<Backup>,
    pub transfers: Vec<Transfer>,
    pub listening: bool,
    pub local_addresses: Vec<String>,
    pub pairing_code: Option<String>,
    pub account: Option<Account>,
    pub permissions: Permissions,
    pub warnings: Vec<String>,
    /// True when at least one warning can be dismissed from the app.
    pub warnings_dismissible: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub accessibility: bool,
    pub input_monitoring: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInvite {
    pub invitation: String,
    pub code: String,
    pub address: String,
    pub expires_at: String,
    pub fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persisted {
    pub schema_version: u32,
    pub device_id: String,
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub peers: Vec<TrustedPeer>,
    #[serde(default)]
    pub flow_peers: Vec<SavedFlow>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFlow {
    pub peer: String,
    pub edge: FlowEdge,
    #[serde(default)]
    pub mode: FlowMode,
    pub delay_ms: u64,
    pub block_while_button: bool,
    #[serde(default)]
    pub clipboard: bool,
}
impl Persisted {
    pub fn forget_flow(&mut self, peer: &str) -> bool {
        let before = self.flow_peers.len();
        self.flow_peers.retain(|f| f.peer != peer);
        self.flow_peers.len() != before
    }
}
