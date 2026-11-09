//! Microphone auto-sync for one output.
//!
//! 1. Mute every output and start recording the microphone.
//! 2. Play sweep #1 on the source: the mic hears the source alone.
//! 3. Unmute the output being synced and play sweep #2: the mic hears the source and
//!    the output's delayed copy.
//! 4. The analysis gives the output-minus-source offset. Correct the delay by it, then run
//!    a second pass to verify.
//!
//! Mutes are restored on every exit path.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tandem_engine::calibrate::{self, CalibrationError};
use tandem_engine::engine::{Engine, MIN_DELAY_MS};
use tandem_engine::mic::Recording;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;
use crate::supervisor::resolve;

const SWEEP_S: f64 = 1.0;
const SWEEP_F0: f64 = 120.0;
const SWEEP_F1: f64 = 9_000.0;
const SWEEP_AMPLITUDE: f32 = 0.35;
/// The output may be off by up to this much before auto-sync.
const MAX_OFFSET_S: f64 = 1.0;
/// Worst-case source latency (Bluetooth) plus margin, waited after each sweep.
const SOURCE_LATENCY_MARGIN_S: f64 = 0.8;
/// Below this residual, the second pass is not needed.
const GOOD_ENOUGH_MS: f64 = 1.5;
const PASSES: u8 = 2;

/// Error codes the UI translates.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Failure {
    NoMic,
    MicMissing,
    MicFailed,
    NotRunning,
    SourceNotHeard,
    TargetNotHeard,
    TooShort,
    Busy,
}

