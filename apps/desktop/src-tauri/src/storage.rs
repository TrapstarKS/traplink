use crate::model::*;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;
const MAX_DOCUMENT: u64 = 8 * 1024 * 1024;

pub fn load(directory: &Path, downloads: String) -> Result<Persisted, String> {
    fs::create_dir_all(directory).map_err(|e| format!("Cannot create app data folder: {e}"))?;
    let path = directory.join("settings.json");
    if path.exists() {
        let mut data: Persisted = read_document(&path)?;
        if data.schema_version != SCHEMA_VERSION {
            return Err("This settings file is from an unsupported TrapLink version. It has not been changed.".into());
        }
        Uuid::parse_str(&data.device_id).map_err(|_| "Invalid installation ID in settings.")?;
        data.settings.validate()?;
        for p in &data.profiles {
            p.validate()?;
        }
        for p in &mut data.peers {
            p.peer.connected = false;
        }
        sanitize_flow(&mut data);
        Ok(data)
    } else {
        let data = Persisted {
            schema_version: SCHEMA_VERSION,
            device_id: Uuid::new_v4().to_string(),
            settings: Settings::defaults(downloads),
            profiles: vec![],
            peers: vec![],
            flow_peers: vec![],
        };
        save(directory, &data)?;
        Ok(data)
    }
}
pub fn sanitize_flow(data: &mut Persisted) {
    let mut peers = std::collections::HashSet::new();
    let mut edges = std::collections::HashSet::new();
    let known: std::collections::HashSet<String> =
        data.peers.iter().map(|p| p.peer.id.clone()).collect();
    data.flow_peers.retain(|f| {
        known.contains(&f.peer)
            && crate::flow::FlowConfig::new(f.edge, f.mode, f.delay_ms, f.block_while_button, f.clipboard).is_ok()
            && peers.insert(f.peer.clone())
            && edges.insert(f.edge)
    });
}
pub fn write_document<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let serialized =
        serde_json::to_vec_pretty(data).map_err(|e| format!("Cannot serialize document: {e}"))?;
    if serialized.len() as u64 > MAX_DOCUMENT {
        return Err("Document exceeds the 8 MiB limit.".into());
    }
    let parent = path.parent().ok_or("Invalid destination folder.")?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create folder: {e}"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("Cannot create temporary file: {e}"))?;
    file.write_all(&serialized)
        .map_err(|e| format!("Cannot write document: {e}"))?;
    file.flush()
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| format!("Cannot save document: {e}"))?;
    file.persist(path)
        .map_err(|e| format!("Cannot replace document: {}", e.error))?;
    Ok(())
}
pub fn read_document<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = fs::File::open(path).map_err(|e| format!("Cannot read document: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("Cannot read document: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_DOCUMENT {
        return Err("Document exceeds the 8 MiB limit or is not a regular file.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read document: {e}"))?;
    if bytes.len() as u64 > MAX_DOCUMENT {
        return Err("Document exceeds the 8 MiB limit.".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid TrapLink document: {e}"))
}
pub fn save(directory: &Path, data: &Persisted) -> Result<(), String> {
    write_document(&directory.join("settings.json"), data)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupDocument {
    pub format: String,
    pub version: u32,
    pub backup: Backup,
    pub settings: Settings,
    pub profiles: Vec<Profile>,
}
impl BackupDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.format != "traplink-backup" || self.version != SCHEMA_VERSION {
            return Err("Unsupported TrapLink backup format/version.".into());
        }
        Uuid::parse_str(&self.backup.id).map_err(|_| "Invalid backup ID.")?;
        if self.settings.device_name.trim().is_empty()
            || self.settings.device_name.chars().count() > 80
        {
            return Err("Backup device name must contain 1–80 characters.".into());
        }
        if !["dark", "light", "system"].contains(&self.settings.theme.as_str()) {
            return Err("Backup contains an unknown theme.".into());
        }
        if self.profiles.len() > 2000 {
            return Err("Backup contains too many profiles.".into());
        }
        let mut seen = std::collections::HashSet::new();
        for profile in &self.profiles {
            profile.validate()?;
            if !seen.insert(&profile.id) {
                return Err("Backup contains duplicate profile IDs.".into());
            }
        }
        Ok(())
    }
}
pub fn backup_path(directory: &Path, id: &str) -> Result<PathBuf, String> {
    let id = Uuid::parse_str(id).map_err(|_| "Invalid backup ID.")?;
    Ok(directory.join("backups").join(format!("{id}.json")))
}
pub fn backups(directory: &Path) -> Vec<Backup> {
    let Ok(entries) = fs::read_dir(directory.join("backups")) else {
        return vec![];
    };
    let mut result: Vec<Backup> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            if entry.path().extension().is_none_or(|ext| ext != "json") {
                return None;
            }
            read_document::<BackupDocument>(&entry.path())
                .ok()
                .filter(|doc| doc.validate().is_ok())
                .map(|doc| doc.backup)
        })
        .collect();
    result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_no_secrets_in_backup() {
        let directory = tempfile::tempdir().unwrap();
        let original = load(directory.path(), directory.path().display().to_string()).unwrap();
        let loaded = load(directory.path(), "ignored".into()).unwrap();
        assert_eq!(original.device_id, loaded.device_id);
        let doc = BackupDocument {
            format: "traplink-backup".into(),
            version: 1,
            backup: Backup {
                id: Uuid::new_v4().to_string(),
                name: "Test".into(),
                created_at: now(),
                profile_count: 0,
            },
            settings: loaded.settings,
            profiles: vec![],
        };
        let encoded = serde_json::to_string(&doc).unwrap();
        assert!(!encoded.contains("privateKey"));
        assert!(!encoded.contains("peers"));
        assert!(!encoded.contains("deviceToken"));
        assert!(backup_path(directory.path(), "../../escape").is_err());
    }
    #[test]
    fn start_on_login_is_allowed_but_remote_http_is_not() {
        let mut settings = Settings::defaults(std::env::temp_dir().display().to_string());
        assert!(
            !settings.start_on_login,
            "start on login must default to off"
        );
        settings.start_on_login = true;
        assert!(settings.validate().is_ok());
        settings.auth_server_url = "http://example.com".into();
        assert!(settings.validate().is_err());
    }
    #[test]
    fn older_settings_files_load_with_no_flow_armed() {
        let directory = tempfile::tempdir().unwrap();
        let downloads = serde_json::to_string(&directory.path().display().to_string()).unwrap();
        let json = format!(r#"{{"schemaVersion":1,"deviceId":"{}","settings":{{"deviceName":"PC","authServerUrl":"https://traplink.traphub.xyz","clipboardEnabled":false,"fileSharingEnabled":false,"inputEnabled":false,"startOnLogin":false,"theme":"dark","downloadDirectory":{downloads},"port":48372}},"profiles":[],"peers":[]}}"#, Uuid::new_v4());
        fs::write(directory.path().join("settings.json"), json).unwrap();
        let data = load(directory.path(), "ignored".into()).unwrap();
        assert!(data.flow_peers.is_empty());
        assert!(!data.settings.input_enabled);
    }
    #[test]
    fn invalid_saved_flow_entries_are_dropped_on_load() {
        let directory = tempfile::tempdir().unwrap();
        let mut data = load(directory.path(), directory.path().display().to_string()).unwrap();
        let entry = |peer: &str, edge, delay_ms| SavedFlow {
            peer: peer.into(),
            edge,
            mode: FlowMode::Edge,
            delay_ms,
            block_while_button: true,
            clipboard: false,
        };
        for id in ["a", "b", "c"] {
            data.peers.push(TrustedPeer {
                peer: Peer {
                    flow_clipboard: false,
                    id: id.into(),
                    name: id.into(),
                    address: String::new(),
                    platform: "windows".into(),
                    fingerprint: String::new(),
                    connected: false,
                    trusted: true,
                    last_seen: None,
                    permissions: PeerPermissions::default(),
                },
                public_key: String::new(),
            });
        }
        data.flow_peers = vec![
            entry("a", FlowEdge::Left, 250),
            entry("a", FlowEdge::Top, 250),
            entry("b", FlowEdge::Left, 250),
            entry("c", FlowEdge::Right, u64::MAX),
            entry("unknown", FlowEdge::Bottom, 250),
        ];
        save(directory.path(), &data).unwrap();
        let loaded = load(directory.path(), "ignored".into()).unwrap();
        assert_eq!(loaded.flow_peers, vec![entry("a", FlowEdge::Left, 250)]);
    }
    #[test]
    fn older_settings_files_keep_start_on_login_off() {
        let downloads = serde_json::to_string(&std::env::temp_dir().display().to_string()).unwrap();
        let json = format!(r#"{{"deviceName":"PC","authServerUrl":"https://traplink.traphub.xyz","clipboardEnabled":false,"fileSharingEnabled":false,"inputEnabled":false,"startOnLogin":false,"theme":"dark","downloadDirectory":{downloads},"port":48372}}"#);
        let settings: Settings = serde_json::from_str(&json).unwrap();
        assert!(!settings.start_on_login);
        assert!(!settings.remap_enabled);
        assert!(settings.validate().is_ok());
    }
}
