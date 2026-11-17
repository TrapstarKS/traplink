use crate::{
    auth::PendingLogin,
    identity::Identity,
    model::*,
    network::{Invite, Session},
    storage,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;

pub struct Core {
    pub directory: PathBuf,
    pub data: Mutex<Persisted>,
    pub identity: Option<Identity>,
    pub account: Mutex<Option<crate::identity::Credential>>,
    pub invite: Mutex<Option<Invite>>,
    pub sessions: Mutex<HashMap<String, Arc<Session>>>,
    pub transfers: Mutex<Vec<Transfer>>,
    pub warnings: Mutex<Vec<String>>,
    pub listening: AtomicBool,
    pub network_changed: Notify,
    pub pending_login: Mutex<Option<PendingLogin>>,
    pub auth_gate: tokio::sync::Mutex<()>,
    pub auth_epoch: AtomicU64,
    pub input: Mutex<crate::input::InputState>,
    pub consent: Mutex<crate::consent::ConsentState>,
    pub client: reqwest::Client,
}
impl Core {
    #[cfg(test)]
    pub fn for_test(directory: PathBuf, port: u16) -> Arc<Self> {
        let mut data = storage::load(
            &directory,
            directory.join("downloads").display().to_string(),
        )
        .unwrap();
        data.settings.port = port;
        data.settings.file_sharing_enabled = true;
        let key = snow::Builder::new(crate::identity::NOISE_RECONNECT.parse().unwrap())
            .generate_keypair()
            .unwrap();
        Arc::new(Self {
            directory,
            data: Mutex::new(data),
            identity: Some(Identity {
                private: key.private,
                public: key.public,
            }),
            account: Mutex::new(None),
            invite: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            transfers: Mutex::new(vec![]),
            warnings: Mutex::new(vec![]),
            listening: AtomicBool::new(false),
            network_changed: Notify::new(),
            pending_login: Mutex::new(None),
            auth_gate: tokio::sync::Mutex::new(()),
            auth_epoch: AtomicU64::new(0),
            input: Mutex::new(Default::default()),
            consent: Mutex::new(Default::default()),
            client: reqwest::Client::new(),
        })
    }
    pub fn new(directory: PathBuf, downloads: String) -> Result<Arc<Self>, String> {
        let data = storage::load(&directory, downloads)?;
        let mut warnings = vec![];
        let identity = match crate::identity::identity(&data.device_id) {
            Ok(id) => Some(id),
            Err(error) => {
                warnings.push(error);
                None
            }
        };
        let account = match crate::identity::credential(&data.device_id) {
            Ok(account) => account,
            Err(error) => {
                warnings.push(error);
                None
            }
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            directory,
            data: Mutex::new(data),
            identity,
            account: Mutex::new(account),
            invite: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            transfers: Mutex::new(vec![]),
            warnings: Mutex::new(warnings),
            listening: AtomicBool::new(false),
            network_changed: Notify::new(),
            pending_login: Mutex::new(None),
            auth_gate: tokio::sync::Mutex::new(()),
            auth_epoch: AtomicU64::new(0),
            input: Mutex::new(Default::default()),
            consent: Mutex::new(Default::default()),
            client,
        }))
    }
    pub fn warning(&self, message: String) {
        if let Ok(mut warnings) = self.warnings.lock() {
            if !warnings.contains(&message) {
                warnings.push(message);
            }
            if warnings.len() > 12 {
                warnings.remove(0);
            }
        }
    }
    pub fn push_transfer(&self, transfer: Transfer) {
        if let Ok(mut transfers) = self.transfers.lock() {
            transfers.insert(0, transfer);
            transfers.truncate(100);
        }
    }
    pub fn transfer_status(&self, id: &str, status: &str) {
        if let Ok(mut transfers) = self.transfers.lock() {
            if let Some(t) = transfers.iter_mut().find(|t| t.id == id) {
                t.status = status.into();
            }
        }
    }
    pub fn addresses(&self) -> Vec<String> {
        let port = self.data.lock().map(|d| d.settings.port).unwrap_or(48372);
        let mut addresses = local_ip_address::list_afinet_netifas()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, ip)| match ip {
                std::net::IpAddr::V4(ip) if ip.is_private() => Some(ip),
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
            .into_iter()
            .map(|ip| format!("{ip}:{port}"))
            .collect()
    }
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let data = self
            .data
            .lock()
            .map_err(|_| "Settings lock is unavailable.")?
            .clone();
        let mut warnings = self
            .warnings
            .lock()
            .map_err(|_| "Warning lock is unavailable.")?
            .clone();
        let devices = match crate::devices::inventory() {
            Ok(devices) => devices,
            Err(error) => {
                warnings.push(error);
                vec![]
            }
        };
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "Network lock is unavailable.")?;
        let peers = data
            .peers
            .iter()
            .map(|p| {
                let mut peer = p.peer.clone();
                peer.connected = sessions.contains_key(&peer.id);
                peer
            })
            .collect();
        drop(sessions);
        let pairing_code = self
            .invite
            .lock()
            .map_err(|_| "Pairing lock is unavailable.")?
            .as_ref()
            .filter(|i| i.expires > std::time::Instant::now())
            .map(|i| i.code.clone());
        let account = self
            .account
            .lock()
            .map_err(|_| "Account lock is unavailable.")?
            .as_ref()
            .map(|c| c.account.clone());
        Ok(Snapshot {
            device_id: data.device_id,
            device_name: data.settings.device_name.clone(),
            platform: platform(),
            version: env!("CARGO_PKG_VERSION").into(),
            settings: data.settings,
            devices,
            peers,
            profiles: data.profiles,
            backups: storage::backups(&self.directory),
            transfers: self
                .transfers
                .lock()
                .map_err(|_| "Transfer lock is unavailable.")?
                .clone(),
            listening: self.listening.load(Ordering::Relaxed),
            local_addresses: self.addresses(),
            pairing_code,
            account,
            permissions: Permissions {
                accessibility: false,
                input_monitoring: false,
            },
            warnings,
        })
    }
}
