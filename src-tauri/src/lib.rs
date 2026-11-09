mod calibration;
mod commands;
mod settings;
mod state;
mod supervisor;
mod tray;

use tauri::{Manager, WindowEvent};

use state::AppState;

/// Launched by Windows at login (autostart passes this flag).
const MINIMIZED_FLAG: &str = "--minimized";

pub fn run() {
    tauri::Builder::default()
        // Must be first: a second launch just brings the running window forward.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_main_window(app);
        }))
        // Opens the source-code link in the browser; the capability scopes it to the repository.
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_FLAG]),
        ))
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
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
                if app.state::<AppState>().settings().close_to_tray {
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
            commands::set_output_enabled,
            commands::set_output_delay,
            commands::set_output_gain,
            commands::set_clicks,
            commands::set_preferences,
            commands::get_autostart,
            commands::set_autostart,
            commands::start_calibration,
            commands::show_main_window,
            commands::hide_tray_popup,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tandem");
}
