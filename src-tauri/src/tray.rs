//! The system tray: left-click toggles a small status popup (the "tray" window), right-click
//! opens a menu. The icon is drawn in code in two states: colored while audio flows to at
//! least one output, grey otherwise.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

use crate::settings::Settings;
use crate::state::{AppState, OutputState, Snapshot, SourceState};

const TRAY_ID: &str = "tandem";
const POPUP_MARGIN: f64 = 12.0;
const TASKBAR_RESERVE: f64 = 48.0;
/// A tray click right after the popup lost focus is the click that dismissed it.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct TrayState {
    hidden_at: Mutex<Option<Instant>>,
}

struct Labels {
    mirroring: &'static str,
    show: &'static str,
    quit: &'static str,
}

fn labels(locale: &str) -> Labels {
    if locale == "fr" {
        Labels { mirroring: "Recopie active", show: "Afficher Tandem", quit: "Quitter" }
    } else {
        Labels { mirroring: "Mirroring", show: "Show Tandem", quit: "Quit" }
    }
}

fn build_menu(app: &AppHandle, settings: &Settings) -> tauri::Result<Menu<tauri::Wry>> {
    let l = labels(&settings.locale);
    let mirroring = CheckMenuItem::with_id(app, "mirroring", l.mirroring, true, settings.enabled, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", l.show, true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
    Menu::with_items(app, &[&mirroring, &show, &sep, &quit])
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    app.manage(TrayState::default());
    let settings = app.state::<AppState>().settings();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(draw_icon(32, false))
        .tooltip("Tandem")
        .menu(&build_menu(app, &settings)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "mirroring" => {
                let state = app.state::<AppState>();
                state.update_settings(|s| s.enabled = !s.enabled);
                let _ = app.emit("settings", state.settings());
                refresh_menu(app);
            }
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_popup(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn tray(app: &AppHandle) -> Option<TrayIcon> {
    app.tray_by_id(TRAY_ID)
}

/// Rebuild the menu (locale or master switch changed).
pub fn refresh_menu(app: &AppHandle) {
    let settings = app.state::<AppState>().settings();
    if let (Some(t), Ok(menu)) = (tray(app), build_menu(app, &settings)) {
        let _ = t.set_menu(Some(menu));
    }
}

/// Called by the supervisor when the coarse state or the tooltip changes.
pub fn update(app: &AppHandle, active: bool, tooltip: &str, _enabled: bool) {
    if let Some(t) = tray(app) {
        let _ = t.set_icon(Some(draw_icon(32, active)));
        let _ = t.set_tooltip(Some(tooltip));
    }
    refresh_menu(app);
}

/// A one-line summary for the tray tooltip.
pub fn tooltip(snap: &Snapshot, settings: &Settings) -> String {
    let fr = settings.locale == "fr";
    let playing = snap.outputs.iter().filter(|o| o.state == OutputState::Playing).count();
    let line = match snap.source.state {
        SourceState::Off => (if fr { "En pause" } else { "Paused" }).to_string(),
        SourceState::Waiting => match &snap.source.device_name {
            Some(n) if fr => format!("En attente de {n}"),
            Some(n) => format!("Waiting for {n}"),
            None if fr => "En attente de la source".into(),
            None => "Waiting for the source".into(),
        },
        SourceState::Error => (if fr { "Source indisponible" } else { "Source unavailable" }).to_string(),
        SourceState::Running if playing == 0 => {
            (if fr { "Aucune sortie active" } else { "No active output" }).to_string()
        }
        SourceState::Running => match (fr, playing) {
            (true, 1) => "1 sortie synchronisée".into(),
            (true, n) => format!("{n} sorties synchronisées"),
            (false, 1) => "1 output in sync".into(),
            (false, n) => format!("{n} outputs in sync"),
        },
    };
    format!("Tandem · {line}")
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Some(p) = app.get_webview_window("tray") {
        let _ = p.hide();
    }
}

pub fn stamp_hidden(app: &AppHandle) {
    if let Some(s) = app.try_state::<TrayState>() {
        *s.hidden_at.lock().unwrap() = Some(Instant::now());
    }
}

fn toggle_popup(app: &AppHandle) {
    let Some(window) = app.get_webview_window("tray") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let just_dismissed = app
        .try_state::<TrayState>()
        .and_then(|s| *s.hidden_at.lock().unwrap())
        .is_some_and(|t| t.elapsed() < REOPEN_GUARD);
    if visible || just_dismissed {
        let _ = window.hide();
        stamp_hidden(app);
        return;
    }
    position_popup(app, &window);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Seat the popup at the bottom-right of the primary monitor, above the taskbar.
fn position_popup(app: &AppHandle, window: &WebviewWindow) {
    let (Ok(Some(monitor)), Ok(size)) = (app.primary_monitor(), window.outer_size()) else {
        return;
    };
    let scale = monitor.scale_factor();
    let margin = (POPUP_MARGIN * scale) as i32;
    let reserve = (TASKBAR_RESERVE * scale) as i32;
    let origin = monitor.position();
    let extent = monitor.size();
    let x = (origin.x + extent.width as i32 - size.width as i32 - margin).max(origin.x + margin);
    let y = (origin.y + extent.height as i32 - size.height as i32 - margin - reserve).max(origin.y + margin);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Windows 11 rounds frameless windows only when asked.
#[cfg(windows)]
pub fn round_corners(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    if let Ok(hwnd) = window.hwnd() {
        let pref = DWMWCP_ROUND;
        unsafe {
            let _ = DwmSetWindowAttribute(
                HWND(hwnd.0 as _),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &pref as *const _ as *const _,
                std::mem::size_of_val(&pref) as u32,
            );
        }
    }
}

#[cfg(not(windows))]
pub fn round_corners(_window: &WebviewWindow) {}

// ---------------------------------------------------------------------------------------
// Icon drawing: the Tandem mark (three staggered signal bars ending on one sync line).
// ---------------------------------------------------------------------------------------

type Rgb = [f32; 3];

fn hex(c: u32) -> Rgb {
    [((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32]
}

/// Signed distance from (px, py) to a capsule spanning (x0..x1) at height cy with radius r.
fn capsule_sd(px: f32, py: f32, x0: f32, x1: f32, cy: f32, r: f32) -> f32 {
    let qx = px.clamp(x0 + r, x1 - r);
    ((px - qx).powi(2) + (py - cy).powi(2)).sqrt() - r
}

fn vcapsule_sd(px: f32, py: f32, cx: f32, y0: f32, y1: f32, r: f32) -> f32 {
    let qy = py.clamp(y0 + r, y1 - r);
    ((px - cx).powi(2) + (py - qy).powi(2)).sqrt() - r
}

/// Draw the mark at `size` px. Geometry is in a 0..1 unit square, matching the SVG logo.
pub fn draw_icon(size: u32, active: bool) -> Image<'static> {
    let s = size as f32;
    // (start x, center y, color from, color to)
    let bars: [(f32, f32, u32, u32); 3] = [
        (0.10, 0.25, 0x1e9cc0, 0x6bbed5),
        (0.36, 0.50, 0x54b24c, 0x8ecc89),
        (0.22, 0.75, 0x7a56d0, 0xa78fe0),
    ];
    let bar_end = 0.74;
    let r = 0.095;
    let sync = (0.86, 0.08, 0.92, 0.055, 0xf2a81e);
    let grey = hex(0x8c8c98);

    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (px, py) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let mut color: Rgb = [0.0; 3];
            let mut alpha = 0f32;
            for (x0, cy, c0, c1) in bars {
                let d = capsule_sd(px, py, x0, bar_end, cy, r) * s;
                let a = (0.5 - d).clamp(0.0, 1.0);
                if a > 0.0 {
                    let t = ((px - x0) / (bar_end - x0)).clamp(0.0, 1.0);
                    let (a0, a1) = (hex(c0), hex(c1));
                    let c = if active {
                        [0, 1, 2].map(|i| a0[i] + (a1[i] - a0[i]) * t)
                    } else {
                        grey
                    };
                    color = [0, 1, 2].map(|i| color[i] * (1.0 - a) + c[i] * a);
                    alpha = alpha + a * (1.0 - alpha);
                }
            }
            let (cx, y0, y1, sr, sc) = sync;
            let d = vcapsule_sd(px, py, cx, y0, y1, sr) * s;
            let a = (0.5 - d).clamp(0.0, 1.0);
            if a > 0.0 {
                let c = if active { hex(sc) } else { grey };
                color = [0, 1, 2].map(|i| color[i] * (1.0 - a) + c[i] * a);
                alpha = alpha + a * (1.0 - alpha);
            }
            let o = ((y * size + x) * 4) as usize;
            rgba[o] = color[0] as u8;
            rgba[o + 1] = color[1] as u8;
            rgba[o + 2] = color[2] as u8;
            rgba[o + 3] = (alpha * 255.0) as u8;
        }
    }
    Image::new_owned(rgba, size, size)
}
