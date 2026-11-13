//! Persisted user settings: a JSON file in the app config directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_DELAY_MS: u32 = 200;
/// Format version written to the settings file. Bump it with a migration in
/// [`Settings::load`] whenever the meaning of an existing field changes.
pub const SETTINGS_VERSION: u32 = 1;
/// The steps of the first-run guide are numbered from 0 to this.
pub const LAST_GUIDE_STEP: u8 = 4;
/// What `guide_step` reads as in a file written before the guide existed.
const GUIDE_NOT_RECORDED: u8 = u8::MAX;

fn guide_not_recorded() -> Option<u8> {
    Some(GUIDE_NOT_RECORDED)
}

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
        Self {
            follow_default: true,
            device: None,
        }
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
    /// Format version of the file. Files written before versions existed read as 0.
    #[serde(default)]
    pub version: u32,
    /// Master switch.
    pub enabled: bool,
    pub source: SourceSetting,
    pub outputs: Vec<OutputSetting>,
    /// Microphone used for auto-sync.
    pub mic: Option<DeviceRef>,
    pub resync_on_reconnect: bool,
    /// Auto-sync against this output (its device id) instead of the source: for a source
    /// nobody listens to, which the microphone cannot hear.
    pub sync_reference: Option<String>,
    /// Apply the source's hardware volume and mute to the outputs, so the Windows volume
    /// keys act on every speaker.
    pub follow_source_volume: bool,
    /// Close every stream after a while of silence so Windows can sleep.
    pub pause_when_silent: bool,
    /// Look for a new version shortly after launch and a few times a day.
    pub auto_check_updates: bool,
    /// A version the user chose to skip: it stays quiet until a newer one ships.
    pub skipped_update: Option<String>,
    /// "system" | "light" | "dark"
    pub theme: String,
    /// "en" | "fr"
    pub locale: String,
    pub close_to_tray: bool,
    pub start_minimized: bool,
    /// The first-run guide: the step it is at while it is pending, `None` once it has been
    /// finished or skipped. A new installation starts at step 0. A file written before the
    /// guide existed says nothing: it is read as finished if it already has outputs (someone
    /// set Tandem up without it) and as pending otherwise (see [`Settings::load`]).
    #[serde(default = "guide_not_recorded")]
    pub guide_step: Option<u8>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            enabled: true,
            source: SourceSetting::default(),
            outputs: Vec::new(),
            mic: None,
            resync_on_reconnect: false,
            sync_reference: None,
            follow_source_volume: true,
            pause_when_silent: false,
            auto_check_updates: true,
            skipped_update: None,
            theme: "system".into(),
            locale: default_locale(),
            close_to_tray: true,
            start_minimized: true,
            guide_step: Some(0),
        }
    }
}

fn default_locale() -> String {
    let lang = std::env::var("LANG").unwrap_or_default().to_lowercase();
    if lang.starts_with("fr") || sys_locale_is_french() {
        "fr".into()
    } else {
        "en".into()
    }
}

/// Whether Windows itself is displayed in French, for the language of a first launch. The
/// standard library has no way to ask, hence the direct call.
#[cfg(windows)]
#[allow(unsafe_code)]
fn sys_locale_is_french() -> bool {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    // SAFETY: the function takes no argument, has no precondition and cannot fail: it
    // returns the language id of the current user. It is `unsafe` only because the bindings
    // mark every Windows function so.
    let language = unsafe { GetUserDefaultUILanguage() };
    // Primary language id 0x0C = French.
    (language & 0x3ff) == 0x0c
}

#[cfg(not(windows))]
fn sys_locale_is_french() -> bool {
    false
}

impl Settings {
    fn path(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    /// Load the settings, or the defaults when there is no file yet. A file that cannot be
    /// used as it is never silently lost: the next save would overwrite it, so a copy is kept
    /// next to it first (`settings.unreadable.json`, or `settings.v<N>.json` for a file from
    /// a newer version of the app, whose extra fields this version would drop).
    pub fn load(dir: &Path) -> Self {
        let path = Self::path(dir);
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(mut settings) => {
                if settings.version > SETTINGS_VERSION {
                    let _ = fs::copy(
                        &path,
                        dir.join(format!("settings.v{}.json", settings.version)),
                    );
                }
                // Versions 0 and 1 share one layout: nothing to migrate yet.
                settings.version = SETTINGS_VERSION;
                settings.settle_guide();
                settings
            }
            Err(_) => {
                let _ = fs::copy(&path, dir.join("settings.unreadable.json"));
                Self::default()
            }
        }
    }

