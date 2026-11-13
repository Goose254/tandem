// `unsafe` is denied in this crate. Two functions call Windows directly, where neither Tauri
// nor the `windows` crate offers a safe way, and opt in one by one with the reason:
// `settings::sys_locale_is_french` and `tray::round_corners`.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

mod calibration;
mod commands;
mod quiet;
mod settings;
mod state;
mod supervisor;
mod tray;

use tauri::{Emitter, Manager, WindowEvent};

use state::AppState;

/// Launched by Windows at login (autostart passes this flag).
const MINIMIZED_FLAG: &str = "--minimized";
/// Set this environment variable to a folder to keep the settings there, and not in the
/// usual `%APPDATA%\com.vdouay.tandem`. For trying Tandem as a new installation would see
/// it, or for testing, without touching the real settings.
const CONFIG_DIR_VARIABLE: &str = "TANDEM_CONFIG_DIR";

/// The folder the settings live in: the override when one is given, else the usual one.
fn config_dir(
    overridden: Option<std::ffi::OsString>,
    usual: std::path::PathBuf,
) -> std::path::PathBuf {
    match overridden {
        Some(dir) if !dir.is_empty() => dir.into(),
        _ => usual,
    }
}

pub fn run() {
    tauri::Builder::default()
        // Must be first: a second launch just brings the running window forward.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_main_window(app);
        }))
        // Opens the source-code link in the browser; the capability scopes it to the repository.
        .plugin(tauri_plugin_opener::init())
        // In-app updates: the UI asks the signed release feed, downloads, installs, restarts.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_FLAG]),
        ))
        .setup(|app| {
            let config_dir = config_dir(
                std::env::var_os(CONFIG_DIR_VARIABLE),
                app.path().app_config_dir()?,
            );
            app.manage(AppState::new(config_dir));
            tray::build(app.handle())?;
            supervisor::spawn(app.handle().clone());

            if let Some(popup) = app.get_webview_window("tray") {
                tray::round_corners(&popup);
            }
            let from_login = std::env::args().any(|a| a == MINIMIZED_FLAG);
            let start_hidden = from_login && app.state::<AppState>().settings().start_minimized;
            if !start_hidden && let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.set_focus();
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                let app = window.app_handle();
                // Someone who has outputs and closes the window is set up, guide or not:
                // do not bring the guide back at them next time.
                let state = app.state::<AppState>();
                if state.settings().guide_done_by_closing() {
                    state.update_settings(|s| s.guide_step = None);
                    let _ = app.emit("settings", state.settings());
                }
                if state.settings().close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    app.exit(0);
                }
            }
            WindowEvent::Focused(false) if window.label() == "tray" => {
                let _ = window.hide();
                tray::stamp_hidden(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_status,
            commands::list_devices,
            commands::set_enabled,
            commands::set_source,
            commands::add_output,
            commands::remove_output,
            commands::restore_output,
            commands::set_output_enabled,
            commands::set_output_delay,
            commands::set_output_gain,
            commands::set_clicks,
            commands::set_update_notice,
            commands::set_preferences,
            commands::get_autostart,
            commands::set_autostart,
            commands::start_calibration,
            commands::open_sound_settings,
            commands::show_main_window,
            commands::show_update_prompt,
            commands::show_guide,
            commands::hide_tray_popup,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tandem");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_settings_folder_can_be_overridden_and_an_empty_override_is_ignored() {
        let usual = PathBuf::from("usual");
        assert_eq!(config_dir(None, usual.clone()), usual);
        assert_eq!(config_dir(Some("".into()), usual.clone()), usual);
        assert_eq!(
            config_dir(Some("elsewhere".into()), usual),
            PathBuf::from("elsewhere")
        );
    }
}
