use crate::{
    identity::{save_credential, Credential},
    model::{validate_server_url, Account},
    state::Core,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

/// Shown when the server answers 401 to this computer's credential.
pub const SIGNED_OUT_MESSAGE: &str =
    "This computer's sign-in expired or was revoked. Sign in again.";
const MAX_ERROR_BODY: usize = 16 * 1024;

pub struct PendingLogin {
    id: uuid::Uuid,
    code: String,
    pub server: String,
    expires: Instant,
    next_poll: Instant,
    interval: u64,
    pub verification_uri: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    pub verification_uri: String,
    pub user_code: String,
    pub expires_in: u64,
    pub interval: u64,
}
#[derive(Serialize)]
pub struct LoginPoll {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
}
#[derive(Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: u64,
}

pub async fn start(core: Arc<Core>) -> Result<LoginStart, String> {
    let _gate = core.auth_gate.lock().await;
    let epoch = core.auth_epoch.load(Ordering::SeqCst);
    let server = core
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?
        .settings
        .auth_server_url
        .trim_end_matches('/')
        .to_owned();
    let trusted = validate_server_url(&server)?;
    let response = core
        .client
        .post(format!("{server}/api/auth/device/code"))
        .json(&serde_json::json!({"client_id":"traplink-desktop", "scope":"devices backups"}))
        .send()
        .await
        .map_err(|e| format!("Cannot reach authentication server: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Authentication server rejected the login request (HTTP {}).",
            response.status().as_u16()
        ));
    }
    let code: DeviceCode = response
        .json()
        .await
        .map_err(|_| "Invalid device-code response from server.")?;
    if code.device_code.len() > 2048
        || code.user_code.len() > 128
        || code.expires_in > 3600
        || code.expires_in == 0
    {
        return Err("Invalid device-code response limits.".into());
    }
    let open_url = code
        .verification_uri_complete
        .unwrap_or(code.verification_uri);
    let verification = validate_server_url(open_url.split('?').next().unwrap_or(&open_url))?;
    let actual = url::Url::parse(&open_url).map_err(|_| "Invalid verification URL.")?;
    if verification.origin() != trusted.origin() || actual.origin() != trusted.origin() {
        return Err("Authentication server returned a verification URL on another origin.".into());
    }
    let interval = code.interval.clamp(3, 30);
    let mut pending = core
        .pending_login
        .lock()
        .map_err(|_| "Account lock unavailable.")?;
    if core.auth_epoch.load(Ordering::SeqCst) != epoch {
        return Err("Login was cancelled.".into());
    }
    *pending = Some(PendingLogin {
        id: uuid::Uuid::new_v4(),
        code: code.device_code,
        server,
        expires: Instant::now() + Duration::from_secs(code.expires_in),
        next_poll: Instant::now() + Duration::from_secs(interval),
        interval,
        verification_uri: open_url.clone(),
    });
    Ok(LoginStart {
        verification_uri: open_url,
        user_code: code.user_code,
        expires_in: code.expires_in,
        interval,
    })
}

