mod auth;
mod background;
mod clipboard;
mod cloud_backup;
mod consent;
mod devices;
mod flow;
mod hid_thread;
mod hidpp;
mod identity;
mod input;
mod keymap;
mod link;
#[cfg(target_os = "macos")]
mod macos;
mod model;
mod network;
mod remap;
mod screen;
mod state;
mod storage;

use model::*;
use state::Core;
use std::{path::PathBuf, sync::Arc};
use tauri::{Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

type AppState<'a> = State<'a, Arc<Core>>;

#[tauri::command]
async fn select_file() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "File picker failed.".into())
}
#[tauri::command]
async fn select_folder() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "Folder picker failed.".into())
}
#[tauri::command]
async fn select_backup_destination(default_name: Option<String>) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        rfd::FileDialog::new()
            .add_filter("TrapLink backup", &["json"])
            .set_file_name(default_name.unwrap_or("traplink-backup.json".into()))
            .save_file()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "Save picker failed.".into())
}

#[tauri::command]
async fn get_snapshot(
    refresh_devices: Option<bool>,
    state: AppState<'_>,
) -> Result<Snapshot, String> {
    let core = state.inner().clone();
    let refresh_devices = refresh_devices.unwrap_or(false);
    tokio::task::spawn_blocking(move || core.snapshot_with(refresh_devices))
        .await
        .map_err(|_| "Device inventory task failed.".to_string())?
}
#[tauri::command]
fn get_capabilities() -> serde_json::Value {
    serde_json::json!({"hidInventory":true,"logitechConfiguration":true,"profileMappingApplication":remap::supported(),"lanPairing":true,"clipboardSharing":true,"fileSharing":true,"inputSharing":cfg!(any(target_os="windows",target_os="macos")),"flowEdgeSwitching":flow::supported(),"cloudBackups":true,"startOnLogin":cfg!(any(target_os="windows",target_os="macos"))})
}
#[tauri::command]
async fn upload_cloud_backup(
    id: String,
    passphrase: String,
    state: AppState<'_>,
) -> Result<cloud_backup::CloudBackupMetadata, String> {
    cloud_backup::save_cloud_backup(state.inner(), &id, passphrase).await
}
#[tauri::command]
async fn download_cloud_backup(passphrase: String, state: AppState<'_>) -> Result<Backup, String> {
    let mut doc = cloud_backup::fetch_cloud_backup(state.inner(), passphrase).await?;
    doc.backup.id = uuid::Uuid::new_v4().to_string();
    doc.backup.created_at = now();
    doc.backup.profile_count = doc.profiles.len();
    storage::write_document(
        &storage::backup_path(&state.directory, &doc.backup.id)?,
        &doc,
    )?;
    Ok(doc.backup)
}
#[tauri::command]
async fn delete_cloud_backup(state: AppState<'_>) -> Result<(), String> {
    cloud_backup::delete_cloud_backup(state.inner()).await
}
#[tauri::command]
async fn get_keyboard_settings(device_id: String) -> Result<hidpp::KeyboardSettings, String> {
    tokio::task::spawn_blocking(move || hidpp::get_keyboard_settings(&device_id))
        .await
        .map_err(|_| "Keyboard query failed.".to_string())?
}
#[tauri::command]
async fn set_fn_inversion(
    device_id: String,
    enabled: bool,
) -> Result<hidpp::KeyboardSettings, String> {
    tokio::task::spawn_blocking(move || hidpp::set_fn_inversion(&device_id, enabled))
        .await
        .map_err(|_| "Keyboard configuration failed.".to_string())?
}
fn pick<T: PartialEq + Clone>(current: &T, base: &T, next: &T) -> T {
    if next != base {
        next.clone()
    } else {
        current.clone()
    }
}
/// Applies only the fields the caller changed (`next` differs from the `base` it
/// edited) on top of the current settings, so a stale copy cannot undo changes made
/// in the background, such as an emergency stop or a consent grant. Remapping is
/// owned by its own command and always keeps the current value.
fn merge_settings(current: &Settings, base: &Settings, next: &Settings) -> Settings {
    Settings {
        device_name: pick(&current.device_name, &base.device_name, &next.device_name),
        auth_server_url: pick(
            &current.auth_server_url,
            &base.auth_server_url,
            &next.auth_server_url,
        ),
        clipboard_enabled: pick(
            &current.clipboard_enabled,
            &base.clipboard_enabled,
            &next.clipboard_enabled,
        ),
        file_sharing_enabled: pick(
            &current.file_sharing_enabled,
            &base.file_sharing_enabled,
            &next.file_sharing_enabled,
        ),
        input_enabled: pick(
            &current.input_enabled,
            &base.input_enabled,
            &next.input_enabled,
        ),
        start_on_login: pick(
            &current.start_on_login,
            &base.start_on_login,
            &next.start_on_login,
        ),
        theme: pick(&current.theme, &base.theme, &next.theme),
        download_directory: pick(
            &current.download_directory,
            &base.download_directory,
            &next.download_directory,
        ),
        port: pick(&current.port, &base.port, &next.port),
        remap_enabled: current.remap_enabled,
    }
}
/// Resolves the settings to store. Without a base the caller's copy replaces the
/// current settings, as older webviews expect.
fn resolve_settings(current: &Settings, next: &Settings, base: Option<&Settings>) -> Settings {
    match base {
        Some(base) => merge_settings(current, base, next),
        None => Settings {
            remap_enabled: current.remap_enabled,
            ..next.clone()
        },
    }
}
struct SettingsChange {
    port_changed: bool,
    server_changed: bool,
    input_revoked: bool,
    input_turned_on: bool,
}
/// Merges against the state under the lock and saves. `start_on_login` is the
/// value the OS reports after the autostart change.
fn commit_settings(
    core: &Core,
    next: &Settings,
    base: Option<&Settings>,
    start_on_login: Option<bool>,
) -> Result<SettingsChange, String> {
    let mut data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    let mut settings = resolve_settings(&data.settings, next, base);
    if let Some(start_on_login) = start_on_login {
        settings.start_on_login = start_on_login;
    }
    settings.validate()?;
    let change = SettingsChange {
        port_changed: data.settings.port != settings.port,
        server_changed: data.settings.auth_server_url != settings.auth_server_url,
        input_revoked: data.settings.input_enabled && !settings.input_enabled,
        input_turned_on: !data.settings.input_enabled && settings.input_enabled,
    };
    let mut updated = data.clone();
    if change.input_revoked {
        updated.flow_peers.clear();
    }
    updated.settings = settings;
    storage::save(&core.directory, &updated)?;
    *data = updated;
    Ok(change)
}
#[tauri::command]
async fn update_settings(
    settings: Settings,
    base: Option<Settings>,
    app: tauri::AppHandle,
    state: AppState<'_>,
) -> Result<Snapshot, String> {
    let _gate = state.auth_gate.lock().await;
    let mut settings = settings;
    let merged = {
        let data = state
            .data
            .lock()
            .map_err(|_| "Settings lock unavailable.")?;
        // A stored name that predates the shared rule stays usable until the user edits it.
        // An unchanged name from the base snapshot is fine too: the merge keeps the stored one.
        let mut previous = vec![data.settings.device_name.as_str()];
        previous.extend(base.as_ref().map(|base| base.device_name.as_str()));
        settings.device_name = model::accept_device_name(&settings.device_name, &previous)?;
        resolve_settings(&data.settings, &settings, base.as_ref())
    };
    merged.validate()?;
    let start_on_login = background::set_autostart(&app, merged.start_on_login)?;
    ensure_emergency_shortcut(&app, merged.input_enabled)?;
    let SettingsChange {
        port_changed,
        server_changed,
        input_revoked,
        input_turned_on,
    } = commit_settings(&state, &settings, base.as_ref(), Some(start_on_login))?;
    if port_changed || input_revoked {
        release_all_input(&state);
    }
    if port_changed || input_revoked || input_turned_on {
        flow::restore_saved(state.inner());
    }
    if port_changed {
        network::cancel_invite(&state);
        state.network_changed.notify_one();
    }
    if server_changed {
        *state
            .pending_login
            .lock()
            .map_err(|_| "Account lock unavailable.")? = None;
    }
    state.snapshot()
}
#[tauri::command]
fn create_profile(
    name: String,
    application: Option<String>,
    device_id: Option<String>,
    mappings: Option<Vec<KeyMapping>>,
    state: AppState<'_>,
) -> Result<Profile, String> {
    let profile = Profile {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().into(),
        application,
        device_id,
        mappings: mappings.unwrap_or_default(),
        created_at: now(),
        updated_at: now(),
    };
    profile.validate()?;
    let mut data = state
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?;
    if data.profiles.len() >= 2000 {
        return Err("Profile limit reached.".into());
    }
    let mut updated = data.clone();
    updated.profiles.push(profile.clone());
    storage::save(&state.directory, &updated)?;
    *data = updated;
    drop(data);
    reload_remap(&state);
    Ok(profile)
}
#[tauri::command]
fn update_profile(mut profile: Profile, state: AppState<'_>) -> Result<Profile, String> {
    profile.validate()?;
    let mut data = state
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?;
    let mut updated = data.clone();
    let saved = updated
        .profiles
        .iter_mut()
        .find(|p| p.id == profile.id)
        .ok_or("Profile not found.")?;
    profile.created_at = saved.created_at.clone();
    profile.updated_at = now();
    *saved = profile.clone();
    storage::save(&state.directory, &updated)?;
    *data = updated;
    drop(data);
    reload_remap(&state);
    Ok(profile)
}
#[tauri::command]
fn delete_profile(id: String, state: AppState<'_>) -> Result<(), String> {
    let mut data = state
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?;
    let mut updated = data.clone();
    updated.profiles.retain(|p| p.id != id);
    if updated.profiles.len() == data.profiles.len() {
        return Err("Profile not found.".into());
    }
    storage::save(&state.directory, &updated)?;
    *data = updated;
    drop(data);
    reload_remap(&state);
    Ok(())
}
fn reload_remap(core: &Core) {
    match remap::apply(core) {
        Ok(()) => remap::clear_resolved_warnings(core),
        Err(error) => core.warning(format!("{} {error}", remap::UPDATE_WARNING)),
    }
}
#[tauri::command]
fn dismiss_warnings(state: AppState<'_>) {
    state.dismiss_warnings();
}
#[tauri::command]
fn create_backup(name: String, state: AppState<'_>) -> Result<Backup, String> {
    if name.trim().is_empty() || name.chars().count() > 80 {
        return Err("Backup name must contain 1–80 characters.".into());
    }
    let data = state
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?;
    let backup = Backup {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().into(),
        created_at: now(),
        profile_count: data.profiles.len(),
    };
    let doc = storage::BackupDocument {
        format: "traplink-backup".into(),
        version: 1,
        backup: backup.clone(),
        settings: data.settings.clone(),
        profiles: data.profiles.clone(),
    };
    storage::write_document(&storage::backup_path(&state.directory, &backup.id)?, &doc)?;
    Ok(backup)
}
fn restore(doc: storage::BackupDocument, core: &Core) -> Result<(), String> {
    doc.validate()?;
    {
        let mut data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
        let mut updated = data.clone();
        updated.profiles = doc.profiles;
        // Backups from older versions can hold names the server rejects.
        if let Some(name) = model::sanitize_device_name(&doc.settings.device_name) {
            updated.settings.device_name = name;
        }
        updated.settings.theme = doc.settings.theme;
        updated.settings.clipboard_enabled = false;
        updated.settings.file_sharing_enabled = false;
        updated.settings.input_enabled = false;
        updated.flow_peers.clear();
        updated.settings.remap_enabled = false;
        storage::save(&core.directory, &updated)?;
        *data = updated;
    }
    release_all_input(core);
    reload_remap(core);
    Ok(())
}
/// Runs a restore off the UI thread. The app refreshes its snapshot afterwards.
async fn restore_in_background(
    core: Arc<Core>,
    read: impl FnOnce(&Core) -> Result<storage::BackupDocument, String> + Send + 'static,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || restore(read(&core)?, &core))
        .await
        .map_err(|_| "Backup restore task failed.".to_string())?
}
#[tauri::command]
async fn restore_backup(id: String, state: AppState<'_>) -> Result<(), String> {
    restore_in_background(state.inner().clone(), move |core| {
        storage::read_document(&storage::backup_path(&core.directory, &id)?)
    })
    .await
}
#[tauri::command]
fn delete_backup(id: String, state: AppState<'_>) -> Result<(), String> {
    std::fs::remove_file(storage::backup_path(&state.directory, &id)?)
        .map_err(|e| format!("Cannot remove backup: {e}"))
}
#[tauri::command]
fn export_backup(id: String, state: AppState<'_>) -> Result<String, String> {
    let doc: storage::BackupDocument =
        storage::read_document(&storage::backup_path(&state.directory, &id)?)?;
    doc.validate()?;
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}
#[tauri::command]
fn export_backup_file(id: String, path: String, state: AppState<'_>) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("Choose an absolute file path.".into());
    }
    // The webview supplies this path, so this legacy command never replaces a file.
    storage::export_backup(&state.directory, &id, &path, false)
}
/// Asks where to save the backup in the native save dialog, then writes it there. The OS
/// dialog asks before replacing a file, so a confirmed file is replaced atomically. Returns
/// `false` when the user cancels.
#[tauri::command]
async fn save_backup_file(
    id: String,
    default_name: Option<String>,
    state: AppState<'_>,
) -> Result<bool, String> {
    let directory = state.directory.clone();
    storage::read_document::<storage::BackupDocument>(&storage::backup_path(&directory, &id)?)?
        .validate()?;
    let destination = tokio::task::spawn_blocking(move || {
        rfd::FileDialog::new()
            .add_filter("TrapLink backup", &["json"])
            .set_file_name(default_name.unwrap_or("traplink-backup.json".into()))
            .save_file()
    })
    .await
    .map_err(|_| "Save picker failed.")?;
    let Some(destination) = destination else {
        return Ok(false);
    };
    tokio::task::spawn_blocking(move || {
        storage::export_backup(&directory, &id, &destination, true)
    })
    .await
    .map_err(|_| "Cannot export backup.")??;
    Ok(true)
}
#[tauri::command]
async fn import_backup_file(path: String, state: AppState<'_>) -> Result<(), String> {
    restore_in_background(state.inner().clone(), move |_| {
        storage::read_document(&PathBuf::from(path))
    })
    .await
}
#[tauri::command]
fn create_pairing_invite(state: AppState<'_>) -> Result<PairingInvite, String> {
    network::create_invite(&state)
}
#[tauri::command]
fn cancel_pairing_invite(state: AppState<'_>) {
    network::cancel_invite(&state);
}
#[tauri::command]
async fn pair_peer(address: String, code: String, state: AppState<'_>) -> Result<Peer, String> {
    network::connect(state.inner().clone(), address, Some(code), None).await
}
#[tauri::command]
async fn pair_with_invite(invitation: String, state: AppState<'_>) -> Result<Peer, String> {
    network::connect_invitation(state.inner().clone(), &invitation).await
}
#[tauri::command]
async fn connect_peer(id: String, state: AppState<'_>) -> Result<Peer, String> {
    let address = state
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?
        .peers
        .iter()
        .find(|p| p.peer.id == id)
        .ok_or("Unknown peer.")?
        .peer
        .address
        .clone();
    network::connect(state.inner().clone(), address, None, Some(id)).await
}
#[tauri::command]
fn disconnect_peer(id: String, state: AppState<'_>) -> Result<(), String> {
    network::disconnect(&state, &id)
}
#[tauri::command]
fn forget_peer(id: String, state: AppState<'_>) -> Result<(), String> {
    let mut data = state
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?;
    let mut updated = data.clone();
    updated.peers.retain(|p| p.peer.id != id);
    updated.forget_flow(&id);
    storage::save(&state.directory, &updated)?;
    *data = updated;
    drop(data);
    network::disconnect(&state, &id)?;
    Ok(())
}
#[tauri::command]
fn set_peer_permissions(
    id: String,
    clipboard: bool,
    files: bool,
    input: Option<bool>,
    state: AppState<'_>,
) -> Result<(), String> {
    let mut data = state
        .data
        .lock()
        .map_err(|_| "Settings lock unavailable.")?;
    let mut updated = data.clone();
    let peer = updated
        .peers
        .iter_mut()
        .find(|p| p.peer.id == id)
        .ok_or("Unknown peer.")?;
    let next = PeerPermissions {
        clipboard,
        files,
        input: input.unwrap_or(false),
    };
    let input_revoked = peer.peer.permissions.input && !next.input;
    peer.peer.permissions = next;
    if input_revoked {
        updated.forget_flow(&id);
    }
    storage::save(&state.directory, &updated)?;
    *data = updated;
    drop(data);
    if input_revoked {
        let active = input::stop_peer(&state, &id);
        if let Ok(link) = network::session(&state, &id) {
            link.notify_input_stop(active);
        }
    }
    Ok(())
}
fn ensure_emergency_shortcut(app: &tauri::AppHandle, input_enabled: bool) -> Result<(), String> {
    if input_enabled && !app.global_shortcut().is_registered("Control+Alt+Escape") {
        app.global_shortcut().on_shortcut("Control+Alt+Escape", |app, _, event| {
            if event.state == ShortcutState::Pressed { emergency_stop_core(&app.state::<Arc<Core>>()); }
        }).map_err(|e| format!("Cannot register the Ctrl+Alt+Escape emergency shortcut: {e}. Input sharing remains disabled."))?;
    }
    Ok(())
}
#[tauri::command]
async fn request_input_consent(
    peer_id: String,
    flow: Option<bool>,
    state: AppState<'_>,
) -> Result<(), String> {
    network::request_input_consent(state.inner().clone(), &peer_id, flow.unwrap_or(false)).await
}
#[tauri::command]
fn get_input_consent_requests(state: AppState<'_>) -> Result<Vec<consent::ConsentRequest>, String> {
    Ok(state
        .consent
        .lock()
        .map_err(|_| "Consent lock unavailable.")?
        .pending(std::time::Instant::now()))
}
#[tauri::command]
async fn respond_input_consent(
    peer_id: String,
    request: String,
    allow: bool,
    app: tauri::AppHandle,
    state: AppState<'_>,
) -> Result<(), String> {
    let _gate = state.auth_gate.lock().await;
    let open = state
        .consent
        .lock()
        .map_err(|_| "Consent lock unavailable.")?
        .answer(&peer_id, &request, allow, std::time::Instant::now());
    if open.is_none() {
        return Err("This permission request expired. Ask again from the other computer.".into());
    }
    let result = if allow {
        grant_input(&app, &state, &peer_id)
    } else {
        Err(consent::DECLINED.to_string())
    };
    network::answer_consent(&state, &peer_id, request, result.as_ref().err().cloned());
    if allow && result.is_ok() {
        flow::restore_saved(state.inner());
    }
    if allow {
        result
    } else {
        Ok(())
    }
}
/// Allows input from one peer. While mouse and keyboard was off, the stored grants
/// of other peers were hidden, so turning it on for this peer revokes them instead
/// of silently reactivating them. Returns the peers whose grant was revoked.
fn grant_peer_input(data: &mut Persisted, peer_id: &str) -> Result<Vec<String>, String> {
    if !data.peers.iter().any(|p| p.peer.id == peer_id) {
        return Err("Unknown peer.".into());
    }
    let mut revoked = Vec::new();
    if !data.settings.input_enabled {
        for peer in data.peers.iter_mut() {
            if peer.peer.id != peer_id && peer.peer.permissions.input {
                peer.peer.permissions.input = false;
                revoked.push(peer.peer.id.clone());
            }
        }
        for id in &revoked {
            data.forget_flow(id);
        }
    }
    for peer in data.peers.iter_mut().filter(|p| p.peer.id == peer_id) {
        peer.peer.permissions.input = true;
    }
    data.settings.input_enabled = true;
    Ok(revoked)
}
/// Joins names as "A", "A and B" or "A, B and C".
fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
/// Tells the user which computers lost mouse and keyboard (and their saved Flow
/// setup) because allowing `granted` turned the global switch back on.
fn revoked_input_notice(before: &Persisted, granted: &str, revoked: &[String]) -> Option<String> {
    if revoked.is_empty() {
        return None;
    }
    let name = |id: &str| {
        before
            .peers
            .iter()
            .find(|p| p.peer.id == id)
            .map_or_else(|| id.to_string(), |p| p.peer.name.clone())
    };
    let names: Vec<String> = revoked.iter().map(|id| name(id.as_str())).collect();
    let with_flow: Vec<String> = revoked
        .iter()
        .filter(|id| before.flow_peers.iter().any(|f| f.peer == **id))
        .map(|id| name(id.as_str()))
        .collect();
    let flow = if with_flow.is_empty() {
        String::new()
    } else {
        format!(
            " Any saved Flow setup with {} was removed.",
            join_names(&with_flow)
        )
    };
    Some(format!(
        "Allowing {} turned off mouse and keyboard for {}.{flow} Allow them again in Permissions if needed.",
        name(granted),
        join_names(&names)
    ))
}
fn grant_input(app: &tauri::AppHandle, core: &Core, peer_id: &str) -> Result<(), String> {
    ensure_emergency_shortcut(app, true)?;
    commit_input_grant(core, peer_id)
}
fn commit_input_grant(core: &Core, peer_id: &str) -> Result<(), String> {
    let mut data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    let mut updated = data.clone();
    let revoked = grant_peer_input(&mut updated, peer_id)?;
    let notice = revoked_input_notice(&data, peer_id, &revoked);
    storage::save(&core.directory, &updated)?;
    *data = updated;
    drop(data);
    for id in revoked {
        let active = input::stop_peer(core, &id);
        if let Ok(link) = network::session(core, &id) {
            link.notify_input_stop(active);
        }
    }
    if let Some(notice) = notice {
        core.warning(notice);
    }
    Ok(())
}
#[tauri::command]
fn open_privacy_settings(pane: String, app: tauri::AppHandle) -> Result<(), String> {
    let url = match pane.as_str() {
        "accessibility" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        "input_monitoring" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        }
        _ => return Err("Unknown settings pane.".into()),
    };
    if !cfg!(target_os = "macos") {
        return Err("These settings exist only on macOS.".into());
    }
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|e| format!("Cannot open System Settings: {e}"))
}
#[tauri::command]
async fn set_peer_control(id: String, enabled: bool, state: AppState<'_>) -> Result<(), String> {
    network::set_control(state.inner().clone(), &id, enabled).await
}
#[tauri::command]
async fn send_input(peer_id: String, event: InputEvent, state: AppState<'_>) -> Result<(), String> {
    network::send_input(state.inner().clone(), &peer_id, Some(event)).await
}
#[tauri::command]
async fn input_keepalive(peer_id: String, state: AppState<'_>) -> Result<(), String> {
    network::send_input(state.inner().clone(), &peer_id, None).await
}
#[tauri::command]
fn get_input_status(state: AppState<'_>) -> input::InputStatus {
    input::status(&state.input)
}
#[tauri::command]
async fn release_flow_control(
    peer_id: String,
    session: uuid::Uuid,
    state: AppState<'_>,
) -> Result<(), String> {
    network::release_flow_control(&state, &peer_id, session).await
}
#[tauri::command]
fn emergency_stop(state: AppState<'_>) {
    emergency_stop_core(&state);
}
pub(crate) fn emergency_stop_core(core: &Core) {
    stop_all_input(core, true);
    remap::emergency_off(core);
}
pub(crate) fn release_on_exit(core: &Arc<Core>) {
    stop_all_input(core, false);
    remap::release_held();
    network::cancel_invite(core);
}
fn restore_input_on_launch(app: &tauri::AppHandle, core: &Core) {
    let enabled = core
        .data
        .lock()
        .map(|d| d.settings.input_enabled)
        .unwrap_or(false);
    if !enabled {
        return;
    }
    if let Err(error) = ensure_emergency_shortcut(app, true) {
        if let Ok(mut data) = core.data.lock() {
            let mut updated = data.clone();
            updated.settings.input_enabled = false;
            if storage::save(&core.directory, &updated).is_ok() {
                *data = updated;
            }
        }
        core.warning(error);
    }
}
fn sync_start_on_login(core: &Core, enabled: bool) {
    if let Ok(mut data) = core.data.lock() {
        if data.settings.start_on_login != enabled {
            let mut updated = data.clone();
            updated.settings.start_on_login = enabled;
            if storage::save(&core.directory, &updated).is_ok() {
                *data = updated;
            }
        }
    }
}
fn stop_all_input(core: &Core, turn_off: bool) {
    if turn_off {
        if let Ok(mut data) = core.data.lock() {
            data.settings.input_enabled = false;
            let active = input::stop_all(core);
            let _ = storage::save(&core.directory, &data);
            drop(data);
            notify_input_stopped(core, active.as_deref());
            return;
        }
    }
    release_all_input(core);
}
fn release_all_input(core: &Core) {
    let active = input::stop_all(core);
    notify_input_stopped(core, active.as_deref());
}
fn notify_input_stopped(core: &Core, active: Option<&str>) {
    if let Ok(sessions) = core.sessions.lock() {
        for (id, session) in sessions.iter() {
            session.notify_input_stop(active == Some(id.as_str()));
        }
    }
}
#[tauri::command]
async fn send_clipboard(peer_id: String, state: AppState<'_>) -> Result<(), String> {
    network::send_clipboard(state.inner().clone(), &peer_id).await
}
#[tauri::command]
async fn send_file(peer_id: String, path: String, state: AppState<'_>) -> Result<Transfer, String> {
    network::send_file(state.inner().clone(), &peer_id, &PathBuf::from(path)).await
}
#[tauri::command]
async fn start_login(
    app: tauri::AppHandle,
    state: AppState<'_>,
) -> Result<auth::LoginStart, String> {
    let login = auth::start(state.inner().clone()).await?;
    if app
        .opener()
        .open_url(login.verification_uri.clone(), None::<String>)
        .is_err()
    {
        state.warning("The default browser could not be opened. Use the login verification address shown in the account dialog.".into());
    }
    Ok(login)
}
#[tauri::command]
async fn poll_login(state: AppState<'_>) -> Result<auth::LoginPoll, String> {
    auth::poll(state.inner().clone()).await
}
#[tauri::command]
async fn cancel_login(state: AppState<'_>) -> Result<(), String> {
    auth::cancel(state.inner().clone()).await
}
#[tauri::command]
fn reopen_login(app: tauri::AppHandle, state: AppState<'_>) -> Result<(), String> {
    let uri = state
        .pending_login
        .lock()
        .map_err(|_| "Account lock unavailable.")?
        .as_ref()
        .map(|p| p.verification_uri.clone())
        .ok_or("Start a login request first.")?;
    app.opener()
        .open_url(uri, None::<String>)
        .map_err(|e| format!("Cannot open default browser: {e}"))
}
#[tauri::command]
async fn logout(state: AppState<'_>) -> Result<(), String> {
    auth::logout(state.inner().clone()).await
}
#[tauri::command]
async fn list_account_devices(state: AppState<'_>) -> Result<Vec<auth::AccountDevice>, String> {
    auth::list_devices(state.inner().clone()).await
}
/// Returns `true` when the revoked device was this computer, which is now signed out.
#[tauri::command]
async fn revoke_account_device(id: String, state: AppState<'_>) -> Result<bool, String> {
    auth::revoke_device(state.inner().clone(), &id).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !background::launched_by_autostart(&args) {
                background::show_main(app);
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![background::AUTOSTART_ARG]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            if !background::launched_by_autostart(std::env::args()) {
                background::show_main(app.handle());
            }
            let directory = app.path().app_data_dir()?;
            let downloads = app
                .path()
                .download_dir()
                .or_else(|_| app.path().document_dir())?;
            let core = match Core::new(directory.clone(), downloads.display().to_string()) {
                Ok(core) => core,
                Err(error) => {
                    // Release builds have no console, so say why TrapLink cannot start.
                    rfd::MessageDialog::new()
                        .set_level(rfd::MessageLevel::Error)
                        .set_title("TrapLink cannot start")
                        .set_description(storage::startup_failure_message(&error, &directory))
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                    return Err(std::io::Error::other(error).into());
                }
            };
            app.manage(core.clone());
            sync_start_on_login(&core, background::autostart_enabled(app.handle()));
            restore_input_on_launch(app.handle(), &core);
            background::install_tray(app.handle(), core.clone())?;
            let handle = app.handle().clone();
            consent::set_notifier(Box::new(move |request| {
                use tauri::Emitter;
                background::show_main(&handle);
                let _ = handle.emit("input-consent-request", request);
            }));
            tauri::async_runtime::spawn(network::listen(core.clone()));
            remap::init(core.clone());
            let restoring = core.clone();
            tauri::async_runtime::spawn(async move { flow::restore_saved(&restoring) });
            let input_window = app.handle().clone();
            tauri::async_runtime::spawn(input::watchdog(core, move |status| {
                use tauri::Emitter;
                let _ = input_window.emit("input-status-changed", status);
            }));
            Ok(())
        })
        .on_window_event(|window, event| {
            let Some(core) = window.try_state::<Arc<Core>>() else {
                return;
            };
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if background::should_hide_on_close(&core) {
                        api.prevent_close();
                        let _ = window.hide();
                        background::notify_hidden_once(window.app_handle(), &core);
                    } else {
                        release_on_exit(&core);
                    }
                }
                tauri::WindowEvent::Destroyed => release_on_exit(&core),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            dismiss_warnings,
            get_capabilities,
            upload_cloud_backup,
            download_cloud_backup,
            delete_cloud_backup,
            get_keyboard_settings,
            set_fn_inversion,
            hidpp::get_battery_status,
            update_settings,
            create_profile,
            update_profile,
            delete_profile,
            create_backup,
            restore_backup,
            delete_backup,
            export_backup,
            export_backup_file,
            import_backup_file,
            create_pairing_invite,
            cancel_pairing_invite,
            pair_peer,
            pair_with_invite,
            connect_peer,
            disconnect_peer,
            forget_peer,
            set_peer_permissions,
            request_input_consent,
            get_input_consent_requests,
            respond_input_consent,
            open_privacy_settings,
            set_peer_control,
            send_clipboard,
            send_file,
            start_login,
            poll_login,
            cancel_login,
            logout,
            send_input,
            input_keepalive,
            get_input_status,
            release_flow_control,
            flow::set_flow,
            flow::get_flow_status,
            flow::remember_flow_clipboard,
            flow::repair_flow,
            remap::get_remap_status,
            remap::set_remap_enabled,
            remap::list_running_apps,
            emergency_stop,
            select_file,
            select_folder,
            select_backup_destination,
            reopen_login,
            list_account_devices,
            revoke_account_device,
            save_backup_file
        ])
        .build(tauri::generate_context!())
        .expect("Cannot start TrapLink")
        .run(on_run_event);
}