impl From<CalibrationError> for Failure {
    fn from(e: CalibrationError) -> Self {
        match e {
            CalibrationError::SourceNotHeard => Failure::SourceNotHeard,
            CalibrationError::TargetNotHeard => Failure::TargetNotHeard,
            CalibrationError::RecordingTooShort => Failure::TooShort,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationEvent {
    pub device_id: String,
    /// "listening" | "done" | "limited" | "failed". "limited": the output was set to 0 ms
    /// and still plays `lag_ms` after the source, so the source is too fast for it.
    pub phase: &'static str,
    pub pass: u8,
    pub automatic: bool,
    /// Offset found on the last pass, output minus source.
    pub offset_ms: Option<f64>,
    /// Delay before auto-sync started.
    pub previous_delay_ms: Option<u32>,
    /// Delay after auto-sync.
    pub delay_ms: Option<u32>,
    /// With phase "limited": how late the output still is at 0 ms.
    pub lag_ms: Option<f64>,
    pub failure: Option<Failure>,
}

pub fn spawn(app: AppHandle, device_id: String, automatic: bool) {
    let _ = thread::Builder::new()
        .name("auto-sync".into())
        .spawn(move || run(&app, &device_id, automatic));
}

fn emit(app: &AppHandle, ev: CalibrationEvent) {
    let _ = app.emit("calibration", ev);
}

fn run(app: &AppHandle, device_id: &str, automatic: bool) {
    let state = app.state::<AppState>();
    let base = CalibrationEvent {
        device_id: device_id.to_string(),
        phase: "listening",
        pass: 1,
        automatic,
        offset_ms: None,
        previous_delay_ms: None,
        delay_ms: None,
        lag_ms: None,
        failure: None,
    };
    {
        let mut busy = state.calibrating.lock().unwrap();
        if busy.is_some() {
            emit(app, CalibrationEvent { phase: "failed", failure: Some(Failure::Busy), ..base });
            return;
        }
        *busy = Some(device_id.to_string());
    }

    let previous = current_delay(&state, device_id);
    let mut last = base.clone();
    last.previous_delay_ms = previous;
    let mut outcome: Result<(), Failure> = Ok(());
    for pass in 1..=PASSES {
        emit(app, CalibrationEvent { phase: "listening", pass, ..last.clone() });
        let offset = match measure_once(&state, device_id) {
            Ok(o) => o,
            Err(f) => {
                outcome = Err(f);
                break;
            }
        };
        let Some(delay) = current_delay(&state, device_id) else {
            outcome = Err(Failure::NotRunning);
            break;
        };
        let wanted = delay as f64 - offset * 1000.0;
        let new_delay = wanted.round().max(MIN_DELAY_MS as f64) as u32;
        apply_delay(&state, device_id, new_delay);
        last.pass = pass;
        last.offset_ms = Some(offset * 1000.0);
        last.delay_ms = Some(new_delay);
        if wanted < MIN_DELAY_MS as f64 - 0.5 {
            // Even at 0 ms the output lags: no delay can fix that, only a slower source.
            last.lag_ms = Some(MIN_DELAY_MS as f64 - wanted);
            break;
        }
        if (offset * 1000.0).abs() < GOOD_ENOUGH_MS {
            break;
        }
        // Let the delay jump settle before measuring again.
        thread::sleep(Duration::from_millis(600));
    }

    *state.calibrating.lock().unwrap() = None;
    match outcome {
        Ok(()) if last.lag_ms.is_some() => emit(app, CalibrationEvent { phase: "limited", ..last }),
        Ok(()) => emit(app, CalibrationEvent { phase: "done", ..last }),
        Err(f) => emit(app, CalibrationEvent { phase: "failed", failure: Some(f), ..last }),
    }
}

fn with_engine<R>(state: &AppState, f: impl FnOnce(&Engine) -> R) -> Result<R, Failure> {
    let rt = state.runtime.lock().unwrap();
    rt.engine.as_ref().map(f).ok_or(Failure::NotRunning)
}

fn current_delay(state: &AppState, device_id: &str) -> Option<u32> {
    let source = state.runtime.lock().unwrap().source_id.clone();
    let settings = state.settings.lock().unwrap();
    settings
        .outputs
        .iter()
        .find(|o| o.device.id == device_id)
        .map(|o| o.delay_for(source.as_deref()))
}

pub fn apply_delay(state: &AppState, device_id: &str, ms: u32) {
    let rt = state.runtime.lock().unwrap();
    let source = rt.source_id.clone();
    if let Some(h) = rt.engine.as_ref().and_then(|e| e.output(device_id)) {
        h.set_delay_ms(ms);
    }
    drop(rt);
    state.update_settings(|s| {
        if let Some(o) = s.output_mut(device_id) {
            o.set_delay_for(source.as_deref(), ms);
        }
    });
}

/// Restores every output's mute state when dropped.
struct MuteGuard<'a> {
    state: &'a AppState,
    prior: Vec<(String, bool)>,
}

impl Drop for MuteGuard<'_> {
    fn drop(&mut self) {
        let _ = with_engine(self.state, |e| {
            for (key, muted) in &self.prior {
                if let Some(h) = e.output(key) {
                    h.set_muted(*muted);
                }
            }
        });
    }
}

fn sleep_s(s: f64) {
    thread::sleep(Duration::from_secs_f64(s));
}

/// One measurement: returns output-minus-source offset in seconds.
fn measure_once(state: &AppState, device_id: &str) -> Result<f64, Failure> {
    let mic = state.settings().mic.ok_or(Failure::NoMic)?;
    let mic_id = {
        let devices = state.devices.lock().unwrap();
        resolve(&devices.inputs, &mic).ok_or(Failure::MicMissing)?.id.clone()
    };
    let (src_rate, delay_ms) = with_engine(state, |e| {
        e.output(device_id)
            .filter(|h| h.is_alive() && h.status().running)
            .map(|h| (e.source_format().rate, h.delay_ms()))
    })?
    .ok_or(Failure::NotRunning)?;

    // Silence every output; only the one being synced comes back for sweep #2.
    let prior = with_engine(state, |e| {
        e.outputs()
            .iter()
            .map(|h| {
                let p = (h.key().to_string(), h.muted());
                h.set_muted(true);
                p
            })
            .collect::<Vec<_>>()
    })?;
    let _guard = MuteGuard { state, prior };

    let recording = Recording::start(&mic_id).map_err(|_| Failure::MicFailed)?;
    if !recording.wait_started(Duration::from_secs(2)) {
        return Err(Failure::MicFailed);
    }
    let mic_rate = recording.rate();
    sleep_s(0.4);

    let sweep = Arc::new(calibrate::log_sweep(src_rate, SWEEP_S, SWEEP_F0, SWEEP_F1, SWEEP_AMPLITUDE));
    let before = with_engine(state, |e| e.signal_start_frames().len())?;
    with_engine(state, |e| e.play_signal(Arc::clone(&sweep)))?;
    // Wait until the muted output's delayed copy of sweep #1 is entirely past.
    sleep_s(SWEEP_S + delay_ms as f64 / 1000.0 + 0.4);
    with_engine(state, |e| e.output(device_id).map(|h| h.set_muted(false)))?
        .ok_or(Failure::NotRunning)?;
    sleep_s(0.15);
    with_engine(state, |e| e.play_signal(Arc::clone(&sweep)))?;
    sleep_s(SWEEP_S + SOURCE_LATENCY_MARGIN_S + MAX_OFFSET_S);

    let frames = with_engine(state, |e| e.signal_start_frames())?;
    let samples = recording.finish().map_err(|_| Failure::MicFailed)?;
    if frames.len() < before + 2 {
        return Err(Failure::NotRunning);
    }
    let spacing_s = (frames[before + 1] - frames[before]) as f64 / src_rate as f64;
    let template = calibrate::log_sweep(mic_rate, SWEEP_S, SWEEP_F0, SWEEP_F1, SWEEP_AMPLITUDE);
    let m = calibrate::analyze(&samples, mic_rate, &template, spacing_s, MAX_OFFSET_S)?;
    Ok(m.target_offset_s)
}
