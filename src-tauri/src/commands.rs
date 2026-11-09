//! Commands the UI invokes. Every settings change is persisted and broadcast as a
//! `settings` event so the main window and the tray popup stay in step.

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::settings::{DeviceRef, OutputSetting, Settings};
use crate::state::{AppState, Devices, Snapshot};

fn commit<R>(app: &AppHandle, state: &AppState, f: impl FnOnce(&mut Settings) -> R) -> R {
    let r = state.update_settings(f);
    let _ = app.emit("settings", state.settings());
    r
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> Snapshot {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn list_devices(state: State<AppState>) -> Devices {
    state.devices.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_enabled(app: AppHandle, state: State<AppState>, enabled: bool) {
    commit(&app, &state, |s| s.enabled = enabled);
}

#[tauri::command]
pub fn set_source(app: AppHandle, state: State<AppState>, follow_default: bool, device: Option<DeviceRef>) {
    commit(&app, &state, |s| {
        s.source.follow_default = follow_default;
        if device.is_some() {
            s.source.device = device;
        }
    });
}

#[tauri::command]
pub fn add_output(app: AppHandle, state: State<AppState>, device: DeviceRef) {
    commit(&app, &state, |s| {
        if s.output_mut(&device.id).is_none() {
            s.outputs.push(OutputSetting { device, ..Default::default() });
        }
    });
}

#[tauri::command]
pub fn remove_output(app: AppHandle, state: State<AppState>, device_id: String) {
    commit(&app, &state, |s| {
        s.outputs.retain(|o| o.device.id != device_id);
        if s.sync_reference.as_deref() == Some(device_id.as_str()) {
            s.sync_reference = None;
        }
    });
}

/// Put back an output exactly as it was (undo of a removal), with its remembered delays.
#[tauri::command]
pub fn restore_output(app: AppHandle, state: State<AppState>, output: OutputSetting) {
    commit(&app, &state, |s| {
        if s.output_mut(&output.device.id).is_none() {
            s.outputs.push(output);
        }
    });
}

#[tauri::command]
pub fn set_output_enabled(app: AppHandle, state: State<AppState>, device_id: String, enabled: bool) {
    commit(&app, &state, |s| {
        if let Some(o) = s.output_mut(&device_id) {
            o.enabled = enabled;
        }
    });
}

#[tauri::command]
pub fn set_output_delay(app: AppHandle, state: State<AppState>, device_id: String, delay_ms: u32) {
    let ms = delay_ms.clamp(tandem_engine::engine::MIN_DELAY_MS, tandem_engine::engine::MAX_DELAY_MS);
    // Apply to the engine right away so tuning by ear feels immediate.
    crate::calibration::apply_delay(&state, &device_id, ms);
    let _ = app.emit("settings", state.settings());
}

#[tauri::command]
pub fn set_output_gain(app: AppHandle, state: State<AppState>, device_id: String, gain_db: f32) {
    let db = gain_db.clamp(-40.0, 12.0);
    {
        // Engine and settings change together under the runtime lock, so the supervisor
        // never re-applies an older value in between.
        let rt = state.runtime.lock().unwrap();
        if let Some(h) = rt.engine.as_ref().and_then(|e| e.output(&device_id)) {
            h.set_gain_db(db);
        }
        state.update_settings(|s| {
            if let Some(o) = s.output_mut(&device_id) {
                o.gain_db = db;
            }
        });
    }
    let _ = app.emit("settings", state.settings());
}

#[tauri::command]
pub fn set_clicks(state: State<AppState>, on: bool) {
    let mut rt = state.runtime.lock().unwrap();
    rt.clicks = on;
    if let Some(e) = rt.engine.as_ref() {
        e.set_clicks(on);
    }
}

/// The main window found (or no longer has) an update: every window's status carries it,
/// so the tray popup can point at it while the main window is hidden.
#[tauri::command]
pub fn set_update_notice(state: State<AppState>, version: Option<String>) {
    *state.update_notice.lock().unwrap() = version;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesPatch {
    theme: Option<String>,
    locale: Option<String>,
    close_to_tray: Option<bool>,
    start_minimized: Option<bool>,
    resync_on_reconnect: Option<bool>,
    follow_source_volume: Option<bool>,
    pause_when_silent: Option<bool>,
    auto_check_updates: Option<bool>,
    /// `Some(None)` forgets the skipped version.
    #[serde(default, deserialize_with = "double_option")]
    skipped_update: Option<Option<String>>,
    /// `Some(None)` goes back to syncing against the source.
    #[serde(default, deserialize_with = "double_option")]
    sync_reference: Option<Option<String>>,
    /// `Some(None)` clears the microphone.
    #[serde(default, deserialize_with = "double_option")]
    mic: Option<Option<DeviceRef>>,
}

/// Distinguish a missing field (`None`) from an explicit `null` (`Some(None)`).
fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[tauri::command]
pub fn set_preferences(app: AppHandle, state: State<AppState>, patch: PreferencesPatch) -> Settings {
    let locale_changed = patch.locale.is_some();
    commit(&app, &state, |s| {
        if let Some(v) = patch.theme {
            s.theme = v;
        }
        if let Some(v) = patch.locale {
            s.locale = v;
        }
        if let Some(v) = patch.close_to_tray {
            s.close_to_tray = v;
        }
        if let Some(v) = patch.start_minimized {
            s.start_minimized = v;
        }
        if let Some(v) = patch.resync_on_reconnect {
            s.resync_on_reconnect = v;
        }
        if let Some(v) = patch.mic {
            s.mic = v;
        }
        if let Some(v) = patch.follow_source_volume {
            s.follow_source_volume = v;
        }
        if let Some(v) = patch.pause_when_silent {
            s.pause_when_silent = v;
        }
        if let Some(v) = patch.auto_check_updates {
            s.auto_check_updates = v;
        }
        if let Some(v) = patch.skipped_update {
            s.skipped_update = v;
        }
        if let Some(v) = patch.sync_reference {
            s.sync_reference = v;
        }
    });
    if locale_changed {
        crate::tray::refresh_menu(&app);
    }
    state.settings()
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, on: bool) -> Result<bool, String> {
    let launcher = app.autolaunch();
    let r = if on { launcher.enable() } else { launcher.disable() };
    r.map_err(|e| e.to_string())?;
    Ok(launcher.is_enabled().unwrap_or(on))
}

#[tauri::command]
pub fn start_calibration(app: AppHandle, device_id: String) {
    crate::calibration::spawn(app, device_id, false);
}

/// Open the classic Windows Sound panel, where each device's sample rate is set
/// (device > Properties > Advanced).
#[tauri::command]
pub fn open_sound_settings() -> Result<(), String> {
    std::process::Command::new("control")
        .arg("mmsys.cpl")
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn show_main_window(app: AppHandle) {
    crate::tray::show_main_window(&app);
}

/// From the tray popup: bring up the main window with its update prompt.
#[tauri::command]
pub fn show_update_prompt(app: AppHandle) {
    crate::tray::show_main_window(&app);
    let _ = app.emit("show-update", ());
}

#[tauri::command]
pub fn hide_tray_popup(app: AppHandle) {
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.hide();
        crate::tray::stamp_hidden(&app);
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