#[cfg(target_os = "macos")]
fn on_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    if let tauri::RunEvent::Reopen {
        has_visible_windows: false,
        ..
    } = event
    {
        background::show_main(app);
    }
}
#[cfg(not(target_os = "macos"))]
fn on_run_event(_app: &tauri::AppHandle, _event: tauri::RunEvent) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let mut settings = Settings::defaults("C:/Downloads".into());
        settings.device_name = "A".into();
        settings
    }

    fn peer(id: &str, input: bool) -> TrustedPeer {
        TrustedPeer {
            peer: Peer {
                flow_clipboard: false,
                id: id.into(),
                name: id.into(),
                address: "127.0.0.1:48373".into(),
                platform: "windows".into(),
                fingerprint: "test".into(),
                connected: false,
                trusted: true,
                last_seen: None,
                permissions: PeerPermissions {
                    clipboard: false,
                    files: false,
                    input,
                },
            },
            public_key: String::new(),
        }
    }

    fn saved_flow(peer: &str, edge: FlowEdge) -> SavedFlow {
        SavedFlow {
            peer: peer.into(),
            edge,
            mode: FlowMode::Edge,
            delay_ms: 250,
            block_while_button: true,
            clipboard: false,
        }
    }

    #[test]
    fn merge_keeps_background_changes_the_caller_did_not_edit() {
        let mut current = settings();
        current.input_enabled = false;
        current.remap_enabled = true;
        let mut base = settings();
        base.input_enabled = true;
        let mut next = base.clone();
        next.device_name = "B".into();
        next.remap_enabled = false;
        let merged = merge_settings(&current, &base, &next);
        assert!(!merged.input_enabled);
        assert_eq!(merged.device_name, "B");
        assert!(merged.remap_enabled);

        let mut current = settings();
        current.input_enabled = true;
        current.clipboard_enabled = true;
        let base = settings();
        let merged = merge_settings(&current, &base, &base.clone());
        assert!(merged.input_enabled);
        assert!(merged.clipboard_enabled);
    }

    #[test]
    fn merge_applies_fields_the_caller_changed() {
        let current = settings();
        let base = settings();
        let mut next = settings();
        next.input_enabled = true;
        next.theme = "light".into();
        next.port = 50000;
        let merged = merge_settings(&current, &base, &next);
        assert!(merged.input_enabled);
        assert_eq!(merged.theme, "light");
        assert_eq!(merged.port, 50000);
    }

    #[test]
    fn stale_settings_save_does_not_undo_an_emergency_stop() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let base = {
            let mut data = core.data.lock().unwrap();
            data.settings.input_enabled = true;
            data.flow_peers.push(saved_flow("peer-a", FlowEdge::Right));
            data.settings.clone()
        };
        stop_all_input(&core, true);
        let mut stale = base.clone();
        stale.device_name = "Renamed".into();
        let change = commit_settings(&core, &stale, Some(&base), None).unwrap();
        assert!(!change.input_turned_on);
        let data = core.data.lock().unwrap();
        assert!(!data.settings.input_enabled);
        assert_eq!(data.settings.device_name, "Renamed");
        let saved = storage::load(&core.directory, String::new()).unwrap();
        assert!(!saved.settings.input_enabled);
        assert_eq!(saved.settings.device_name, "Renamed");
    }

    #[test]
    fn settings_without_a_base_replace_the_current_copy() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let mut next = core.data.lock().unwrap().settings.clone();
        core.data.lock().unwrap().settings.remap_enabled = true;
        next.clipboard_enabled = true;
        next.remap_enabled = false;
        commit_settings(&core, &next, None, None).unwrap();
        let data = core.data.lock().unwrap();
        assert!(data.settings.clipboard_enabled);
        assert!(data.settings.remap_enabled);
    }

    #[test]
    fn granting_input_while_off_revokes_other_masked_grants() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let mut data = core.data.lock().unwrap().clone();
        data.settings.input_enabled = false;
        data.peers = vec![peer("a", true), peer("b", true), peer("c", false)];
        data.flow_peers = vec![
            saved_flow("a", FlowEdge::Left),
            saved_flow("b", FlowEdge::Right),
        ];
        let revoked = grant_peer_input(&mut data, "b").unwrap();
        assert_eq!(revoked, vec!["a".to_string()]);
        assert!(data.settings.input_enabled);
        let input = |id: &str| {
            data.peers
                .iter()
                .find(|p| p.peer.id == id)
                .unwrap()
                .peer
                .permissions
                .input
        };
        assert!(!input("a"));
        assert!(input("b"));
        assert!(!input("c"));
        assert!(data.flow_peers.iter().all(|f| f.peer != "a"));
        assert!(data.flow_peers.iter().any(|f| f.peer == "b"));
    }

    #[test]
    fn granting_input_while_on_keeps_other_grants() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let mut data = core.data.lock().unwrap().clone();
        data.settings.input_enabled = true;
        data.peers = vec![peer("a", true), peer("b", false)];
        data.flow_peers = vec![saved_flow("a", FlowEdge::Left)];
        assert!(grant_peer_input(&mut data, "b").unwrap().is_empty());
        assert!(data.peers.iter().all(|p| p.peer.permissions.input));
        assert_eq!(data.flow_peers.len(), 1);
        assert!(grant_peer_input(&mut data, "missing").is_err());
    }

    #[test]
    fn granting_input_names_the_computers_it_turned_off() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        {
            let mut data = core.data.lock().unwrap();
            data.settings.input_enabled = false;
            data.peers = vec![peer("a", true), peer("b", false), peer("c", true)];
            data.peers[0].peer.name = "Laptop".into();
            data.flow_peers = vec![saved_flow("a", FlowEdge::Left)];
        }
        commit_input_grant(&core, "b").unwrap();
        let warnings = core.warnings.lock().unwrap().clone();
        assert_eq!(
            warnings,
            vec!["Allowing b turned off mouse and keyboard for Laptop and c. Any saved Flow setup with Laptop was removed. Allow them again in Permissions if needed.".to_string()]
        );
        let saved = storage::load(&core.directory, String::new()).unwrap();
        assert!(saved.settings.input_enabled);
        assert!(saved.flow_peers.is_empty());
    }

    #[test]
    fn granting_input_while_on_adds_no_notice() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        {
            let mut data = core.data.lock().unwrap();
            data.settings.input_enabled = true;
            data.peers = vec![peer("a", true), peer("b", false)];
        }
        commit_input_grant(&core, "b").unwrap();
        assert!(core.warnings.lock().unwrap().is_empty());
        assert_eq!(
            join_names(&["A".into(), "B".into(), "C".into()]),
            "A, B and C"
        );
    }

    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn restore_finishes_while_the_hid_thread_is_busy() {
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let settings = core.data.lock().unwrap().settings.clone();
        let doc = storage::BackupDocument {
            format: "traplink-backup".into(),
            version: 1,
            backup: Backup {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Test".into(),
                created_at: now(),
                profile_count: 0,
            },
            settings,
            profiles: vec![],
        };
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let job = std::thread::spawn(move || {
            hid_thread::run(move || hold.recv_timeout(Duration::from_secs(10)))
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !hid_thread::busy() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let restored = tokio::time::timeout(
            Duration::from_secs(2),
            restore_in_background(core.clone(), move |_| Ok(doc)),
        )
        .await;
        release.send(()).unwrap();
        let _ = job.join().unwrap();
        assert!(matches!(restored, Ok(Ok(()))), "{restored:?}");
        assert!(!core.data.lock().unwrap().settings.input_enabled);
    }
}
