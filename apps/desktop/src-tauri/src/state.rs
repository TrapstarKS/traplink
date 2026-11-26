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
    /// Problems found at launch (identity or keychain) that last for the whole run.
    pub startup_warnings: Vec<String>,
    /// Runtime warnings; transient ones are cleared when resolved or dismissed.
    pub warnings: Mutex<Vec<String>>,
    pub listening: AtomicBool,
    pub network_changed: Notify,
    pub pending_login: Mutex<Option<PendingLogin>>,
    pub auth_gate: tokio::sync::Mutex<()>,
    pub auth_epoch: AtomicU64,
    pub input: Mutex<crate::input::InputState>,
    pub clipboard: crate::clipboard::Clipboard,
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
            startup_warnings: vec![],
            warnings: Mutex::new(vec![]),
            listening: AtomicBool::new(false),
            network_changed: Notify::new(),
            pending_login: Mutex::new(None),
            auth_gate: tokio::sync::Mutex::new(()),
            auth_epoch: AtomicU64::new(0),
            input: Mutex::new(Default::default()),
            clipboard: crate::clipboard::Clipboard::in_memory(),
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
            startup_warnings: warnings,
            warnings: Mutex::new(vec![]),
            listening: AtomicBool::new(false),
            network_changed: Notify::new(),
            pending_login: Mutex::new(None),
            auth_gate: tokio::sync::Mutex::new(()),
            auth_epoch: AtomicU64::new(0),
            input: Mutex::new(Default::default()),
            clipboard: crate::clipboard::Clipboard::native(),
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
    /// Removes runtime warnings whose condition has been resolved.
    pub fn clear_warnings(&self, resolved: impl Fn(&str) -> bool) {
        if let Ok(mut warnings) = self.warnings.lock() {
            warnings.retain(|warning| !resolved(warning));
        }
    }
    /// Clears every runtime warning the user dismissed. Startup problems stay
    /// because they last until TrapLink restarts.
    pub fn dismiss_warnings(&self) {
        self.clear_warnings(|_| true);
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
        self.snapshot_with(false)
    }
    /// Builds a snapshot; `refresh_devices` bypasses the cached device list.
    pub fn snapshot_with(&self, refresh_devices: bool) -> Result<Snapshot, String> {
        let data = self
            .data
            .lock()
            .map_err(|_| "Settings lock is unavailable.")?
            .clone();
        let runtime = self
            .warnings
            .lock()
            .map_err(|_| "Warning lock is unavailable.")?
            .clone();
        let warnings_dismissible = !runtime.is_empty();
        let mut warnings = self.startup_warnings.clone();
        warnings.extend(runtime.into_iter().filter(|w| !self.startup_warnings.contains(w)));
        let devices = match crate::devices::inventory_cached(refresh_devices) {
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
            warnings_dismissible,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn stats(label: &str, mut samples: Vec<Duration>) {
        samples.sort();
        let total: Duration = samples.iter().sum();
        let mean = total / samples.len() as u32;
        let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
        println!(
            "{label}: n={} mean={:.2} ms p95={:.2} ms max={:.2} ms",
            samples.len(),
            mean.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            samples.last().unwrap().as_secs_f64() * 1000.0
        );
    }

    #[test]
    fn resolved_and_dismissed_warnings_are_removed() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        core.warning("Cannot listen on LAN port 1: in use".into());
        core.warning("Remapping could not be updated: x".into());
        core.clear_warnings(|w| w.starts_with("Cannot listen on LAN port"));
        assert_eq!(
            *core.warnings.lock().unwrap(),
            vec!["Remapping could not be updated: x".to_string()]
        );
        core.dismiss_warnings();
        assert!(core.warnings.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn lan_port_warning_clears_once_the_listener_binds() {
        let directory = tempfile::tempdir().unwrap();
        let blocker = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = blocker.local_addr().unwrap().port();
        let core = Core::for_test(directory.path().to_path_buf(), port);
        let listener = tokio::spawn(crate::network::listen(core.clone()));
        let lan_warning = |core: &Core| {
            core.warnings
                .lock()
                .unwrap()
                .iter()
                .any(|w| w.starts_with("Cannot listen on LAN port"))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !lan_warning(&core) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(lan_warning(&core));
        drop(blocker);
        core.network_changed.notify_one();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !core.listening.load(Ordering::Relaxed) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(core.listening.load(Ordering::Relaxed));
        assert!(!lan_warning(&core));
        listener.abort();
    }

    /// Measures the snapshot path on this machine's real HID devices.
    /// Run with `cargo test --release snapshot_latency_benchmark -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn snapshot_latency_benchmark() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        core.snapshot().unwrap();
        let runs = 40;
        let enumerate = (0..runs)
            .map(|_| {
                let start = Instant::now();
                crate::devices::inventory().unwrap();
                start.elapsed()
            })
            .collect();
        stats("full HID enumeration", enumerate);
        let snapshot = (0..runs)
            .map(|_| {
                let start = Instant::now();
                core.snapshot().unwrap();
                start.elapsed()
            })
            .collect();
        stats("Core::snapshot", snapshot);
        let busy = (0..5)
            .map(|_| {
                let (release, hold) = std::sync::mpsc::channel::<()>();
                let job = std::thread::spawn(move || {
                    crate::hid_thread::run(move || {
                        let _ = hold.recv_timeout(Duration::from_secs(5));
                    })
                });
                std::thread::sleep(Duration::from_millis(50));
                let releaser = std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1000));
                    let _ = release.send(());
                });
                let start = Instant::now();
                core.snapshot().unwrap();
                let elapsed = start.elapsed();
                releaser.join().unwrap();
                job.join().unwrap().unwrap();
                elapsed
            })
            .collect();
        stats("Core::snapshot behind a 1 s HID job", busy);
    }
}
