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
/// Longest device name, in Unicode code points. The server and the Preferences form use
/// the same limit (apps/web/src/server/devices.ts, apps/desktop/src/lib/device-name.ts).
pub const MAX_DEVICE_NAME_CHARS: usize = 80;

/// Trims like JavaScript `String.prototype.trim`, which the server applies before it checks
/// the name. Rust `str::trim` also strips U+0085 and keeps U+FEFF, so it is not used here.
pub fn trim_device_name(name: &str) -> &str {
    name.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}

/// The device-name rule shared with the server: after trimming, 1 to 80 code points and no
/// control characters (Unicode category Cc). Returns the trimmed name to store.
pub fn validate_device_name(name: &str) -> Result<&str, String> {
    let name = trim_device_name(name);
    let length = name.chars().count();
    if length == 0 || length > MAX_DEVICE_NAME_CHARS || name.chars().any(char::is_control) {
        return Err("Device name must contain 1–80 characters without control characters.".into());
    }
    Ok(name)
}

/// Turns a stored or restored name into one the server accepts, or `None` when nothing usable
/// is left. Older versions allowed tabs and other control characters.
pub fn sanitize_device_name(name: &str) -> Option<String> {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = trim_device_name(&cleaned)
        .chars()
        .take(MAX_DEVICE_NAME_CHARS)
        .collect();
    let cleaned = trim_device_name(&cleaned);
    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

/// The name to store when the user saves Preferences. A changed name must follow the shared
/// rule; an unchanged one is kept, so settings from older versions can still be saved.
/// `previous` holds the stored name and, for a save made against an older snapshot, the name
/// that snapshot showed (the merge keeps the stored name when the draft did not change it).
pub fn accept_device_name(name: &str, previous: &[&str]) -> Result<String, String> {
    if previous.contains(&name) {
        return Ok(name.to_owned());
    }
    validate_device_name(name).map(str::to_owned)
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
    Key { key: String, down: bool },
    Move { x: f64, y: f64 },
    Button { button: u8, down: bool },
    Scroll { x: i32, y: i32 },
    MoveBy { dx: i32, dy: i32 },
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
    /// The server rejected this computer's credential, so it was signed out.
    pub account_expired: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    // Same table as apps/web/tests/device-name.test.ts, so both sides agree.
    #[test]
    fn device_name_rule_matches_the_server() {
        let emoji80 = "\u{1f5a5}".repeat(80);
        let mixed = format!("{}\u{1f3ae}", "a".repeat(79));
        let a80 = "a".repeat(80);
        let a81 = "a".repeat(81);
        let emoji81 = "\u{1f5a5}".repeat(81);
        let cases: [(&str, bool); 17] = [
            ("Work PC", true),
            ("  Work PC  ", true),
            ("\tWork PC\n", true),
            ("\u{feff}Work PC", true),
            (&a80, true),
            (&mixed, true),
            (&emoji80, true),
            (&a81, false),
            (&emoji81, false),
            ("", false),
            ("   ", false),
            ("\u{3000}", false),
            ("Work\tPC", false),
            ("Work\u{0}PC", false),
            ("Work\u{7f}PC", false),
            ("\u{85}Work PC", false),
            ("Work\u{9f}PC", false),
        ];
        for (name, valid) in cases {
            assert_eq!(validate_device_name(name).is_ok(), valid, "{name:?}");
        }
        assert_eq!(validate_device_name("  Studio Mac  "), Ok("Studio Mac"));
    }

    #[test]
    fn stored_names_are_cleaned_before_registration() {
        assert_eq!(sanitize_device_name("Work\tPC").as_deref(), Some("WorkPC"));
        assert_eq!(sanitize_device_name(" \u{7}  ").as_deref(), None);
        let long = format!("{} b", "a".repeat(79));
        assert_eq!(sanitize_device_name(&long), Some("a".repeat(79)));
        let clean = sanitize_device_name(&"\u{1f5a5}".repeat(90)).unwrap();
        assert!(validate_device_name(&clean).is_ok());
    }

    #[test]
    fn preferences_reject_new_names_the_server_would_reject() {
        assert!(accept_device_name("PC\tHome", &["My computer"]).is_err());
        assert!(accept_device_name("Home\u{9f}", &["My computer"]).is_err());
        assert_eq!(
            accept_device_name("  Home PC  ", &["My computer"]).as_deref(),
            Ok("Home PC")
        );
        // A name stored by an older version can still be kept while other settings change.
        assert_eq!(
            accept_device_name("PC\tHome", &["PC\tHome"]).as_deref(),
            Ok("PC\tHome")
        );
        // A draft from before a background rename still shows the old legacy name. Saving an
        // unrelated setting must not fail over a name the user never edited.
        assert_eq!(
            accept_device_name("PC\tHome", &["Restored PC", "PC\tHome"]).as_deref(),
            Ok("PC\tHome")
        );
    }
}
