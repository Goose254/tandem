//! Persisted user settings: a JSON file in the app config directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_DELAY_MS: u32 = 200;

/// A device remembered by id, with its name as a fallback: Windows sometimes re-creates a
/// Bluetooth endpoint with a new id after re-pairing, while the name stays the same.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SourceSetting {
    /// Follow the Windows default output (recommended: make the slowest device the default).
    pub follow_default: bool,
    /// The pinned device when not following the default.
    pub device: Option<DeviceRef>,
}

impl Default for SourceSetting {
    fn default() -> Self {
        Self { follow_default: true, device: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct OutputSetting {
    pub device: DeviceRef,
    pub enabled: bool,
    pub gain_db: f32,
    /// Delay to use with a source this output has never been synced against.
    pub default_delay_ms: u32,
    /// Delay per source device id: the right delay depends on which device is the source.
    pub delays: BTreeMap<String, u32>,
}

impl Default for OutputSetting {
    fn default() -> Self {
        Self {
            device: DeviceRef::default(),
            enabled: true,
            gain_db: 0.0,
            default_delay_ms: DEFAULT_DELAY_MS,
            delays: BTreeMap::new(),
        }
    }
}

impl OutputSetting {
    pub fn delay_for(&self, source_id: Option<&str>) -> u32 {
        source_id
            .and_then(|id| self.delays.get(id).copied())
            .unwrap_or(self.default_delay_ms)
    }

    pub fn set_delay_for(&mut self, source_id: Option<&str>, ms: u32) {
        match source_id {
            Some(id) => {
                self.delays.insert(id.to_string(), ms);
            }
            None => self.default_delay_ms = ms,
        }
        // A fresh source starts from the most recent value, which is usually close.
        self.default_delay_ms = ms;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Master switch.
    pub enabled: bool,
    pub source: SourceSetting,
    pub outputs: Vec<OutputSetting>,
    /// Microphone used for auto-sync.
    pub mic: Option<DeviceRef>,
    pub resync_on_reconnect: bool,
    /// "system" | "light" | "dark"
    pub theme: String,
    /// "en" | "fr"
    pub locale: String,
    pub close_to_tray: bool,
    pub start_minimized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            source: SourceSetting::default(),
            outputs: Vec::new(),
            mic: None,
            resync_on_reconnect: false,
            theme: "system".into(),
            locale: default_locale(),
            close_to_tray: true,
            start_minimized: true,
        }
    }
}

fn default_locale() -> String {
    let lang = std::env::var("LANG").unwrap_or_default().to_lowercase();
    if lang.starts_with("fr") || sys_locale_is_french() { "fr".into() } else { "en".into() }
}

#[cfg(windows)]
fn sys_locale_is_french() -> bool {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    // Primary language id 0x0C = French.
    (unsafe { GetUserDefaultUILanguage() } & 0x3ff) == 0x0c
}

#[cfg(not(windows))]
fn sys_locale_is_french() -> bool {
    false
}

impl Settings {
    fn path(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Write atomically: a temp file then a rename, so a crash never leaves half a file.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join("settings.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        fs::rename(tmp, Self::path(dir))
    }

    pub fn output_mut(&mut self, id: &str) -> Option<&mut OutputSetting> {
        self.outputs.iter_mut().find(|o| o.device.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_are_remembered_per_source() {
        let mut o = OutputSetting::default();
        o.set_delay_for(Some("bt"), 240);
        o.set_delay_for(Some("hdmi"), 60);
        assert_eq!(o.delay_for(Some("bt")), 240);
        assert_eq!(o.delay_for(Some("hdmi")), 60);
        // Unknown source starts from the latest value.
        assert_eq!(o.delay_for(Some("new")), 60);
    }

    #[test]
    fn old_or_partial_files_still_load() {
        let s: Settings = serde_json::from_str(r#"{"enabled":false,"outputs":[{"device":{"id":"a","name":"A"}}]}"#).unwrap();
        assert!(!s.enabled);
        assert_eq!(s.outputs[0].default_delay_ms, DEFAULT_DELAY_MS);
        assert!(s.close_to_tray);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("tandem-test-{}", std::process::id()));
        let mut s = Settings::default();
        s.outputs.push(OutputSetting { gain_db: -3.0, ..Default::default() });
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir), s);
        let _ = fs::remove_dir_all(dir);
    }
}
