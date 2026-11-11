//! Microphone auto-sync for one output.
//!
//! 1. Mute every output and start recording the microphone.
//! 2. Play sweep #1 on the source: the mic hears the source alone.
//! 3. Unmute the output being synced and play sweep #2: the mic hears the source and
//!    the output's delayed copy.
//! 4. The analysis gives the output-minus-source offset. Correct the delay by it, then run
//!    a second pass to verify.
//!
//! With a **reference output** chosen, that output plays the part of the source: it alone is
//! unmuted for sweep #1. This is for a source nobody listens to (several outputs of one
//! interface), which the microphone cannot hear. An output that is still behind the
//! reference at 0 ms cannot be moved earlier, so the reference is delayed instead.
//!
//! Mutes are restored on every exit path.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tandem_engine::calibrate::{self, CalibrationError};
use tandem_engine::engine::{Engine, MAX_DELAY_MS, MIN_DELAY_MS};
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
    ReferenceNotRunning,
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
    /// Device id of the output used in place of the source, if any.
    pub reference: Option<String>,
    /// Set when the reference had to be delayed to line up: its new delay.
    pub reference_delay_ms: Option<u32>,
    pub failure: Option<Failure>,
}

pub fn spawn(app: AppHandle, device_id: String, automatic: bool) {
    let _ = thread::Builder::new()
        .name("auto-sync".into())
        .spawn(move || run(&app, &device_id, automatic));
}

/// Clears the "auto-sync in progress" flag when dropped, so a failure halfway (even a
/// panic) never leaves auto-sync refusing to start again.
struct Busy<'a>(&'a AppState);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        *self.0.calibrating.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
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
        reference: None,
        reference_delay_ms: None,
        failure: None,
    };
    {
        let mut busy = state.calibrating.lock().unwrap();
        if busy.is_some() {
            emit(
                app,
                CalibrationEvent {
                    phase: "failed",
                    failure: Some(Failure::Busy),
                    ..base
                },
            );
            return;
        }
        *busy = Some(device_id.to_string());
    }
    let busy = Busy(&state);

    let previous = current_delay(&state, device_id);
    let reference = reference_for(&state, device_id);
    let mut last = base.clone();
    last.previous_delay_ms = previous;
    last.reference = reference.clone();
    let mut outcome: Result<(), Failure> = Ok(());
    for pass in 1..=PASSES {
        emit(
            app,
            CalibrationEvent {
                phase: "listening",
                pass,
                ..last.clone()
            },
        );
        let offset = match measure_once(&state, device_id, reference.as_deref()) {
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
        let short = MIN_DELAY_MS as f64 - wanted;
        if short > 0.5
            && let Some(reference) = reference.as_deref()
            && let Some(ref_delay) = current_delay(&state, reference)
        {
            // The output is behind the reference even at 0 ms: hold the reference back.
            let moved = ((ref_delay as f64 + short).round() as u32).min(MAX_DELAY_MS);
            apply_delay(&state, reference, moved);
            last.reference_delay_ms = Some(moved);
        } else if short > 0.5 {
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

    drop(busy);
    match outcome {
        Ok(()) if last.lag_ms.is_some() => emit(
            app,
            CalibrationEvent {
                phase: "limited",
                ..last
            },
        ),
        Ok(()) => emit(
            app,
            CalibrationEvent {
                phase: "done",
                ..last
            },
        ),
        Err(f) => emit(
            app,
            CalibrationEvent {
                phase: "failed",
                failure: Some(f),
                ..last
            },
        ),
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
    // Engine and settings change together under the runtime lock, so the supervisor never
    // re-applies an older delay in between.
    let rt = state.runtime.lock().unwrap();
    let source = rt.source_id.clone();
    if let Some(h) = rt.engine.as_ref().and_then(|e| e.output(device_id)) {
        h.set_delay_ms(ms);
    }
    state.update_settings(|s| {
        if let Some(o) = s.output_mut(device_id) {
            o.set_delay_for(source.as_deref(), ms);
        }
    });
}

/// The output to sync against instead of the source, when one is set and usable.
fn reference_for(state: &AppState, device_id: &str) -> Option<String> {
    let settings = state.settings.lock().unwrap();
    settings.sync_reference.clone().filter(|r| {
        r != device_id
            && settings
                .outputs
                .iter()
                .any(|o| &o.device.id == r && o.enabled)
    })
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

/// One measurement: returns the output's offset in seconds from the source, or from the
/// `reference` output when there is one.
fn measure_once(
    state: &AppState,
    device_id: &str,
    reference: Option<&str>,
) -> Result<f64, Failure> {
    let mic = state.settings().mic.ok_or(Failure::NoMic)?;
    let mic_id = {
        let devices = state.devices.lock().unwrap();
        resolve(&devices.inputs, &mic)
            .ok_or(Failure::MicMissing)?
            .id
            .clone()
    };
    let (src_rate, delay_ms) = with_engine(state, |e| {
        e.output(device_id)
            .filter(|h| h.is_alive() && h.status().running)
            .map(|h| (e.source_format().rate, h.delay_ms()))
    })?
    .ok_or(Failure::NotRunning)?;
    let reference_delay_ms = match reference {
        Some(r) => with_engine(state, |e| {
            e.output(r)
                .filter(|h| h.is_alive() && h.status().running)
                .map(|h| h.delay_ms())
        })?
        .ok_or(Failure::ReferenceNotRunning)?,
        None => 0,
    };
    let longest_delay_s = delay_ms.max(reference_delay_ms) as f64 / 1000.0;

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
    if let Some(r) = reference {
        // The reference stands in for the source: it plays both sweeps.
        with_engine(state, |e| e.output(r).map(|h| h.set_muted(false)))?
            .ok_or(Failure::ReferenceNotRunning)?;
    }
    sleep_s(0.4);

    let sweep = Arc::new(calibrate::log_sweep(
        src_rate,
        SWEEP_S,
        SWEEP_F0,
        SWEEP_F1,
        SWEEP_AMPLITUDE,
    ));
    let before = with_engine(state, |e| e.signal_start_frames().len())?;
    with_engine(state, |e| e.play_signal(Arc::clone(&sweep)))?;
    // Wait until every delayed copy of sweep #1 is entirely past.
    sleep_s(SWEEP_S + longest_delay_s + 0.4);
    with_engine(state, |e| e.output(device_id).map(|h| h.set_muted(false)))?
        .ok_or(Failure::NotRunning)?;
    sleep_s(0.15);
    with_engine(state, |e| e.play_signal(Arc::clone(&sweep)))?;
    let reference_wait_s = if reference.is_some() {
        longest_delay_s
    } else {
        0.0
    };
    sleep_s(SWEEP_S + SOURCE_LATENCY_MARGIN_S + MAX_OFFSET_S + reference_wait_s);

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
