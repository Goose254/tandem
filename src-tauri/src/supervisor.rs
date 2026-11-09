//! The supervisor: a background loop that keeps the running engine matching the settings
//! and the devices actually connected.
//!
//! - Starts the engine on the source (the Windows default output, or a pinned device),
//!   restarts it when the default changes or the source comes back after a disconnect.
//! - Adds, removes and retries outputs as they are enabled, plugged in or lost.
//! - Applies delay and gain changes live.
//! - Publishes a status snapshot to the UI and the tray.
//! - Optionally re-runs microphone auto-sync after a reconnect.

use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

use tandem_engine::devices::{self, mix_format};
use tandem_engine::engine::{Engine, OutputConfig, SourceConfig};
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::DeviceRef;
use crate::state::{
    AppState, DeviceDto, Devices, OutputState, OutputStatusDto, Snapshot, SourceState, SourceStatus,
};

const TICK: Duration = Duration::from_millis(250);
const DEVICE_SCAN_EVERY: Duration = Duration::from_secs(1);
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// Let a fresh connection settle before auto-syncing it.
const RESYNC_SETTLE: Duration = Duration::from_secs(4);
/// Connections in the first moments after launch are not "reconnects".
const STARTUP_GRACE: Duration = Duration::from_secs(10);

pub fn spawn(app: AppHandle) {
    thread::Builder::new()
        .name("supervisor".into())
        .spawn(move || {
            let _ = wasapi::initialize_mta();
            let mut sup = Supervisor::new();
            loop {
                sup.tick(&app);
                thread::sleep(TICK);
            }
        })
        .expect("spawn supervisor");
}

/// Find a remembered device among the connected ones: by id first, then by name.
pub fn resolve<'a>(connected: &'a [DeviceDto], wanted: &DeviceRef) -> Option<&'a DeviceDto> {
    connected
        .iter()
        .find(|d| d.id == wanted.id)
        .or_else(|| connected.iter().find(|d| d.name == wanted.name))
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
        }
    }

    fn scan_devices(&mut self, state: &AppState) {
        let now = Instant::now();
        if self.last_scan.is_some_and(|t| now - t < DEVICE_SCAN_EVERY) {
            return;
        }
        self.last_scan = Some(now);
        let outputs = devices::render_devices().unwrap_or_default();
        let inputs = devices::capture_devices().unwrap_or_default();
        *state.devices.lock().unwrap() = Devices {
            outputs: outputs.into_iter().map(Into::into).collect(),
            inputs: inputs.into_iter().map(Into::into).collect(),
        };
    }

    fn tick(&mut self, app: &AppHandle) {
        let state = app.state::<AppState>();
        self.scan_devices(&state);
        let now = Instant::now();
        let settings = state.settings();
        let devices = state.devices.lock().unwrap().clone();
        let calibrating = state.calibrating.lock().unwrap().clone();
        let after_startup = now - self.started_at > STARTUP_GRACE;
        let can_resync = settings.resync_on_reconnect
            && settings.mic.as_ref().is_some_and(|m| resolve(&devices.inputs, m).is_some());

        let mut rt = state.runtime.lock().unwrap();

        // ---- Source ----------------------------------------------------------------
        let source_dev = if settings.source.follow_default {
            devices.outputs.iter().find(|d| d.is_default)
        } else {
            settings.source.device.as_ref().and_then(|r| resolve(&devices.outputs, r))
        };
        let mut source = SourceStatus::default();

        if !settings.enabled {
            if let Some(engine) = rt.engine.take() {
                engine.stop();
            }
            rt.source_id = None;
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
            if stale && let Some(engine) = rt.engine.take() {
                engine.stop();
            }
            if rt.engine.is_none() && self.source_retry_at.is_none_or(|t| now >= t) {
                let started = mix_format(&dev.id).and_then(|fmt| {
                    Engine::start(SourceConfig { device_id: dev.id.clone(), fmt })
                });
                match started {
                    Ok(engine) => {
                        engine.set_clicks(rt.clicks);
                        rt.engine = Some(engine);
                        rt.source_id = Some(dev.id.clone());
                        self.source_error = None;
                        self.source_retry_at = None;
                        self.output_retry_at.clear();
                        // A (re)connected source changes the path latency: re-sync everything.
                        if after_startup && can_resync {
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
                    source.state = SourceState::Running;
                    source.level = e.source_level();
                }
                _ => {
                    source.state = SourceState::Error;
                    source.error = self.source_error.clone();
                }
            }
        } else {
            if let Some(engine) = rt.engine.take() {
                engine.stop();
            }
            rt.source_id = None;
            source.state = SourceState::Waiting;
            source.device_name = settings.source.device.as_ref().map(|d| d.name.clone());
        }

        // ---- Outputs -----------------------------------------------------------------
        let source_id = rt.source_id.clone();
        let current_source = source_dev.map(|d| d.id.clone());
        let mut id_updates: Vec<(String, DeviceRef)> = Vec::new();
        let mut outputs = Vec::new();
        let wanted_keys: HashSet<&str> = settings.outputs.iter().map(|o| o.device.id.as_str()).collect();

        if let Some(engine) = rt.engine.as_mut() {
            let unknown: Vec<String> = engine
                .outputs()
                .iter()
                .map(|o| o.key().to_string())
                .filter(|k| !wanted_keys.contains(k.as_str()))
                .collect();
            for key in unknown {
                engine.remove_output(&key);
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
            let present = resolve(&devices.outputs, &o.device);
            if let Some(dev) = present
                && dev.id != o.device.id
            {
                // The device came back under a new id (e.g. Bluetooth re-pairing): adopt it.
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
                    engine.remove_output(&key);
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
                engine.remove_output(&key);
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
                        if after_startup && can_resync {
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
                    handle.set_delay_ms(delay_ms);
                    handle.set_gain_db(o.gain_db);
                    let st = handle.status();
                    dto.state = if st.running { OutputState::Playing } else { OutputState::Buffering };
                    dto.buffered_ms = st.buffered_ms;
                    dto.correction_ppm = st.correction_ppm;
                    dto.underruns = st.underruns;
                    dto.level = st.level;
                }
                None => {
                    dto.state = OutputState::Error;
                    dto.error = self.output_errors.get(&key).cloned();
                }
            }
            outputs.push(dto);
        }
        let clicks = rt.clicks;
        drop(rt);

        if !id_updates.is_empty() {
            state.update_settings(|s| {
                for (old, new) in id_updates {
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
