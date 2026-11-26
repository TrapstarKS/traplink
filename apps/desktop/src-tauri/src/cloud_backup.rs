use crate::{model::validate_server_url, state::Core, storage::BackupDocument};
use argon2::{Algorithm, Argon2, Block, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{aead::AeadInPlace, KeyInit, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Write, sync::Arc};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

const MAX_PLAINTEXT: usize = 5 * 1024 * 1024;
const TAG_BYTES: usize = 16;
const MAX_CIPHERTEXT: usize = MAX_PLAINTEXT + TAG_BYTES;
const MAX_ENCODED_CIPHERTEXT: usize = MAX_CIPHERTEXT.div_ceil(3) * 4;
const MAX_ENVELOPE_JSON: usize = 8 * 1024 * 1024;
const MAX_RESPONSE_JSON: usize = MAX_ENVELOPE_JSON + 4096;
const MAX_PASSPHRASE_BYTES: usize = 1024;
const DECRYPT_ERROR: &str = "Cannot decrypt this backup. Check its passphrase and try again.";
const AAD: &[u8] = b"TrapLink cloud backup v1 | Argon2id v19 m=65536 t=3 p=1 | XChaCha20Poly1305";

static CRYPTO_GATE: Semaphore = Semaphore::const_new(1);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    algorithm: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudBackupMetadata {
    pub updated_at: String,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Deserialize)]
struct DownloadResponse {
    backup: Envelope,
}

struct CloudAccount {
    endpoint: url::Url,
    token: Zeroizing<String>,
}

fn cloud_account(core: &Core) -> Result<CloudAccount, String> {
    let guard = core
        .account
        .lock()
        .map_err(|_| "Account lock unavailable.")?;
    let account = guard
        .as_ref()
        .ok_or("Sign in before using cloud backups.")?;
    let origin = validate_server_url(&account.server)?;
    if origin.path() != "/" {
        return Err("The authentication server must use an origin without a path.".into());
    }
    if account.token.is_empty() || account.token.len() > 8192 {
        return Err("Sign in again before using cloud backups.".into());
    }
    Ok(CloudAccount {
        endpoint: origin
            .join("/api/v1/backups/latest")
            .map_err(|_| "Invalid cloud endpoint.")?,
        token: Zeroizing::new(account.token.clone()),
    })
}

fn validate_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.len() > MAX_PASSPHRASE_BYTES
        || passphrase.chars().count() < 12
        || passphrase.trim().is_empty()
    {
        return Err(
            "Use a backup passphrase of at least 12 characters and at most 1024 bytes.".into(),
        );
    }
    Ok(())
}

fn derive_key(passphrase: &str, salt: &[u8; 16]) -> Result<Zeroizing<[u8; 32]>, String> {
    let params =
        Params::new(65_536, 3, 1, Some(32)).map_err(|_| "Backup encryption is unavailable.")?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    let mut workspace = Zeroizing::new(vec![Block::default(); 65_536]);
    argon
        .hash_password_into_with_memory(passphrase.as_bytes(), salt, &mut *key, &mut *workspace)
        .map_err(|_| "Backup encryption is unavailable.")?;
    Ok(key)
}

struct BoundedBuffer {
    bytes: Zeroizing<Vec<u8>>,
    max: usize,
}
impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("Cloud backup size limit exceeded."));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialize_bounded<T: Serialize>(value: &T, max: usize) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut writer = BoundedBuffer {
        bytes: Zeroizing::new(Vec::new()),
        max,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| {
        "This backup exceeds the 5 MiB cloud limit or cannot be encoded.".to_string()
    })?;
    Ok(writer.bytes)
}

fn encrypt_document(
    document: BackupDocument,
    passphrase: Zeroizing<String>,
) -> Result<Envelope, String> {
    validate_passphrase(&passphrase)?;
    document.validate()?;
    let mut plaintext = serialize_bounded(&document, MAX_PLAINTEXT)?;
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng
        .try_fill_bytes(&mut salt)
        .map_err(|_| "Secure randomness is unavailable.")?;
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| "Secure randomness is unavailable.")?;
    let key = derive_key(&passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&*key)
        .map_err(|_| "Backup encryption is unavailable.")?;
    plaintext.reserve(TAG_BYTES);
    cipher
        .encrypt_in_place(XNonce::from_slice(&nonce), AAD, &mut *plaintext)
        .map_err(|_| "Cannot encrypt the backup.")?;
    Ok(Envelope {
        version: 1,
        algorithm: "xchacha20poly1305".into(),
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(&*plaintext),
    })
}

