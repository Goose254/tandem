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
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub enabled: bool,
    pub source: SourceStatus,
    pub outputs: Vec<OutputStatusDto>,
    pub clicks: bool,
    pub calibrating: Option<String>,
}

impl Snapshot {
    /// The tray's coarse state: active when audio flows to at least one output.
    pub fn is_active(&self) -> bool {
        self.source.state == SourceState::Running
            && self.outputs.iter().any(|o| o.state == OutputState::Playing)
    }
}
