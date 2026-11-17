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
    let response = core.client.post(format!("{server}/api/auth/device/token")).json(&serde_json::json!({"grant_type":"urn:ietf:params:oauth:grant-type:device_code","device_code":code,"client_id":"traplink-desktop"})).send().await.map_err(|e| format!("Cannot poll authentication server: {e}"))?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid token response from server.")?;
    if !pending_is_current(&core, id)? {
        return Ok(cancelled_poll());
    }
    if !status.is_success() || body.get("error").is_some() {
        return match body["error"].as_str() {
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
        return Err(format!(
            "Device registration failed (HTTP {}). Start login again.",
            response.status().as_u16()
        ));
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
    Ok(())
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
}
