//! The supervisor: a background loop that keeps the running engine matching the settings
//! and the devices actually connected.
//!
//! - Starts the engine on the source (the Windows default output, or a pinned device),
//!   restarts it when the default changes or the source comes back after a disconnect.
//! - Adds, removes and retries outputs as they are enabled, plugged in or lost.
//! - Applies delay and gain changes live.
//! - Optionally closes every stream while nothing plays, so Windows can sleep.
//! - Publishes a status snapshot to the UI and the tray.
//! - Optionally re-runs microphone auto-sync after a reconnect.
//!
//! The runtime lock is shared with the UI's commands, so nothing slow happens under it:
//! engines and outputs that go away are only told to stop there, and are waited for once
//! the lock is released (a thread on a vanished device can take a second to notice).

use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

use tandem_engine::devices::{self, mix_format};
use tandem_engine::engine::{Engine, OutputConfig, OutputHandle, SourceConfig};
use tauri::{AppHandle, Emitter, Manager};
use wasapi::{AudioMeterInformation, DeviceEnumerator};

use crate::settings::DeviceRef;
use crate::state::{
    AppState, DeviceDto, Levels, OutputState, OutputStatusDto, Snapshot, SourceState, SourceStatus,
    error_kind,
};

/// Meter levels go out at this pace; the full reconciliation runs every `FULL_EVERY` beats
/// (about four times per second).
const BEAT: Duration = Duration::from_millis(62);
const FULL_EVERY: u32 = 4;
const DEVICE_SCAN_EVERY: Duration = Duration::from_secs(1);
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// Let a fresh connection settle before auto-syncing it.
const RESYNC_SETTLE: Duration = Duration::from_secs(4);
/// Connections in the first moments after launch are not "reconnects".
const STARTUP_GRACE: Duration = Duration::from_secs(10);
/// With "pause when nothing plays": how long the source must stay silent.
const PAUSE_AFTER: Duration = Duration::from_secs(120);
/// Peak level below which the source counts as silent (-80 dB: below any dither).
const SILENCE: f32 = 1e-4;
/// Waking up from a pause is not a reconnect either.
const WAKE_GRACE: Duration = Duration::from_secs(5);

pub fn spawn(app: AppHandle) {
    thread::Builder::new()
        .name("supervisor".into())
        .spawn(move || {
            let _ = wasapi::initialize_mta();
            let mut sup = Supervisor::new();
            let mut beat: u32 = 0;
            loop {
                if beat % FULL_EVERY == 0 {
                    sup.tick(&app);
                } else {
                    publish_levels(&app);
                }
                beat = beat.wrapping_add(1);
                thread::sleep(BEAT);
            }
        })
        .expect("spawn supervisor");
}

fn publish_levels(app: &AppHandle) {
    let state = app.state::<AppState>();
    let rt = state.runtime.lock().unwrap();
    let Some(engine) = rt.engine.as_ref() else {
        return;
    };
    let levels = Levels {
        source: engine.source_level(),
        outputs: engine.outputs().iter().map(|o| (o.key().to_string(), o.status().level)).collect(),
    };
    drop(rt);
    let _ = app.emit("levels", &levels);
}

/// Find a remembered device among the connected ones: by id first, then by name.
pub fn resolve<'a>(connected: &'a [DeviceDto], wanted: &DeviceRef) -> Option<&'a DeviceDto> {
    connected
        .iter()
        .find(|d| d.id == wanted.id)
        .or_else(|| connected.iter().find(|d| d.name == wanted.name))
}

/// Engine parts that are going away. They have been told to stop; dropping this waits for
/// their threads, so it must happen outside the runtime lock.
#[derive(Default)]
struct Retired {
    engines: Vec<Engine>,
    outputs: Vec<OutputHandle>,
}

impl Retired {
    fn engine(&mut self, engine: Option<Engine>) {
        if let Some(engine) = engine {
            engine.signal_stop();
            self.engines.push(engine);
        }
    }

    fn output(&mut self, engine: &mut Engine, key: &str) {
        self.outputs.extend(engine.take_output(key));
    }
}

struct Supervisor {
    started_at: Instant,
    last_scan: Option<Instant>,
    source_retry_at: Option<Instant>,
    source_error: Option<String>,
    output_retry_at: HashMap<String, Instant>,
    output_errors: HashMap<String, String>,
    pending_resync: HashMap<String, Instant>,
    last_tray: Option<(bool, String)>,
    /// Last time the source carried sound (or pausing was not allowed).
    last_sound: Instant,
    /// Streams are closed because nothing plays; the source's meter is watched instead.
    paused: bool,
    /// The watched source's peak meter, by device id.
    meter: Option<(String, AudioMeterInformation)>,
    no_resync_until: Option<Instant>,
}

