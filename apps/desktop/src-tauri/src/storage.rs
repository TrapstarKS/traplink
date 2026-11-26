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
            && crate::flow::FlowConfig::new(
                f.edge,
                f.mode,
                f.delay_ms,
                f.block_while_button,
                f.clipboard,
            )
            .is_ok()
            && peers.insert(f.peer.clone())
            && edges.insert(f.edge)
    });
}
pub fn write_document<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    staged_document(path, data)?
        .persist(path)
        .map_err(|e| format!("Cannot replace document: {}", e.error))?;
    Ok(())
}
/// Writes `data` to a synced temporary file next to `path`, ready to be moved into place.
fn staged_document<T: Serialize>(path: &Path, data: &T) -> Result<tempfile::NamedTempFile, String> {
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
    Ok(file)
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
/// Writes a saved backup to `destination` atomically. `replace` is set only when the user
/// picked the path in the native save dialog, which already asked before replacing a file.
pub fn export_backup(
    directory: &Path,
    id: &str,
    destination: &Path,
    replace: bool,
) -> Result<(), String> {
    let doc: BackupDocument = read_document(&backup_path(directory, id)?)?;
    doc.validate()?;
    if replace {
        return write_document(destination, &doc);
    }
    staged_document(destination, &doc)?
        .persist_noclobber(destination)
        .map_err(|e| format!("Cannot export backup (choose a new filename): {}", e.error))?;
    Ok(())
}

/// The native error shown when TrapLink cannot start. When settings.json exists it is the
/// likely cause (damaged, or from a newer version) and is never changed or deleted. Otherwise
/// the data folder itself could not be created or written, so there is no file to fix.
pub fn startup_failure_message(error: &str, directory: &Path) -> String {
    let path = directory.join("settings.json");
    if path.is_file() {
        format!(
            "TrapLink could not open its settings.\n\n{error}\n\nSettings file: {}\n\nThe file has not been changed. Fix it, or move it elsewhere to start with default settings.",
            path.display()
        )
    } else {
        format!(
            "TrapLink cannot use its data folder.\n\n{error}\n\nData folder: {}\n\nMake sure this folder can be created and written to, then start TrapLink again.",
            directory.display()
        )
    }
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
    fn saved_backup(directory: &Path, name: &str) -> String {
        let data = load(directory, directory.display().to_string()).unwrap();
        let doc = BackupDocument {
            format: "traplink-backup".into(),
            version: 1,
            backup: Backup {
                id: Uuid::new_v4().to_string(),
                name: name.into(),
                created_at: now(),
                profile_count: 0,
            },
            settings: data.settings,
            profiles: vec![],
        };
        write_document(&backup_path(directory, &doc.backup.id).unwrap(), &doc).unwrap();
        doc.backup.id
    }
    #[test]
    fn export_replaces_a_file_confirmed_in_the_save_dialog() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("Work setup.json");
        let first = saved_backup(directory.path(), "First");
        export_backup(directory.path(), &first, &destination, true).unwrap();
        let second = saved_backup(directory.path(), "Second");
        export_backup(directory.path(), &second, &destination, true).unwrap();
        let exported: BackupDocument = read_document(&destination).unwrap();
        exported.validate().unwrap();
        assert_eq!(exported.backup.id, second);
        assert_eq!(exported.backup.name, "Second");
    }
    #[test]
    fn legacy_export_still_refuses_to_replace_and_keeps_the_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("existing.json");
        fs::write(&destination, b"keep me").unwrap();
        let id = saved_backup(directory.path(), "Backup");
        assert!(export_backup(directory.path(), &id, &destination, false).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"keep me");
        let leftovers = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count();
        assert_eq!(
            leftovers, 2,
            "only settings.json and the existing file remain"
        );
    }
    fn settings_json(directory: &Path, schema: u32, device_name: &str) -> String {
        let downloads = serde_json::to_string(&directory.display().to_string()).unwrap();
        let name = serde_json::to_string(device_name).unwrap();
        format!(
            r#"{{"schemaVersion":{schema},"deviceId":"{}","settings":{{"deviceName":{name},"authServerUrl":"https://traplink.traphub.xyz","clipboardEnabled":false,"fileSharingEnabled":false,"inputEnabled":false,"startOnLogin":false,"theme":"dark","downloadDirectory":{downloads},"port":48372}},"profiles":[],"peers":[]}}"#,
            Uuid::new_v4()
        )
    }
    #[test]
    fn startup_errors_name_the_settings_file_and_leave_it_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        for contents in [settings_json(directory.path(), 2, "PC"), "{".to_string()] {
            fs::write(&path, &contents).unwrap();
            let error = load(directory.path(), "ignored".into()).err().unwrap();
            let message = startup_failure_message(&error, directory.path());
            assert!(message.contains(&error));
            assert!(message.contains(&path.display().to_string()), "{message}");
            assert!(message.contains("has not been changed"));
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }
    #[test]
    fn startup_errors_without_a_settings_file_name_the_data_folder() {
        let parent = tempfile::tempdir().unwrap();
        // A file where the data folder should be makes create_dir_all fail.
        let directory = parent.path().join("TrapLink");
        fs::write(&directory, b"not a folder").unwrap();
        let error = load(&directory, "ignored".into()).err().unwrap();
        let message = startup_failure_message(&error, &directory);
        assert!(message.contains(&error));
        assert!(message.contains("data folder"), "{message}");
        assert!(
            message.contains(&directory.display().to_string()),
            "{message}"
        );
        assert!(!message.contains("settings.json"), "{message}");
        assert!(!message.contains("has not been changed"), "{message}");
    }
    #[test]
    fn stored_device_names_with_control_characters_still_load() {
        let directory = tempfile::tempdir().unwrap();
        let json = settings_json(directory.path(), 1, "Work\tPC");
        fs::write(directory.path().join("settings.json"), json).unwrap();
        let data = load(directory.path(), "ignored".into()).unwrap();
        assert_eq!(data.settings.device_name, "Work\tPC");
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
        let json = format!(
            r#"{{"schemaVersion":1,"deviceId":"{}","settings":{{"deviceName":"PC","authServerUrl":"https://traplink.traphub.xyz","clipboardEnabled":false,"fileSharingEnabled":false,"inputEnabled":false,"startOnLogin":false,"theme":"dark","downloadDirectory":{downloads},"port":48372}},"profiles":[],"peers":[]}}"#,
            Uuid::new_v4()
        );
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
        let json = format!(
            r#"{{"deviceName":"PC","authServerUrl":"https://traplink.traphub.xyz","clipboardEnabled":false,"fileSharingEnabled":false,"inputEnabled":false,"startOnLogin":false,"theme":"dark","downloadDirectory":{downloads},"port":48372}}"#
        );
        let settings: Settings = serde_json::from_str(&json).unwrap();
        assert!(!settings.start_on_login);
        assert!(!settings.remap_enabled);
        assert!(settings.validate().is_ok());
    }
}