    /// Write atomically: a temp file then a rename, so a crash never leaves half a file.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join("settings.json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        fs::rename(tmp, Self::path(dir))
    }

    /// True when closing the window should count as finishing the guide: it is pending and
    /// at least one output is set up.
    pub fn guide_done_by_closing(&self) -> bool {
        self.guide_step.is_some() && !self.outputs.is_empty()
    }

    /// Decide what a file that predates the guide means for it, and keep the step in range.
    /// Only the "not recorded" case looks at the outputs: a guide that is recorded as pending
    /// stays pending even once outputs exist, so that leaving it halfway and coming back
    /// resumes it.
    fn settle_guide(&mut self) {
        self.guide_step = match self.guide_step {
            Some(GUIDE_NOT_RECORDED) if self.outputs.is_empty() => Some(0),
            Some(GUIDE_NOT_RECORDED) => None,
            Some(step) => Some(step.min(LAST_GUIDE_STEP)),
            None => None,
        };
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
        let s: Settings = serde_json::from_str(
            r#"{"enabled":false,"outputs":[{"device":{"id":"a","name":"A"}}]}"#,
        )
        .unwrap();
        assert!(!s.enabled);
        assert_eq!(s.outputs[0].default_delay_ms, DEFAULT_DELAY_MS);
        assert!(s.close_to_tray);
        assert_eq!(s.version, 0);
        assert!(s.follow_source_volume);
        assert!(!s.pause_when_silent);
        assert!(s.auto_check_updates);
    }

    #[test]
    fn an_unreadable_file_is_kept_aside() {
        let dir = std::env::temp_dir().join(format!("tandem-test-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), "{ not json").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
        assert_eq!(
            fs::read_to_string(dir.join("settings.unreadable.json")).unwrap(),
            "{ not json"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_from_a_newer_version_is_kept_aside() {
        let dir = std::env::temp_dir().join(format!("tandem-test-new-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("settings.json"),
            r#"{"version":99,"enabled":false}"#,
        )
        .unwrap();
        let s = Settings::load(&dir);
        assert!(!s.enabled);
        assert_eq!(s.version, SETTINGS_VERSION);
        assert!(dir.join("settings.v99.json").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_first_launch_picks_one_of_the_two_languages() {
        // Runs the Windows call behind `sys_locale_is_french`.
        let _ = sys_locale_is_french();
        assert!(["en", "fr"].contains(&default_locale().as_str()));
        assert!(["en", "fr"].contains(&Settings::default().locale.as_str()));
    }

    fn load_text(name: &str, text: &str) -> Settings {
        let dir = std::env::temp_dir().join(format!("tandem-test-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), text).unwrap();
        let s = Settings::load(&dir);
        let _ = fs::remove_dir_all(dir);
        s
    }

    #[test]
    fn closing_the_window_finishes_the_guide_only_once_an_output_exists() {
        let mut s = Settings::default();
        assert!(!s.guide_done_by_closing());
        s.outputs.push(OutputSetting::default());
        assert!(s.guide_done_by_closing());
        s.guide_step = None;
        assert!(!s.guide_done_by_closing());
    }

    #[test]
    fn a_new_installation_starts_the_guide() {
        let dir = std::env::temp_dir().join(format!("tandem-test-fresh-{}", std::process::id()));
        assert_eq!(Settings::load(&dir).guide_step, Some(0));
    }

    #[test]
    fn an_installation_set_up_before_the_guide_existed_never_sees_it() {
        let s = load_text(
            "guide-old",
            r#"{"outputs":[{"device":{"id":"a","name":"A"}}]}"#,
        );
        assert_eq!(s.guide_step, None);
    }

    #[test]
    fn an_older_installation_with_nothing_set_up_gets_the_guide() {
        assert_eq!(
            load_text("guide-empty", r#"{"outputs":[]}"#).guide_step,
            Some(0)
        );
    }

    #[test]
    fn a_guide_left_halfway_resumes_where_it_was_even_with_outputs() {
        let s = load_text(
            "guide-resume",
            r#"{"guideStep":3,"outputs":[{"device":{"id":"a","name":"A"}}]}"#,
        );
        assert_eq!(s.guide_step, Some(3));
    }

    #[test]
    fn a_finished_guide_stays_finished_and_a_wild_step_is_brought_back_in_range() {
        assert_eq!(
            load_text("guide-done", r#"{"guideStep":null}"#).guide_step,
            None
        );
        assert_eq!(
            load_text("guide-wild", r#"{"guideStep":99}"#).guide_step,
            Some(LAST_GUIDE_STEP)
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("tandem-test-{}", std::process::id()));
        let mut s = Settings::default();
        s.outputs.push(OutputSetting {
            gain_db: -3.0,
            ..Default::default()
        });
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir), s);
        let _ = fs::remove_dir_all(dir);
    }
}
