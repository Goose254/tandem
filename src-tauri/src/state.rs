//! Shared application state and the status snapshot sent to the UI.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tandem_engine::devices::DeviceInfo;
use tandem_engine::engine::Engine;

use crate::settings::Settings;

pub struct AppState {
    pub config_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub runtime: Mutex<Runtime>,
    pub devices: Mutex<Devices>,
    pub status: Mutex<Snapshot>,
    /// Device id of the output being auto-synced, if any.
    pub calibrating: Mutex<Option<String>>,
    /// Version of an update the main window found, so the tray popup can mention it.
    pub update_notice: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(config_dir: PathBuf) -> Self {
        let settings = Settings::load(&config_dir);
        Self {
            config_dir,
            settings: Mutex::new(settings),
            runtime: Mutex::new(Runtime::default()),
            devices: Mutex::new(Devices::default()),
            status: Mutex::new(Snapshot::default()),
            calibrating: Mutex::new(None),
            update_notice: Mutex::new(None),
        }
    }

    /// Mutate the settings and persist them.
    pub fn update_settings<R>(&self, f: impl FnOnce(&mut Settings) -> R) -> R {
        let mut s = self.settings.lock().unwrap();
        let r = f(&mut s);
        if let Err(e) = s.save(&self.config_dir) {
            eprintln!("could not save settings: {e}");
        }
        r
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Devices {
    pub outputs: Vec<DeviceDto>,
    pub inputs: Vec<DeviceDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDto {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

impl From<DeviceInfo> for DeviceDto {
    fn from(d: DeviceInfo) -> Self {
        Self { id: d.id, name: d.name, is_default: d.is_default }
    }
}

/// Live engine objects, owned by the supervisor.
#[derive(Default)]
pub struct Runtime {
    pub engine: Option<Engine>,
    /// Id of the device the running engine captures.
    pub source_id: Option<String>,
    /// Calibration clicks, re-applied when the engine restarts.
    pub clicks: bool,
}

/// A coarse reason for a device error, which the UI translates. The raw message stays
/// available next to it for bug reports.
pub fn error_kind(message: &str) -> &'static str {
    let m = message.to_lowercase();
    if m.contains("8889000a") {
        // AUDCLNT_E_DEVICE_IN_USE: another app holds the device in exclusive mode.
        "inUse"
    } else if m.contains("88890004") {
        // AUDCLNT_E_DEVICE_INVALIDATED: unplugged, or its format was changed in Windows.
        "invalidated"
    } else if m.contains("stopped responding") {
        "stalled"
    } else {
        "other"
    }
}

/// Meter levels, sent more often than the full snapshot so the meters move smoothly.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Levels {
    pub source: f32,
    /// By output device id.
    pub outputs: std::collections::HashMap<String, f32>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceState {
    /// Master switch off.
    #[default]
    Off,
    /// Capturing.
    Running,
    /// The source device is not connected.
    Waiting,
    /// The source failed to open; retrying.
    Error,
    /// Nothing plays: streams are closed until the source carries sound again.
    Paused,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OutputState {
    #[default]
    Disabled,
    /// Filling its buffer up to the delay.
    Buffering,
    /// In sync.
    Playing,
    /// Device not connected.
    Waiting,
    /// This device is currently the source, so it cannot also be an output.
    IsSource,
    /// Mirroring is off, or the source is unavailable.
    Idle,
    Error,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub state: SourceState,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub level: f32,
    pub error: Option<String>,
    /// See [`error_kind`].
    pub error_kind: Option<&'static str>,
    /// The source device's sample rate while capturing.
    pub sample_rate: Option<u32>,
    /// The source applies its volume itself, so "follow the source volume" matters for it.
    pub hardware_volume: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputStatusDto {
    pub device_id: String,
    pub state: OutputState,
    pub delay_ms: u32,
    pub buffered_ms: f64,
    pub correction_ppm: f64,
    pub underruns: u64,
    pub level: f32,
    pub error: Option<String>,
    /// See [`error_kind`].
    pub error_kind: Option<&'static str>,
    /// How far behind the source this output plays at a delay of 0 ms (its minimum path).
    pub min_lag_ms: Option<f64>,
    /// The output device's sample rate while it plays.
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub enabled: bool,
    pub source: SourceStatus,
    pub outputs: Vec<OutputStatusDto>,
    pub clicks: bool,
    pub calibrating: Option<String>,
    /// Version of an available update, if one was found.
    pub update: Option<String>,
}

impl Snapshot {
    /// The tray's coarse state: active when audio flows to at least one output.
    pub fn is_active(&self) -> bool {
        self.source.state == SourceState::Running
            && self.outputs.iter().any(|o| o.state == OutputState::Playing)
    }
}