fn decrypt_document(
    envelope: Envelope,
    passphrase: Zeroizing<String>,
) -> Result<BackupDocument, String> {
    validate_passphrase(&passphrase)?;
    if envelope.version != 1
        || envelope.algorithm != "xchacha20poly1305"
        || envelope.salt.len() != 24
        || envelope.nonce.len() != 32
        || envelope.ciphertext.len() > MAX_ENCODED_CIPHERTEXT
        || envelope.ciphertext.len() < 24
    {
        return Err(DECRYPT_ERROR.into());
    }
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    if STANDARD
        .decode_slice(&envelope.salt, &mut salt)
        .map_err(|_| DECRYPT_ERROR)?
        != salt.len()
        || STANDARD
            .decode_slice(&envelope.nonce, &mut nonce)
            .map_err(|_| DECRYPT_ERROR)?
            != nonce.len()
    {
        return Err(DECRYPT_ERROR.into());
    }
    let mut plaintext = Zeroizing::new(
        STANDARD
            .decode(envelope.ciphertext)
            .map_err(|_| DECRYPT_ERROR)?,
    );
    if plaintext.len() > MAX_CIPHERTEXT || plaintext.len() < TAG_BYTES {
        return Err(DECRYPT_ERROR.into());
    }
    let key = derive_key(&passphrase, &salt).map_err(|_| DECRYPT_ERROR)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| DECRYPT_ERROR)?;
    cipher
        .decrypt_in_place(XNonce::from_slice(&nonce), AAD, &mut *plaintext)
        .map_err(|_| DECRYPT_ERROR)?;
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(DECRYPT_ERROR.into());
    }
    let document: BackupDocument = serde_json::from_slice(&plaintext).map_err(|_| DECRYPT_ERROR)?;
    document.validate().map_err(|_| DECRYPT_ERROR)?;
    Ok(document)
}

fn status_error(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 | 403 => "Sign in again before using cloud backups.".into(),
        404 => "No cloud backup has been saved for this account.".into(),
        413 => "The encrypted backup exceeds this server's size limit.".into(),
        429 => "Too many cloud backup requests. Wait and try again.".into(),
        _ => format!(
            "The cloud backup request failed (HTTP {}).",
            status.as_u16()
        ),
    }
}

/// An exact 401 means the server no longer accepts this computer's credential, so forget it.
/// 403, 5xx and network errors keep the credential.
fn forget_if_unauthorized(
    core: &Core,
    token: &str,
    response: &reqwest::Response,
) -> Result<(), String> {
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        crate::auth::forget_rejected_credential(core, token);
        return Err(crate::auth::SIGNED_OUT_MESSAGE.into());
    }
    Ok(())
}