pub async fn poll(core: Arc<Core>) -> Result<LoginPoll, String> {
    let _gate = core.auth_gate.lock().await;
    let (id, server, code) = {
        let mut pending = core
            .pending_login
            .lock()
            .map_err(|_| "Account lock unavailable.")?;
        let Some(login) = pending.as_mut() else {
            return Err("Start a login request first.".into());
        };
        if Instant::now() >= login.expires {
            *pending = None;
            return Ok(LoginPoll {
                status: "expired".into(),
                account: None,
            });
        }
        if Instant::now() < login.next_poll {
            return Ok(LoginPoll {
                status: "pending".into(),
                account: None,
            });
        }
        login.next_poll = Instant::now() + Duration::from_secs(login.interval);
        (login.id, login.server.clone(), login.code.clone())
    };
    let sent = core.client.post(format!("{server}/api/auth/device/token")).json(&serde_json::json!({"grant_type":"urn:ietf:params:oauth:grant-type:device_code","device_code":code,"client_id":"traplink-desktop"})).send().await;
    if !pending_is_current(&core, id)? {
        return Ok(cancelled_poll());
    }
    // A dropped connection, a timeout or a proxy error page does not end the login: the
    // device code stays valid until it expires, so the next poll tries again.
    let Ok(response) = sent else {
        return Ok(pending_poll());
    };
    let status = response.status();
    let busy = status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
    let body: serde_json::Value = match response.json().await {
        Ok(body) => body,
        Err(_) if busy => serde_json::Value::Null,
        Err(_) => return Err("Invalid token response from server.".into()),
    };
    if !pending_is_current(&core, id)? {
        return Ok(cancelled_poll());
    }
    if !status.is_success() || body.get("error").is_some() {
        let error = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            Some("slow_down")
        } else if status.is_server_error() {
            Some("authorization_pending")
        } else {
            body["error"].as_str()
        };
        return match error {
            Some("authorization_pending") => Ok(LoginPoll {
                status: "pending".into(),
                account: None,
            }),
            Some("slow_down") => {
                if let Some(login) = core
                    .pending_login
                    .lock()
                    .map_err(|_| "Account lock unavailable.")?
                    .as_mut()
                {
                    login.interval = (login.interval + 5).min(60);
                    login.next_poll = Instant::now() + Duration::from_secs(login.interval);
                }
                Ok(LoginPoll {
                    status: "pending".into(),
                    account: None,
                })
            }
            Some("expired_token" | "invalid_grant") => {
                *core
                    .pending_login
                    .lock()
                    .map_err(|_| "Account lock unavailable.")? = None;
                Ok(LoginPoll {
                    status: "expired".into(),
                    account: None,
                })
            }
            Some("access_denied") => {
                *core
                    .pending_login
                    .lock()
                    .map_err(|_| "Account lock unavailable.")? = None;
                Ok(LoginPoll {
                    status: "denied".into(),
                    account: None,
                })
            }
            _ => Err(format!("Authentication failed (HTTP {}).", status.as_u16())),
        };
    }
    let token = body["access_token"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .ok_or("Authentication server returned no access token.")?;
    let (installation_id, name) = {
        let data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
        (data.device_id.clone(), data.settings.device_name.clone())
    };
    // Older versions stored names with control characters, which the server rejects.
    let name = crate::model::sanitize_device_name(&name).unwrap_or_else(|| "My computer".into());
    let mut registration = serde_json::json!({"installationId":installation_id,"name":name,"platform":crate::model::platform(),"appVersion":env!("CARGO_PKG_VERSION")});
    if let Some(identity) = &core.identity {
        registration["publicKey"] = STANDARD.encode(&identity.public).into();
    }
    let response = core
        .client
        .post(format!("{server}/api/v1/devices/register"))
        .bearer_auth(token)
        .json(&registration)
        .send()
        .await
        .map_err(|e| format!("Device registration failed: {e}"))?;
    if !response.status().is_success() {
        let code = response.status().as_u16();
        return Err(match server_message(response).await {
            Some(message) => format!("Device registration failed: {message} (HTTP {code})."),
            None => format!("Device registration failed (HTTP {code}). Start login again."),
        });
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid registration response.")?;
    let token = body["deviceToken"]
        .as_str()
        .filter(|t| !t.is_empty() && t.len() <= 8192)
        .ok_or("Missing device token.")?
        .to_owned();
    let account: Account =
        serde_json::from_value(body["user"].clone()).map_err(|_| "Missing account details.")?;
    let credential = Credential {
        server,
        token,
        device_id: body["device"]["id"]
            .as_str()
            .ok_or("Missing registered device ID.")?
            .into(),
        account: account.clone(),
    };
    let committed = {
        let mut pending = core
            .pending_login
            .lock()
            .map_err(|_| "Account lock unavailable.")?;
        if pending.as_ref().is_some_and(|p| p.id == id) {
            save_credential(&installation_id, &credential)?;
            *core
                .account
                .lock()
                .map_err(|_| "Account lock unavailable.")? = Some(credential.clone());
            *pending = None;
            core.clear_warning(SIGNED_OUT_MESSAGE);
            true
        } else {
            false
        }
    };
    if !committed {
        match core.client.delete(format!("{}/api/v1/devices/{}", credential.server, credential.device_id))
            .bearer_auth(&credential.token).send().await {
            Ok(response) if response.status().is_success() || response.status() == reqwest::StatusCode::UNAUTHORIZED => {},
            _ => core.warning("A cancelled login registered a device but could not revoke it. Reconnect and review the account's device list.".into()),
        }
        return Ok(cancelled_poll());
    }
    Ok(LoginPoll {
        status: "authorized".into(),
        account: Some(account),
    })
}

pub async fn logout(core: Arc<Core>) -> Result<(), String> {
    cancel(core.clone()).await?;
    let _gate = core.auth_gate.lock().await;
    let credential = core
        .account
        .lock()
        .map_err(|_| "Account lock unavailable.")?
        .clone();
    if let Some(credential) = &credential {
        let response = core
            .client
            .delete(format!(
                "{}/api/v1/devices/{}",
                credential.server, credential.device_id
            ))
            .bearer_auth(&credential.token)
            .send()
            .await
            .map_err(|_| "Cannot revoke the device while offline. Reconnect and try again.")?;
        if !response.status().is_success()
            && response.status() != reqwest::StatusCode::UNAUTHORIZED
            && response.status() != reqwest::StatusCode::NOT_FOUND
        {
            return Err(format!(
                "Device revocation failed (HTTP {}).",
                response.status().as_u16()
            ));
        }
    }
    let installation = core
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?
        .device_id
        .clone();
    crate::identity::delete_credential(&installation)?;
    *core
        .account
        .lock()
        .map_err(|_| "Account lock unavailable.")? = None;
    *core
        .pending_login
        .lock()
        .map_err(|_| "Account lock unavailable.")? = None;
    core.clear_warning(SIGNED_OUT_MESSAGE);
    Ok(())
}

/// Called with `auth_gate` held after the server answered an exact 401 to `token`. Forgets
/// the credential only if it is still the one in use, so a newer sign-in is never removed.
pub(crate) fn forget_rejected_credential(core: &Core, token: &str) {
    let Ok(installation) = core.data.lock().map(|data| data.device_id.clone()) else {
        return;
    };
    let Ok(mut account) = core.account.lock() else {
        return;
    };
    if !account
        .as_ref()
        .is_some_and(|credential| credential.token == token)
    {
        return;
    }
    if let Err(error) = crate::identity::delete_credential(&installation) {
        core.warning(format!(
            "Cannot remove the expired sign-in from the system keychain: {error}"
        ));
    }
    *account = None;
    drop(account);
    core.warning(SIGNED_OUT_MESSAGE.into());
}

/// Reads at most `MAX_ERROR_BODY` bytes of an error response and returns its `message`
/// field without control characters, capped at 200 characters.
async fn server_message(response: reqwest::Response) -> Option<String> {
    clean_server_message(&read_limited(response, MAX_ERROR_BODY).await?)
}

/// Reads a response body, or returns `None` when it is larger than `max` bytes or breaks off.
async fn read_limited(mut response: reqwest::Response, max: usize) -> Option<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > max.saturating_sub(bytes.len()) {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some(bytes)
}

fn clean_server_message(body: &[u8]) -> Option<String> {
    let body: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message: String = body["message"]
        .as_str()?
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let message: String = message.trim().chars().take(200).collect();
    (!message.is_empty()).then_some(message)
}

const MAX_DEVICE_LIST_BODY: usize = 512 * 1024;
const MAX_DEVICES: usize = 256;
const OFFLINE_MESSAGE: &str =
    "Cannot reach the account server. Check your connection and try again.";
const INVALID_DEVICE_LIST: &str = "The account server sent an invalid device list.";

/// A computer registered to the signed-in account, as shown on the Account page.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub app_version: String,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub current: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceList {
    devices: Vec<RemoteDevice>,
    current_device_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteDevice {
    id: String,
    name: String,
    platform: String,
    #[serde(default)]
    app_version: String,
    created_at: String,
    last_seen_at: String,
    expires_at: String,
    revoked_at: Option<String>,
}

fn valid_date(value: &str) -> Result<String, String> {
    chrono::DateTime::parse_from_rfc3339(value).map_err(|_| INVALID_DEVICE_LIST)?;
    Ok(value.to_owned())
}

/// Validates the server's device list. This computer comes first, then active devices, then
/// revoked devices from the most recently revoked. The server keeps revoked rows forever, so a
/// long list is not an error: at most `MAX_DEVICES` are kept and the oldest revoked ones go
/// first. The body size limit already bounds how many entries are parsed.
pub fn parse_device_list(body: &[u8]) -> Result<Vec<AccountDevice>, String> {
    if body.len() > MAX_DEVICE_LIST_BODY {
        return Err(INVALID_DEVICE_LIST.into());
    }
    let list: DeviceList = serde_json::from_slice(body).map_err(|_| INVALID_DEVICE_LIST)?;
    let current = uuid::Uuid::parse_str(&list.current_device_id).ok();
    let mut devices = list
        .devices
        .into_iter()
        .map(|device| {
            let id = uuid::Uuid::parse_str(&device.id).map_err(|_| INVALID_DEVICE_LIST)?;
            let platform = match device.platform.as_str() {
                "windows" | "macos" => device.platform,
                _ => "unknown".into(),
            };
            Ok(AccountDevice {
                id: id.to_string(),
                name: crate::model::sanitize_device_name(&device.name)
                    .unwrap_or_else(|| "Unnamed computer".into()),
                platform,
                app_version: device
                    .app_version
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(32)
                    .collect(),
                created_at: valid_date(&device.created_at)?,
                last_seen_at: valid_date(&device.last_seen_at)?,
                expires_at: valid_date(&device.expires_at)?,
                revoked_at: device.revoked_at.as_deref().map(valid_date).transpose()?,
                current: current == Some(id),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    devices.sort_by_cached_key(|device| {
        let revoked_at = device
            .revoked_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
        (
            !device.current,
            revoked_at.is_some(),
            std::cmp::Reverse(revoked_at),
        )
    });
    devices.truncate(MAX_DEVICES);
    Ok(devices)
}

fn signed_in_credential(core: &Core) -> Result<Credential, String> {
    core.account
        .lock()
        .map_err(|_| "Account lock unavailable.")?
        .clone()
        .ok_or_else(|| "Sign in to manage this account's devices.".into())
}

fn reject_unauthorized(
    core: &Core,
    credential: &Credential,
    status: reqwest::StatusCode,
) -> Result<(), String> {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        forget_rejected_credential(core, &credential.token);
        return Err(SIGNED_OUT_MESSAGE.into());
    }
    Ok(())
}

/// Lists the account's devices. Opening the list also tells the server this computer is in
/// use, so a revoked or expired credential is noticed here instead of at launch.
pub async fn list_devices(core: Arc<Core>) -> Result<Vec<AccountDevice>, String> {
    let _gate = core.auth_gate.lock().await;
    let credential = signed_in_credential(&core)?;
    let heartbeat = core
        .client
        .post(format!("{}/api/v1/devices/heartbeat", credential.server))
        .bearer_auth(&credential.token)
        .send()
        .await
        .map_err(|_| OFFLINE_MESSAGE)?;
    reject_unauthorized(&core, &credential, heartbeat.status())?;
    let response = core
        .client
        .get(format!("{}/api/v1/devices", credential.server))
        .bearer_auth(&credential.token)
        .send()
        .await
        .map_err(|_| OFFLINE_MESSAGE)?;
    reject_unauthorized(&core, &credential, response.status())?;
    if !response.status().is_success() {
        return Err(format!(
            "Cannot load this account's devices (HTTP {}).",
            response.status().as_u16()
        ));
    }
    let body = read_limited(response, MAX_DEVICE_LIST_BODY)
        .await
        .ok_or(INVALID_DEVICE_LIST)?;
    parse_device_list(&body)
}

/// Revokes one of the account's devices. Revoking this computer signs it out and returns
/// `true`.
pub async fn revoke_device(core: Arc<Core>, id: &str) -> Result<bool, String> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| "Invalid device ID.")?;
    let is_current = |credential: &Credential| {
        uuid::Uuid::parse_str(&credential.device_id).is_ok_and(|current| current == id)
    };
    if is_current(&signed_in_credential(&core)?) {
        logout(core).await?;
        return Ok(true);
    }
    let _gate = core.auth_gate.lock().await;
    let credential = signed_in_credential(&core)?;
    if is_current(&credential) {
        return Err("Refresh the device list and try again.".into());
    }
    let response = core
        .client
        .delete(format!("{}/api/v1/devices/{id}", credential.server))
        .bearer_auth(&credential.token)
        .send()
        .await
        .map_err(|_| OFFLINE_MESSAGE)?;
    reject_unauthorized(&core, &credential, response.status())?;
    match response.status() {
        status if status.is_success() => Ok(false),
        reqwest::StatusCode::NOT_FOUND => {
            Err("This device is no longer on your account. Refresh the list.".into())
        }
        status => Err(format!(
            "Cannot revoke this device (HTTP {}).",
            status.as_u16()
        )),
    }
}

pub async fn cancel(core: Arc<Core>) -> Result<(), String> {
    let mut pending = core
        .pending_login
        .lock()
        .map_err(|_| "Account lock unavailable.")?;
    core.auth_epoch.fetch_add(1, Ordering::SeqCst);
    *pending = None;
    Ok(())
}

fn pending_is_current(core: &Core, id: uuid::Uuid) -> Result<bool, String> {
    Ok(core
        .pending_login
        .lock()
        .map_err(|_| "Account lock unavailable.")?
        .as_ref()
        .is_some_and(|p| p.id == id))
}
fn pending_poll() -> LoginPoll {
    LoginPoll {
        status: "pending".into(),
        account: None,
    }
}
fn cancelled_poll() -> LoginPoll {
    LoginPoll {
        status: "denied".into(),
        account: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancel_is_immediate_while_poll_owns_auth_gate() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let id = uuid::Uuid::new_v4();
        *core.pending_login.lock().unwrap() = Some(PendingLogin {
            id,
            code: "test-only-code".into(),
            server: "https://example.com".into(),
            expires: Instant::now() + Duration::from_secs(120),
            next_poll: Instant::now(),
            interval: 5,
            verification_uri: "https://example.com/device".into(),
        });
        let _in_flight_poll = core.auth_gate.lock().await;
        tokio::time::timeout(Duration::from_millis(100), cancel(core.clone()))
            .await
            .unwrap()
            .unwrap();
        assert!(!pending_is_current(&core, id).unwrap());
        assert_eq!(core.auth_epoch.load(Ordering::SeqCst), 1);
        assert!(core.account.lock().unwrap().is_none());
    }

    fn pending_for(core: &Core, server: &str) {
        *core.pending_login.lock().unwrap() = Some(PendingLogin {
            id: uuid::Uuid::new_v4(),
            code: "test-only-code".into(),
            server: server.into(),
            expires: Instant::now() + Duration::from_secs(60),
            next_poll: Instant::now(),
            interval: 5,
            verification_uri: format!("{server}/device"),
        });
    }
    async fn poll_once(core: &Arc<Core>) -> Result<LoginPoll, String> {
        tokio::time::timeout(Duration::from_secs(10), poll(core.clone()))
            .await
            .expect("poll must not hang")
    }

    #[tokio::test]
    async fn transient_token_poll_failures_keep_the_login_pending() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let stub = test_http::serve(vec![test_http::response(
            "502 Bad Gateway",
            "<html>bad gateway</html>",
        )])
        .await;
        pending_for(&core, &stub.base);
        let result = poll_once(&core).await.expect("a 502 is transient");
        assert_eq!(result.status, "pending");
        assert!(core.pending_login.lock().unwrap().is_some());

        let refused = test_http::unused_base().await;
        pending_for(&core, &refused);
        let result = poll_once(&core)
            .await
            .expect("a refused connection is transient");
        assert_eq!(result.status, "pending");
        assert!(core.pending_login.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn rate_limited_token_poll_backs_off() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let stub = test_http::serve(vec![test_http::response(
            "429 Too Many Requests",
            r#"{"message":"Slow down."}"#,
        )])
        .await;
        pending_for(&core, &stub.base);
        assert_eq!(poll_once(&core).await.unwrap().status, "pending");
        assert_eq!(
            core.pending_login
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .interval,
            10
        );
    }

    #[tokio::test]
    async fn denied_login_is_still_final() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let stub = test_http::serve(vec![test_http::response(
            "400 Bad Request",
            r#"{"error":"access_denied"}"#,
        )])
        .await;
        pending_for(&core, &stub.base);
        assert_eq!(poll_once(&core).await.unwrap().status, "denied");
        assert!(core.pending_login.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn registration_failure_shows_the_server_message_and_sends_a_clean_name() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        core.data.lock().unwrap().settings.device_name = "Work\tPC".into();
        let stub = test_http::serve(vec![
            test_http::response("200 OK", r#"{"access_token":"test-only-access-token"}"#),
            test_http::response(
                "409 Conflict",
                "{\"error\":\"device_limit\",\"message\":\"Revoke an unused device\\u0007 before adding another.\"}",
            ),
        ])
        .await;
        pending_for(&core, &stub.base);
        let error = poll_once(&core).await.err().expect("registration fails");
        assert!(
            error.contains("Revoke an unused device before adding another."),
            "{error}"
        );
        assert!(error.contains("409"), "{error}");
        let requests = stub.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("POST /api/v1/devices/register "));
        assert!(
            requests[1].contains(r#""name":"WorkPC""#),
            "{}",
            requests[1]
        );
    }

    const DEVICE_TOKEN: &str = "tlk_test-only-device-token";

    /// Signs the test core in and returns this computer's registered device ID.
    fn signed_in(core: &Core, server: &str) -> String {
        let device_id = uuid::Uuid::new_v4().to_string();
        *core.account.lock().unwrap() = Some(Credential {
            server: server.into(),
            token: DEVICE_TOKEN.into(),
            device_id: device_id.clone(),
            account: Account {
                name: "Test".into(),
                email: "test@example.com".into(),
            },
        });
        device_id
    }
    fn installation(core: &Core) -> String {
        core.data.lock().unwrap().device_id.clone()
    }
    fn remote_device(id: &str, name: &str, platform: &str, revoked: bool) -> serde_json::Value {
        let revoked_at = revoked.then_some("2026-03-01T00:00:00.000Z");
        serde_json::json!({
            "id": id,
            "installationId": uuid::Uuid::new_v4().to_string(),
            "name": name,
            "platform": platform,
            "appVersion": "0.1.11",
            "publicKey": null,
            "createdAt": "2026-01-02T03:04:05.000Z",
            "lastSeenAt": "2026-02-02T03:04:05.000Z",
            "expiresAt": "2026-04-02T03:04:05.000Z",
            "revokedAt": revoked_at,
        })
    }

    #[test]
    fn device_list_is_validated_and_puts_this_computer_first() {
        let current = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let revoked = uuid::Uuid::new_v4().to_string();
        let body = serde_json::json!({
            "devices": [
                remote_device(&revoked, "Old laptop", "macos", true),
                remote_device(&other, "Studio\u{7}", "linux", false),
                remote_device(&current, "Work PC", "windows", false),
            ],
            "currentDeviceId": current,
        });
        let devices = parse_device_list(body.to_string().as_bytes()).unwrap();
        let ids: Vec<&str> = devices.iter().map(|device| device.id.as_str()).collect();
        assert_eq!(ids, [current.as_str(), other.as_str(), revoked.as_str()]);
        assert!(devices[0].current && !devices[1].current && !devices[2].current);
        assert_eq!(devices[1].name, "Studio");
        assert_eq!(devices[1].platform, "unknown");
        assert_eq!(
            devices[2].revoked_at.as_deref(),
            Some("2026-03-01T00:00:00.000Z")
        );

        let bad_id = serde_json::json!({
            "devices": [remote_device("../heartbeat", "PC", "windows", false)],
            "currentDeviceId": current,
        });
        assert!(parse_device_list(bad_id.to_string().as_bytes()).is_err());
        let mut bad_date = remote_device(&other, "PC", "windows", false);
        bad_date["lastSeenAt"] = "yesterday".into();
        let bad_date = serde_json::json!({ "devices": [bad_date], "currentDeviceId": current });
        assert!(parse_device_list(bad_date.to_string().as_bytes()).is_err());
        assert!(parse_device_list(b"<html>").is_err());
    }

    #[test]
    fn long_device_lists_keep_active_devices_and_drop_the_oldest_revoked() {
        let current = uuid::Uuid::new_v4().to_string();
        let active = uuid::Uuid::new_v4().to_string();
        // Oldest revoked first, the way years of reinstalls would leave the account.
        let mut devices: Vec<_> = (0..MAX_DEVICES + 44)
            .map(|day| {
                let mut device =
                    remote_device(&uuid::Uuid::new_v4().to_string(), "Old PC", "windows", true);
                let revoked_at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
                    + chrono::Duration::days(day as i64);
                device["revokedAt"] = revoked_at.to_rfc3339().into();
                device
            })
            .collect();
        devices.push(remote_device(&active, "Laptop", "macos", false));
        devices.push(remote_device(&current, "Work PC", "windows", false));
        let newest_revoked = devices[MAX_DEVICES + 43]["id"].as_str().unwrap().to_owned();
        let oldest_revoked = devices[0]["id"].as_str().unwrap().to_owned();
        let body = serde_json::json!({ "devices": devices, "currentDeviceId": current });

        let devices = parse_device_list(body.to_string().as_bytes()).expect("long lists load");
        assert_eq!(devices.len(), MAX_DEVICES);
        assert_eq!(devices[0].id, current);
        assert_eq!(devices[1].id, active);
        assert_eq!(devices[2].id, newest_revoked);
        assert!(devices.iter().all(|device| device.id != oldest_revoked));
    }

    #[tokio::test]
    async fn listing_devices_checks_in_with_this_computers_token() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let other = uuid::Uuid::new_v4().to_string();
        let current = uuid::Uuid::new_v4().to_string();
        let list = serde_json::json!({
            "devices": [
                remote_device(&other, "Laptop", "macos", false),
                remote_device(&current, "Work PC", "windows", false),
            ],
            "currentDeviceId": current,
        });
        let stub = test_http::serve(vec![
            test_http::response("200 OK", r#"{"lastSeenAt":"2026-02-02T03:04:05.000Z"}"#),
            test_http::response("200 OK", &list.to_string()),
        ])
        .await;
        signed_in(&core, &stub.base);
        core.account.lock().unwrap().as_mut().unwrap().device_id = current.clone();
        let devices = list_devices(core.clone()).await.unwrap();
        assert_eq!(devices.len(), 2);
        assert!(devices[0].current);
        assert_eq!(devices[0].id, current);
        let requests = stub.requests.lock().unwrap();
        assert!(requests[0].starts_with("POST /api/v1/devices/heartbeat "));
        assert!(requests[1].starts_with("GET /api/v1/devices "));
        for request in requests.iter() {
            let request = request.to_ascii_lowercase();
            assert!(request.contains(&format!("authorization: bearer {DEVICE_TOKEN}")));
        }
    }

    #[tokio::test]
    async fn opening_the_device_list_forgets_a_revoked_credential() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let stub = test_http::serve(vec![test_http::response(
            "401 Unauthorized",
            r#"{"error":"unauthorized","message":"This device must sign in again."}"#,
        )])
        .await;
        signed_in(&core, &stub.base);
        let error = list_devices(core.clone()).await.err().unwrap();
        assert_eq!(error, SIGNED_OUT_MESSAGE);
        assert!(core.account.lock().unwrap().is_none());
        assert!(crate::identity::credential_was_deleted(&installation(
            &core
        )));
        assert!(core
            .warnings
            .lock()
            .unwrap()
            .iter()
            .any(|w| w == SIGNED_OUT_MESSAGE));
    }

    #[tokio::test]
    async fn device_list_errors_other_than_401_keep_the_credential() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let stub = test_http::serve(vec![
            test_http::response("200 OK", r#"{"lastSeenAt":"2026-02-02T03:04:05.000Z"}"#),
            test_http::response("403 Forbidden", r#"{"message":"Forbidden."}"#),
            test_http::response("200 OK", r#"{"lastSeenAt":"2026-02-02T03:04:05.000Z"}"#),
            test_http::response("503 Service Unavailable", "<html>down</html>"),
        ])
        .await;
        signed_in(&core, &stub.base);
        let error = list_devices(core.clone()).await.err().unwrap();
        assert!(error.contains("HTTP 403"), "{error}");
        let error = list_devices(core.clone()).await.err().unwrap();
        assert!(error.contains("HTTP 503"), "{error}");
        let offline = test_http::unused_base().await;
        core.account.lock().unwrap().as_mut().unwrap().server = offline;
        let error = list_devices(core.clone()).await.err().unwrap();
        assert!(error.contains("Cannot reach the account server"), "{error}");
        assert!(core.account.lock().unwrap().is_some());
        assert!(!crate::identity::credential_was_deleted(&installation(
            &core
        )));
    }

    #[tokio::test]
    async fn revoking_another_device_deletes_only_that_device() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let other = uuid::Uuid::new_v4().to_string();
        let stub = test_http::serve(vec![
            test_http::response("200 OK", r#"{"revoked":true}"#),
            test_http::response("404 Not Found", r#"{"message":"Device not found."}"#),
        ])
        .await;
        signed_in(&core, &stub.base);
        assert!(!revoke_device(core.clone(), &other.to_uppercase())
            .await
            .unwrap());
        let error = revoke_device(core.clone(), &other).await.err().unwrap();
        assert!(error.contains("no longer on your account"), "{error}");
        let requests = stub.requests.lock().unwrap();
        assert!(requests[0].starts_with(&format!("DELETE /api/v1/devices/{other} ")));
        assert!(core.account.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn revoking_this_computer_signs_it_out() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let stub =
            test_http::serve(vec![test_http::response("200 OK", r#"{"revoked":true}"#)]).await;
        let current = signed_in(&core, &stub.base);
        core.warning(SIGNED_OUT_MESSAGE.into());
        assert!(revoke_device(core.clone(), &current).await.unwrap());
        assert!(core.account.lock().unwrap().is_none());
        assert!(crate::identity::credential_was_deleted(&installation(
            &core
        )));
        assert!(!core
            .warnings
            .lock()
            .unwrap()
            .iter()
            .any(|w| w == SIGNED_OUT_MESSAGE));
        let requests = stub.requests.lock().unwrap();
        assert!(requests[0].starts_with(&format!("DELETE /api/v1/devices/{current} ")));
    }

    #[tokio::test]
    async fn invalid_device_ids_are_rejected_before_any_request() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let stub = test_http::serve(vec![]).await;
        signed_in(&core, &stub.base);
        for id in ["", "heartbeat", "../backups/latest", "not-a-uuid"] {
            assert_eq!(
                revoke_device(core.clone(), id).await.err().unwrap(),
                "Invalid device ID."
            );
        }
        assert!(stub.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_401_for_an_older_credential_keeps_a_newer_sign_in() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        signed_in(&core, "https://example.com");
        forget_rejected_credential(&core, "tlk_an-older-token");
        assert!(core.account.lock().unwrap().is_some());
        assert!(!crate::identity::credential_was_deleted(&installation(
            &core
        )));
    }
}

#[cfg(test)]
pub(crate) mod test_http {
    use std::sync::{Arc, Mutex};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    pub struct Stub {
        pub base: String,
        pub requests: Arc<Mutex<Vec<String>>>,
    }

    /// Answers one connection per canned response, in order, recording each raw request.
    pub async fn serve(responses: Vec<String>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(Mutex::new(vec![]));
        let seen = requests.clone();
        tokio::spawn(async move {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let request = read_request(&mut stream).await;
                seen.lock().unwrap().push(request);
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Stub { base, requests }
    }

    /// A local address with nothing listening, so connections are refused.
    pub async fn unused_base() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    pub fn response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.unwrap_or(0);
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
