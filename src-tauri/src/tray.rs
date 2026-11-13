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
        Labels {
            mirroring: "Recopie active",
            show: "Afficher Tandem",
            quit: "Quitter",
        }
    } else {
        Labels {
            mirroring: "Mirroring",
            show: "Show Tandem",
            quit: "Quit",
        }
    }
}

fn build_menu(app: &AppHandle, settings: &Settings) -> tauri::Result<Menu<tauri::Wry>> {
    let l = labels(&settings.locale);
    let mirroring = CheckMenuItem::with_id(
        app,
        "mirroring",
        l.mirroring,
        true,
        settings.enabled,
        None::<&str>,
    )?;
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
                position,
                ..
            } = event
            {
                toggle_popup(tray.app_handle(), position);
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
    let playing = snap
        .outputs
        .iter()
        .filter(|o| o.state == OutputState::Playing)
        .count();
    let line = match snap.source.state {
        SourceState::Off => (if fr { "En pause" } else { "Paused" }).to_string(),
        SourceState::Waiting => match &snap.source.device_name {
            Some(n) if fr => format!("En attente de {n}"),
            Some(n) => format!("Waiting for {n}"),
            None if fr => "En attente de la source".into(),
            None => "Waiting for the source".into(),
        },
        SourceState::Error => (if fr {
            "Source indisponible"
        } else {
            "Source unavailable"
        })
        .to_string(),
        SourceState::Paused => (if fr {
            "En veille, rien ne joue"
        } else {
            "Asleep, nothing is playing"
        })
        .to_string(),
        SourceState::Running if playing == 0 => (if fr {
            "Aucune sortie active"
        } else {
            "No active output"
        })
        .to_string(),
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

fn toggle_popup(app: &AppHandle, click: PhysicalPosition<f64>) {
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
    position_popup(app, &window, click);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Seat the popup in the corner of the screen where the tray icon was clicked, inside that
/// monitor's work area: wherever the taskbar is (bottom, top or a side, any monitor, any
/// height), the popup sits next to it without covering it.
fn position_popup(app: &AppHandle, window: &WebviewWindow, click: PhysicalPosition<f64>) {
    let monitor = app
        .monitor_from_point(click.x, click.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        return;
    };
    let margin = (POPUP_MARGIN * monitor.scale_factor()) as i32;
    let area = monitor.work_area();
    let (left, top) = (area.position.x, area.position.y);
    let (right, bottom) = (left + area.size.width as i32, top + area.size.height as i32);
    let (w, h) = (size.width as i32, size.height as i32);
    let x = if click.x >= (left + right) as f64 / 2.0 {
        right - w - margin
    } else {
        left + margin
    };
    let y = if click.y >= (top + bottom) as f64 / 2.0 {
        bottom - h - margin
    } else {
        top + margin
    };
    let _ = window.set_position(PhysicalPosition::new(x.max(left), y.max(top)));
}

/// Windows 11 rounds frameless windows only when asked.
///
/// Tauri's own way to get round corners on a frameless window is `set_shadow(true)`, which
/// also draws a shadow and a one-pixel border; asking the window manager directly rounds the
/// corners and nothing else. That call has no safe wrapper.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn round_corners(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
        DwmSetWindowAttribute,
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let preference: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
    // SAFETY: `hwnd` is the handle of `window`, which is alive for the whole call (we hold a
    // reference to it). For `DWMWA_WINDOW_CORNER_PREFERENCE` Windows reads exactly one
    // `DWM_WINDOW_CORNER_PREFERENCE` from the pointer: it points at `preference`, a live
    // local of that type, and the size passed is that type's size. Windows only reads it.
    // A failure (an older Windows without this attribute) is ignored: corners stay square.
    let _ = unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd.0 as _),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    };
}

#[cfg(not(windows))]
pub fn round_corners(_window: &WebviewWindow) {}

// ---------------------------------------------------------------------------------------
// Icon drawing: the Tandem mark (three notes wrapped round a centre point; each note's flag
// sweeps a third of the ring). Slightly bolder than the logo so it survives 16 px.
// ---------------------------------------------------------------------------------------

type Rgb = [f32; 3];
type Point = (f32, f32);

const CENTRE: Point = (0.5, 0.5);
/// The ring the flags draw, and half the line weight of flags and stems.
const RING_RADIUS: f32 = 0.397;
const HALF_LINE: f32 = 0.045;
/// A flag starts at its stem's top and ends short of the next note (radians, clockwise).
const FLAG_FROM: f32 = -1.749;
const FLAG_TO: f32 = -0.138;
const STEM_TOP: Point = (0.430, 0.109);
const STEM_FOOT: Point = (0.430, 0.385);
const HEAD: Point = (0.352, 0.402);
const HEAD_RADII: Point = (0.120, 0.092);
const HEAD_TILT: f32 = -0.419;
const DOT_RADIUS: f32 = 0.062;