async fn bounded_response(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err(status_error(response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err("The cloud response exceeds the supported size limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Cannot read the cloud backup response.")?
    {
        if chunk.len() > max.saturating_sub(bytes.len()) {
            return Err("The cloud response exceeds the supported size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn save_cloud_backup(
    core: &Arc<Core>,
    id: &str,
    passphrase: String,
) -> Result<CloudBackupMetadata, String> {
    let passphrase = Zeroizing::new(passphrase);
    validate_passphrase(&passphrase)?;
    let _account_guard = core.auth_gate.lock().await;
    let account = cloud_account(core)?;
    let path = crate::storage::backup_path(&core.directory, id)?;
    let crypto_guard = CRYPTO_GATE
        .acquire()
        .await
        .map_err(|_| "Backup encryption is unavailable.")?;
    let envelope = tokio::task::spawn_blocking(move || {
        let _crypto_guard = crypto_guard;
        let document: BackupDocument = crate::storage::read_document(&path)?;
        encrypt_document(document, passphrase)
    })
    .await
    .map_err(|_| "Cannot prepare this cloud backup.")??;
    let body = serialize_bounded(&envelope, MAX_ENVELOPE_JSON)?;
    let expected_bytes = body.len();
    let expected_hash = format!("{:x}", Sha256::digest(&*body));
    let response = core
        .client
        .put(account.endpoint)
        .bearer_auth(account.token.as_str())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_vec())
        .send()
        .await
        .map_err(|_| "Cannot reach the cloud backup server.")?;
    forget_if_unauthorized(core, &account.token, &response)?;
    let bytes = bounded_response(response, 4096).await?;
    let metadata: CloudBackupMetadata =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid cloud backup confirmation.")?;
    if metadata.bytes != expected_bytes
        || metadata.sha256 != expected_hash
        || metadata.bytes > MAX_ENVELOPE_JSON
        || metadata.sha256.len() != 64
        || !metadata
            .sha256
            .bytes()
            .all(|value| value.is_ascii_hexdigit())
        || chrono::DateTime::parse_from_rfc3339(&metadata.updated_at).is_err()
    {
        return Err("Invalid cloud backup confirmation.".into());
    }
    Ok(metadata)
}

pub async fn fetch_cloud_backup(
    core: &Arc<Core>,
    passphrase: String,
) -> Result<BackupDocument, String> {
    let passphrase = Zeroizing::new(passphrase);
    validate_passphrase(&passphrase)?;
    let _account_guard = core.auth_gate.lock().await;
    let account = cloud_account(core)?;
    let response = core
        .client
        .get(account.endpoint)
        .bearer_auth(account.token.as_str())
        .send()
        .await
        .map_err(|_| "Cannot reach the cloud backup server.")?;
    forget_if_unauthorized(core, &account.token, &response)?;
    let bytes = bounded_response(response, MAX_RESPONSE_JSON).await?;
    let download: DownloadResponse = serde_json::from_slice(&bytes).map_err(|_| DECRYPT_ERROR)?;
    let crypto_guard = CRYPTO_GATE
        .acquire()
        .await
        .map_err(|_| "Backup decryption is unavailable.")?;
    tokio::task::spawn_blocking(move || {
        let _crypto_guard = crypto_guard;
        decrypt_document(download.backup, passphrase)
    })
    .await
    .map_err(|_| DECRYPT_ERROR.to_string())?
}

pub async fn delete_cloud_backup(core: &Arc<Core>) -> Result<(), String> {
    let _account_guard = core.auth_gate.lock().await;
    let account = cloud_account(core)?;
    let response = core
        .client
        .delete(account.endpoint)
        .bearer_auth(account.token.as_str())
        .send()
        .await
        .map_err(|_| "Cannot reach the cloud backup server.")?;
    forget_if_unauthorized(core, &account.token, &response)?;
    if !response.status().is_success() {
        return Err(status_error(response.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backup, Settings};

    fn document() -> BackupDocument {
        BackupDocument {
            format: "traplink-backup".into(),
            version: 1,
            backup: Backup {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Encrypted test".into(),
                created_at: crate::model::now(),
                profile_count: 0,
            },
            settings: Settings::defaults(std::env::temp_dir().display().to_string()),
            profiles: vec![],
        }
    }
    fn passphrase() -> Zeroizing<String> {
        Zeroizing::new("test-only backup passphrase".into())
    }

    #[test]
    fn encrypted_round_trip_and_randomized_envelopes() {
        let source = document();
        let expected = serde_json::to_value(&source).unwrap();
        let first = encrypt_document(source.clone(), passphrase()).unwrap();
        let second = encrypt_document(source, passphrase()).unwrap();
        assert_ne!(first.salt, second.salt);
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.ciphertext, second.ciphertext);
        assert!(
            !String::from_utf8(STANDARD.decode(&first.ciphertext).unwrap())
                .unwrap_or_default()
                .contains("Encrypted test")
        );
        let recovered = decrypt_document(first, passphrase()).unwrap();
        assert_eq!(serde_json::to_value(recovered).unwrap(), expected);
    }

    #[test]
    fn wrong_passphrase_and_tampering_have_the_same_error() {
        let envelope = encrypt_document(document(), passphrase()).unwrap();
        assert_eq!(
            decrypt_document(
                envelope,
                Zeroizing::new("wrong test-only passphrase".into())
            )
            .err()
            .unwrap(),
            DECRYPT_ERROR
        );
        let mut envelope = encrypt_document(document(), passphrase()).unwrap();
        let mut bytes = STANDARD.decode(&envelope.ciphertext).unwrap();
        bytes[0] ^= 1;
        envelope.ciphertext = STANDARD.encode(bytes);
        assert_eq!(
            decrypt_document(envelope, passphrase()).err().unwrap(),
            DECRYPT_ERROR
        );
    }

    #[test]
    fn rejects_unknown_versions_short_passphrases_and_oversized_input() {
        assert!(validate_passphrase("too short").is_err());
        assert!(validate_passphrase(&"a".repeat(MAX_PASSPHRASE_BYTES + 1)).is_err());
        let envelope = Envelope {
            version: 2,
            algorithm: "xchacha20poly1305".into(),
            salt: "".into(),
            nonce: "".into(),
            ciphertext: "".into(),
        };
        assert_eq!(
            decrypt_document(envelope, passphrase()).err().unwrap(),
            DECRYPT_ERROR
        );
        let mut writer = BoundedBuffer {
            bytes: Zeroizing::new(vec![]),
            max: 3,
        };
        writer.write_all(b"abc").unwrap();
        assert!(writer.write_all(b"d").is_err());
        assert_eq!(&*writer.bytes, b"abc");
    }

    #[test]
    fn envelope_serialization_matches_server_checksum_format() {
        let envelope = Envelope {
            version: 1,
            algorithm: "xchacha20poly1305".into(),
            salt: "salt".into(),
            nonce: "nonce".into(),
            ciphertext: "cipher".into(),
        };
        let encoded = serialize_bounded(&envelope, 4096).unwrap();
        assert_eq!(std::str::from_utf8(&encoded).unwrap(), "{\"version\":1,\"algorithm\":\"xchacha20poly1305\",\"salt\":\"salt\",\"nonce\":\"nonce\",\"ciphertext\":\"cipher\"}");
    }

    fn signed_in(core: &Core, server: &str) {
        *core.account.lock().unwrap() = Some(crate::identity::Credential {
            server: server.into(),
            token: "tlk_test-only-device-token".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            account: crate::model::Account {
                name: "Test".into(),
                email: "test@example.com".into(),
            },
        });
    }

    #[tokio::test]
    async fn unauthorized_cloud_response_forgets_the_rejected_credential() {
        use crate::auth::test_http;
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let installation = core.data.lock().unwrap().device_id.clone();
        let stub = test_http::serve(vec![test_http::response(
            "401 Unauthorized",
            r#"{"error":"unauthorized","message":"This device must sign in again."}"#,
        )])
        .await;
        signed_in(&core, &stub.base);
        let error = delete_cloud_backup(&core).await.err().unwrap();
        assert!(error.contains("Sign in again"), "{error}");
        assert!(core.account.lock().unwrap().is_none());
        assert!(crate::identity::credential_was_deleted(&installation));
        assert!(core
            .warnings
            .lock()
            .unwrap()
            .iter()
            .any(|warning| warning.contains("expired or was revoked")));
    }

    #[tokio::test]
    async fn forbidden_or_unavailable_cloud_responses_keep_the_credential() {
        use crate::auth::test_http;
        for status in ["403 Forbidden", "503 Service Unavailable"] {
            let directory = tempfile::tempdir().unwrap();
            let core = Core::for_test(directory.path().to_path_buf(), 48372);
            let installation = core.data.lock().unwrap().device_id.clone();
            let stub = test_http::serve(vec![test_http::response(status, "{}")]).await;
            signed_in(&core, &stub.base);
            assert!(delete_cloud_backup(&core).await.is_err());
            assert!(core.account.lock().unwrap().is_some(), "{status}");
            assert!(!crate::identity::credential_was_deleted(&installation));
        }
    }
}
