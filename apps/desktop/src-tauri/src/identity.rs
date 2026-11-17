use crate::model::Account;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SERVICE: &str = "app.traplink.desktop";
pub const NOISE_PAIR: &str = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s";
pub const NOISE_RECONNECT: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

pub struct Identity {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}
pub fn identity(installation: &str) -> Result<Identity, String> {
    if !cfg!(any(target_os = "windows", target_os = "macos")) {
        return Err(
            "Secure identity storage is supported on Windows and macOS in this build.".into(),
        );
    }
    let entry = keyring::Entry::new(SERVICE, &format!("{installation}:identity"))
        .map_err(|e| format!("Cannot open OS credential store: {e}"))?;
    match entry.get_password() {
        Ok(value) => {
            let encoded: Vec<String> = serde_json::from_str(&value)
                .map_err(|_| "Invalid identity in OS credential store.")?;
            if encoded.len() != 2 {
                return Err("Invalid identity in OS credential store.".into());
            }
            let private = STANDARD
                .decode(&encoded[0])
                .map_err(|_| "Invalid private key.")?;
            let public = STANDARD
                .decode(&encoded[1])
                .map_err(|_| "Invalid public key.")?;
            if private.len() != 32 || public.len() != 32 {
                return Err("Invalid identity key length.".into());
            }
            Ok(Identity { private, public })
        }
        Err(keyring::Error::NoEntry) => {
            let key = snow::Builder::new(NOISE_RECONNECT.parse().map_err(|_| "Invalid protocol.")?)
                .generate_keypair()
                .map_err(|e| format!("Cannot generate device identity: {e}"))?;
            let value = serde_json::to_string(&vec![
                STANDARD.encode(&key.private),
                STANDARD.encode(&key.public),
            ])
            .map_err(|e| e.to_string())?;
            entry
                .set_password(&value)
                .map_err(|e| format!("Cannot save device identity in OS credential store: {e}"))?;
            Ok(Identity {
                private: key.private,
                public: key.public,
            })
        }
        Err(e) => Err(format!("Cannot access OS credential store: {e}")),
    }
}
pub fn fingerprint(public: &[u8]) -> String {
    let digest = Sha256::digest(public);
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.concat())
        .collect::<Vec<_>>()
        .join(" ")
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    pub server: String,
    pub token: String,
    pub device_id: String,
    pub account: Account,
}
pub fn credential(installation: &str) -> Result<Option<Credential>, String> {
    let entry = keyring::Entry::new(SERVICE, &format!("{installation}:account"))
        .map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(data) => serde_json::from_str(&data)
            .map(Some)
            .map_err(|_| "Invalid account entry in credential store.".into()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Cannot read account from OS credential store: {e}")),
    }
}
pub fn save_credential(installation: &str, value: &Credential) -> Result<(), String> {
    keyring::Entry::new(SERVICE, &format!("{installation}:account"))
        .map_err(|e| e.to_string())?
        .set_password(&serde_json::to_string(value).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Cannot save account in OS credential store: {e}"))
}
pub fn delete_credential(installation: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, &format!("{installation}:account"))
        .map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!(
            "Cannot remove account from OS credential store: {e}"
        )),
    }
}