fn hex(c: u32) -> Rgb {
    [
        ((c >> 16) & 0xff) as f32,
        ((c >> 8) & 0xff) as f32,
        (c & 0xff) as f32,
    ]
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Signed distance to the segment a-b thickened by `r`.
fn capsule_sd(p: Point, a: Point, b: Point, r: f32) -> f32 {
    let (px, py) = (p.0 - a.0, p.1 - a.1);
    let (bx, by) = (b.0 - a.0, b.1 - a.1);
    let t = ((px * bx + py * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
    ((px - bx * t).powi(2) + (py - by * t).powi(2)).sqrt() - r
}

/// Approximate signed distance to an ellipse tilted by `angle`.
fn ellipse_sd(p: Point, centre: Point, radii: Point, angle: f32) -> f32 {
    let (sin, cos) = angle.sin_cos();
    let (dx, dy) = (p.0 - centre.0, p.1 - centre.1);
    let (x, y) = (dx * cos + dy * sin, -dx * sin + dy * cos);
    (((x / radii.0).powi(2) + (y / radii.1).powi(2)).sqrt() - 1.0) * radii.0.min(radii.1)
}

/// Signed distance to an arc of the ring between two angles, thickened by `half`.
fn arc_sd(p: Point, from: f32, to: f32, half: f32) -> f32 {
    use std::f32::consts::TAU;
    let (dx, dy) = (p.0 - CENTRE.0, p.1 - CENTRE.1);
    let mut angle = dy.atan2(dx);
    while angle < from {
        angle += TAU;
    }
    if angle <= to {
        return (distance(p, CENTRE) - RING_RADIUS).abs() - half;
    }
    let end = |a: f32| {
        (
            CENTRE.0 + RING_RADIUS * a.cos(),
            CENTRE.1 + RING_RADIUS * a.sin(),
        )
    };
    distance(p, end(from)).min(distance(p, end(to))) - half
}

/// Signed distance to one note in its own (unrotated) position.
fn note_sd(p: Point) -> f32 {
    arc_sd(p, FLAG_FROM, FLAG_TO, HALF_LINE)
        .min(capsule_sd(p, STEM_TOP, STEM_FOOT, HALF_LINE))
        .min(ellipse_sd(p, HEAD, HEAD_RADII, HEAD_TILT))
}

/// Draw the mark at `size` px. Geometry is in a 0..1 unit square, matching the SVG logo.
pub fn draw_icon(size: u32, active: bool) -> Image<'static> {
    let s = size as f32;
    // One note per third of a turn: (color from, color to).
    let notes: [(u32, u32); 3] = [
        (0x1e9cc0, 0x6bbed5),
        (0x54b24c, 0x8ecc89),
        (0x7a56d0, 0xa78fe0),
    ];
    let dot = hex(0xf2a81e);
    let grey = hex(0x8c8c98);

    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let p = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let mut color: Rgb = [0.0; 3];
            let mut alpha = 0f32;
            let mut paint = |coverage: f32, c: Rgb| {
                if coverage > 0.0 {
                    color = [0, 1, 2].map(|i| color[i] * (1.0 - coverage) + c[i] * coverage);
                    alpha += coverage * (1.0 - alpha);
                }
            };
            for (k, (c0, c1)) in notes.iter().enumerate() {
                // Rotate the pixel back by this note's turn instead of rotating the note.
                let (sin, cos) = (k as f32 * std::f32::consts::TAU / 3.0).sin_cos();
                let (dx, dy) = (p.0 - CENTRE.0, p.1 - CENTRE.1);
                let local = (
                    CENTRE.0 + dx * cos + dy * sin,
                    CENTRE.1 - dx * sin + dy * cos,
                );
                let coverage = (0.5 - note_sd(local) * s).clamp(0.0, 1.0);
                let t = ((local.0 - 0.3) / 0.6).clamp(0.0, 1.0);
                let (a0, a1) = (hex(*c0), hex(*c1));
                let c = if active {
                    [0, 1, 2].map(|i| a0[i] + (a1[i] - a0[i]) * t)
                } else {
                    grey
                };
                paint(coverage, c);
            }
            let coverage = (0.5 - (distance(p, CENTRE) - DOT_RADIUS) * s).clamp(0.0, 1.0);
            paint(coverage, if active { dot } else { grey });

            let o = ((y * size + x) * 4) as usize;
            rgba[o] = color[0] as u8;
            rgba[o + 1] = color[1] as u8;
            rgba[o + 2] = color[2] as u8;
            rgba[o + 3] = (alpha * 255.0) as u8;
        }
    }
    Image::new_owned(rgba, size, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes the tray icon to the temp directory for a visual check:
    /// `cargo test -p tandem tray_icon_preview -- --ignored`.
    #[test]
    #[ignore]
    fn tray_icon_preview() {
        for (size, active) in [(16, true), (32, true), (32, false), (256, true)] {
            let image = draw_icon(size, active);
            let mut ppm = format!("P6 {size} {size} 255\n").into_bytes();
            for px in image.rgba().chunks_exact(4) {
                // Composite over a dark taskbar grey.
                let a = px[3] as f32 / 255.0;
                ppm.extend(
                    px[..3]
                        .iter()
                        .map(|c| (*c as f32 * a + 32.0 * (1.0 - a)) as u8),
                );
            }
            let name = format!(
                "tandem-tray-{size}-{}.ppm",
                if active { "on" } else { "off" }
            );
            std::fs::write(std::env::temp_dir().join(name), ppm).unwrap();
        }
    }

    #[test]
    fn the_icon_has_three_notes_and_a_centre() {
        let image = draw_icon(64, true);
        let at = |x: f32, y: f32| {
            let o = ((y * 64.0) as usize * 64 + (x * 64.0) as usize) * 4;
            image.rgba()[o + 3]
        };
        // Centre point, each stem, and a point on the ring are painted; the corners are not.
        assert!(at(0.5, 0.5) > 200);
        assert!(at(0.43, 0.25) > 200);
        assert!(at(0.5, 0.103) > 200);
        assert_eq!(at(0.02, 0.02), 0);
        assert_eq!(at(0.98, 0.98), 0);
    }
}