impl Supervisor {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            last_scan: None,
            source_retry_at: None,
            source_error: None,
            output_retry_at: HashMap::new(),
            output_errors: HashMap::new(),
            pending_resync: HashMap::new(),
            last_tray: None,
            last_sound: Instant::now(),
            paused: false,
            meter: None,
            no_resync_until: None,
        }
    }

    fn scan_devices(&mut self, state: &AppState) {
        let now = Instant::now();
        if self.last_scan.is_some_and(|t| now - t < DEVICE_SCAN_EVERY) {
            return;
        }
        self.last_scan = Some(now);
        // A failed enumeration keeps the previous list: treating it as "nothing is connected"
        // would stop every stream over a passing hiccup.
        let outputs = devices::render_devices();
        let inputs = devices::capture_devices();
        let mut known = state.devices.lock().unwrap();
        if let Ok(list) = outputs {
            known.outputs = list.into_iter().map(Into::into).collect();
        }
        if let Ok(list) = inputs {
            known.inputs = list.into_iter().map(Into::into).collect();
        }
    }

    /// What is playing on a device right now, from its own meter: works without any stream
    /// of ours. When the meter cannot be read, report sound, so a pause never gets stuck.
    fn device_peak(&mut self, id: &str) -> f32 {
        if self.meter.as_ref().is_none_or(|(known, _)| known != id) {
            self.meter = DeviceEnumerator::new()
                .and_then(|e| e.get_device(id))
                .and_then(|d| d.get_audiometerinformation())
                .ok()
                .map(|m| (id.to_string(), m));
        }
        match self.meter.as_ref().map(|(_, m)| m.get_peak_value()) {
            Some(Ok(peak)) => peak,
            _ => {
                self.meter = None;
                1.0
            }
        }
    }

    fn tick(&mut self, app: &AppHandle) {
        let state = app.state::<AppState>();
        self.scan_devices(&state);
        let now = Instant::now();
        let devices = state.devices.lock().unwrap().clone();
        let calibrating = state.calibrating.lock().unwrap().clone();
        let mut retired = Retired::default();

        let mut rt = state.runtime.lock().unwrap();
        // Read under the runtime lock: a command that changes a delay holds it while it updates
        // both the engine and the settings, so this copy can never be older than the engine.
        let settings = state.settings();
        let can_resync = settings.resync_on_reconnect
            && settings.mic.as_ref().is_some_and(|m| resolve(&devices.inputs, m).is_some());
        let may_resync = can_resync
            && now - self.started_at > STARTUP_GRACE
            && self.no_resync_until.is_none_or(|t| now >= t);

        // ---- Source ----------------------------------------------------------------
        let source_dev = if settings.source.follow_default {
            devices.outputs.iter().find(|d| d.is_default)
        } else {
            settings.source.device.as_ref().and_then(|r| resolve(&devices.outputs, r))
        };
        let mut source = SourceStatus::default();

        if !settings.enabled {
            retired.engine(rt.engine.take());
            rt.source_id = None;
            self.paused = false;
            source.state = SourceState::Off;
        } else if let Some(dev) = source_dev {
            source.device_id = Some(dev.id.clone());
            source.device_name = Some(dev.name.clone());
            // Tear down a dead engine, or one capturing a device that is no longer the source.
            let stale = match &rt.engine {
                Some(e) if !e.source_alive() => {
                    self.source_error = e.source_error();
                    self.source_retry_at = Some(now + RETRY_AFTER);
                    true
                }
                Some(_) => rt.source_id.as_deref() != Some(dev.id.as_str()),
                None => false,
            };
            if stale {
                retired.engine(rt.engine.take());
            }

            // Pause while nothing plays; wake at the first sound.
            let may_pause = settings.pause_when_silent && !rt.clicks && calibrating.is_none();
            if let Some(level) = rt.engine.as_ref().map(|e| e.source_level()) {
                if !may_pause || level > SILENCE {
                    self.last_sound = now;
                } else if now - self.last_sound > PAUSE_AFTER {
                    retired.engine(rt.engine.take());
                    rt.source_id = Some(dev.id.clone());
                    self.paused = true;
                }
            }
            if self.paused && (!may_pause || self.device_peak(&dev.id) > SILENCE) {
                self.paused = false;
                self.last_sound = now;
                self.no_resync_until = Some(now + WAKE_GRACE);
            }
            if !self.paused {
                self.meter = None;
            }

            if rt.engine.is_none() && !self.paused && self.source_retry_at.is_none_or(|t| now >= t) {
                let started = mix_format(&dev.id).and_then(|fmt| {
                    Engine::start(SourceConfig {
                        device_id: dev.id.clone(),
                        fmt,
                        follow_volume: settings.follow_source_volume,
                    })
                });
                match started {
                    Ok(engine) => {
                        engine.set_clicks(rt.clicks);
                        rt.engine = Some(engine);
                        rt.source_id = Some(dev.id.clone());
                        self.source_error = None;
                        self.source_retry_at = None;
                        self.output_retry_at.clear();
                        self.last_sound = now;
                        // A (re)connected source changes the path latency: re-sync everything.
                        if may_resync && self.no_resync_until.is_none_or(|t| now >= t) {
                            for o in settings.outputs.iter().filter(|o| o.enabled) {
                                self.pending_resync.insert(o.device.id.clone(), now + RESYNC_SETTLE);
                            }
                        }
                    }
                    Err(e) => {
                        self.source_error = Some(format!("{e:#}"));
                        self.source_retry_at = Some(now + RETRY_AFTER);
                    }
                }
            }
            match &rt.engine {
                Some(e) if e.source_alive() => {
                    e.set_follow_volume(settings.follow_source_volume);
                    source.state = SourceState::Running;
                    source.level = e.source_level();
                    source.sample_rate = Some(e.source_format().rate as u32);
                    source.hardware_volume = e.source_volume_is_hardware();
                }
                _ if self.paused => source.state = SourceState::Paused,
                _ => {
                    source.state = SourceState::Error;
                    source.error_kind = self.source_error.as_deref().map(error_kind);
                    source.error = self.source_error.clone();
                }
            }
        } else {
            retired.engine(rt.engine.take());
            rt.source_id = None;
            self.paused = false;
            source.state = SourceState::Waiting;
            source.device_name = settings.source.device.as_ref().map(|d| d.name.clone());
        }

        // ---- Outputs -----------------------------------------------------------------
        let source_id = rt.source_id.clone();
        let current_source = source_dev.map(|d| d.id.clone());
        let mut id_updates: Vec<(String, DeviceRef)> = Vec::new();
        let mut outputs = Vec::new();
        let wanted_keys: HashSet<&str> = settings.outputs.iter().map(|o| o.device.id.as_str()).collect();
        // Device ids that already belong to an output, including ones adopted in this pass.
        let mut claimed: HashSet<String> = wanted_keys.iter().map(|k| k.to_string()).collect();

        if let Some(engine) = rt.engine.as_mut() {
            let unknown: Vec<String> = engine
                .outputs()
                .iter()
                .map(|o| o.key().to_string())
                .filter(|k| !wanted_keys.contains(k.as_str()))
                .collect();
            for key in unknown {
                retired.output(engine, &key);
            }
        }

        for o in &settings.outputs {
            let key = o.device.id.clone();
            let delay_ms = o.delay_for(source_id.as_deref());
            let mut dto = OutputStatusDto {
                device_id: key.clone(),
                delay_ms,
                ..Default::default()
            };
            // A match by name only counts when no other output owns that device: two devices
            // can share a name, and two outputs must never end up playing on the same one.
            let present = resolve(&devices.outputs, &o.device)
                .filter(|dev| dev.id == o.device.id || !claimed.contains(&dev.id));
            if let Some(dev) = present
                && dev.id != o.device.id
            {
                // The device came back under a new id (e.g. Bluetooth re-pairing): adopt it.
                claimed.insert(dev.id.clone());
                id_updates.push((key.clone(), DeviceRef { id: dev.id.clone(), name: dev.name.clone() }));
            }

            let wanted_state = if !o.enabled {
                Some(OutputState::Disabled)
            } else if present.is_none() {
                Some(OutputState::Waiting)
            } else if present.map(|d| &d.id) == current_source.as_ref() {
                Some(OutputState::IsSource)
            } else if rt.engine.is_none() {
                Some(OutputState::Idle)
            } else {
                None
            };

            if let Some(s) = wanted_state {
                dto.state = s;
                if let Some(engine) = rt.engine.as_mut() {
                    retired.output(engine, &key);
                }
                if s != OutputState::Idle {
                    self.output_errors.remove(&key);
                }
                outputs.push(dto);
                continue;
            }

            let engine = rt.engine.as_mut().expect("checked above");
            let dev = present.expect("checked above");

            // Drop a dead output and schedule a retry.
            if let Some(handle) = engine.output(&key)
                && !handle.is_alive()
            {
                if let Some(e) = handle.error() {
                    self.output_errors.insert(key.clone(), e);
                }
                retired.output(engine, &key);
                self.output_retry_at.insert(key.clone(), now + RETRY_AFTER);
            }

            if engine.output(&key).is_none()
                && self.output_retry_at.get(&key).is_none_or(|t| now >= *t)
            {
                let added = mix_format(&dev.id).and_then(|fmt| {
                    engine
                        .add_output(OutputConfig {
                            key: key.clone(),
                            device_id: dev.id.clone(),
                            fmt,
                            delay_ms,
                            gain_db: o.gain_db,
                            muted: false,
                        })
                        .map(|_| ())
                });
                match added {
                    Ok(()) => {
                        self.output_retry_at.remove(&key);
                        self.output_errors.remove(&key);
                        if may_resync {
                            self.pending_resync.insert(key.clone(), now + RESYNC_SETTLE);
                        }
                    }
                    Err(e) => {
                        self.output_errors.insert(key.clone(), format!("{e:#}"));
                        self.output_retry_at.insert(key.clone(), now + RETRY_AFTER);
                    }
                }
            }

            match engine.output(&key) {
                Some(handle) => {
                    if handle.delay_ms() != delay_ms {
                        handle.set_delay_ms(delay_ms);
                    }
                    handle.set_gain_db(o.gain_db);
                    let st = handle.status();
                    dto.state = if st.running { OutputState::Playing } else { OutputState::Buffering };
                    dto.buffered_ms = st.buffered_ms;
                    dto.correction_ppm = st.correction_ppm;
                    dto.underruns = st.underruns;
                    dto.level = st.level;
                    dto.min_lag_ms = (st.min_lag_ms > 0.0).then_some(st.min_lag_ms);
                    dto.sample_rate = Some(handle.format().rate as u32);
                }
                None => {
                    dto.state = OutputState::Error;
                    dto.error = self.output_errors.get(&key).cloned();
                    dto.error_kind = dto.error.as_deref().map(error_kind);
                }
            }
            outputs.push(dto);
        }
        let clicks = rt.clicks;
        drop(rt);
        // Wait for stopped threads only now that the UI's commands are no longer locked out.
        drop(retired);

        if !id_updates.is_empty() {
            state.update_settings(|s| {
                for (old, new) in id_updates {
                    if s.sync_reference.as_deref() == Some(old.as_str()) {
                        s.sync_reference = Some(new.id.clone());
                    }
                    if let Some(o) = s.output_mut(&old) {
                        o.device = new;
                    }
                }
            });
        }

        // ---- Auto-sync after a reconnect ---------------------------------------------------
        if calibrating.is_none() {
            let due: Option<String> = self
                .pending_resync
                .iter()
                .find(|(id, at)| {
                    now >= **at
                        && outputs.iter().any(|o| &o.device_id == *id && o.state == OutputState::Playing)
                })
                .map(|(id, _)| id.clone());
            if let Some(id) = due {
                self.pending_resync.remove(&id);
                crate::calibration::spawn(app.clone(), id, true);
            }
        }
        if !can_resync {
            self.pending_resync.clear();
        }

        // ---- Publish -----------------------------------------------------------------------
        let snapshot = Snapshot {
            enabled: settings.enabled,
            source,
            outputs,
            clicks,
            calibrating,
            update: state.update_notice.lock().unwrap().clone(),
        };
        let tray = (snapshot.is_active(), crate::tray::tooltip(&snapshot, &settings));
        if self.last_tray.as_ref() != Some(&tray) {
            crate::tray::update(app, tray.0, &tray.1, settings.enabled);
            self.last_tray = Some(tray);
        }
        let _ = app.emit("status", &snapshot);
        *state.status.lock().unwrap() = snapshot;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, name: &str) -> DeviceDto {
        DeviceDto { id: id.into(), name: name.into(), is_default: false }
    }

    fn wanted(id: &str, name: &str) -> DeviceRef {
        DeviceRef { id: id.into(), name: name.into() }
    }

    #[test]
    fn a_device_is_found_by_id_before_name() {
        let connected = [dev("new", "Amp"), dev("old", "Amp")];
        assert_eq!(resolve(&connected, &wanted("old", "Amp")).unwrap().id, "old");
    }

    #[test]
    fn a_device_with_a_new_id_is_found_by_name() {
        let connected = [dev("a", "Speakers"), dev("b", "Amp")];
        assert_eq!(resolve(&connected, &wanted("gone", "Amp")).unwrap().id, "b");
        assert!(resolve(&connected, &wanted("gone", "Headphones")).is_none());
    }

    #[test]
    fn device_errors_are_sorted_into_kinds() {
        assert_eq!(error_kind("opening the output device: in use (0x8889000A)"), "inUse");
        assert_eq!(error_kind("The device was removed (0x88890004)"), "invalidated");
        assert_eq!(error_kind("the output device stopped responding"), "stalled");
        assert_eq!(error_kind("something else"), "other");
    }
}
