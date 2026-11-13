//! Keeping other programs quiet during a measurement, and always giving them their sound
//! back.
//!
//! A measurement started by hand mutes the other programs playing on the devices it uses
//! (`tandem_engine::sessions`). Windows remembers a program's mute across restarts, so a
//! crash between "mute" and "unmute" would leave a program silent for good. The rule that
//! prevents it:
//!
//! 1. **Write down** which sessions are about to be muted, in `muted-sessions.json` next to
//!    the settings, *before* muting them.
//! 2. Mute.
//! 3. On every exit path, unmute and strike them from the file.
//! 4. At launch, and now and then while the file exists, unmute whatever is still listed.
//!    A program that is not running then stays listed until it is seen again: Windows will
//!    hand it back its old mute when it restarts, and that is when it can be lifted.
//!
//! The file is only read and written under [`AppState::quiet_lock`], and recovery does
//! nothing while a measurement runs, so it never unmutes what a measurement just muted.
//!
//! The Windows calls sit behind [`Mixer`] so the bookkeeping can be tested without touching
//! a real mixer.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tandem_engine::sessions::{self, SessionRef};

use crate::state::AppState;

/// The part of the Windows mixer this module needs.
pub trait Mixer {
    /// Sessions a measurement on this device would silence. Changes nothing.
    fn candidates(&self, device_id: &str) -> Vec<SessionRef>;
    fn mute(&self, sessions: &[SessionRef]);
    /// Returns the sessions that could not be reached.
    fn unmute(&self, sessions: &[SessionRef]) -> Vec<SessionRef>;
}

/// The real mixer. Needs COM initialised on the calling thread.
pub struct Windows;

impl Mixer for Windows {
    fn candidates(&self, device_id: &str) -> Vec<SessionRef> {
        // A device that cannot be asked (unplugged, an id that just changed) has nothing we
        // can mute: the measurement goes on without it.
        sessions::candidates(device_id).unwrap_or_default()
    }

    fn mute(&self, sessions: &[SessionRef]) {
        sessions::mute(sessions);
    }

    fn unmute(&self, sessions: &[SessionRef]) -> Vec<SessionRef> {
        sessions::unmute(sessions)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    device_id: String,
    instance_id: String,
    session_id: String,
    process_id: u32,
}

impl From<&SessionRef> for Entry {
    fn from(s: &SessionRef) -> Self {
        Self {
            device_id: s.device_id.clone(),
            instance_id: s.instance_id.clone(),
            session_id: s.session_id.clone(),
            process_id: s.process_id,
        }
    }
}

impl From<Entry> for SessionRef {
    fn from(e: Entry) -> Self {
        Self {
            device_id: e.device_id,
            instance_id: e.instance_id,
            session_id: e.session_id,
            process_id: e.process_id,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Journal {
    sessions: Vec<Entry>,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("muted-sessions.json")
}

/// True when some program may still be muted by Tandem.
pub fn pending(dir: &Path) -> bool {
    path(dir).exists()
}

fn load(dir: &Path) -> Vec<SessionRef> {
    let file = path(dir);
    let Ok(text) = fs::read_to_string(&file) else {
        return Vec::new();
    };
    match serde_json::from_str::<Journal>(&text) {
        Ok(journal) => journal.sessions.into_iter().map(Into::into).collect(),
        Err(_) => {
            // Unusable as it is: keep it for a human to read, and stop tripping over it.
            let _ = fs::rename(&file, dir.join("muted-sessions.unreadable.json"));
            Vec::new()
        }
    }
}

/// Write the list atomically; no file at all when the list is empty.
fn save(dir: &Path, sessions: &[SessionRef]) -> std::io::Result<()> {
    let file = path(dir);
    if sessions.is_empty() {
        return match fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    fs::create_dir_all(dir)?;
    let journal = Journal {
        sessions: sessions.iter().map(Entry::from).collect(),
    };
    let tmp = dir.join("muted-sessions.json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(&journal).map_err(std::io::Error::other)?,
    )?;
    fs::rename(tmp, file)
}

fn same(a: &SessionRef, b: &SessionRef) -> bool {
    a.device_id == b.device_id && a.instance_id == b.instance_id
}

/// Mute the other programs playing on `devices`, recording them first. `already` are the
/// ones this measurement muted earlier; the return value is what this call added.
///
/// If the record cannot be written, nothing is muted: a mute that could not be undone after
/// a crash is worse than a measurement with music in it.
pub fn silence(
    dir: &Path,
    mixer: &impl Mixer,
    devices: &[String],
    already: &[SessionRef],
) -> Vec<SessionRef> {
    let mut fresh: Vec<SessionRef> = Vec::new();
    for device in devices {
        for candidate in mixer.candidates(device) {
            if !already.iter().chain(&fresh).any(|s| same(s, &candidate)) {
                fresh.push(candidate);
            }
        }
    }
    if fresh.is_empty() {
        return fresh;
    }
    let mut listed = load(dir);
    for session in &fresh {
        if !listed.iter().any(|s| same(s, session)) {
            listed.push(session.clone());
        }
    }
    if save(dir, &listed).is_err() {
        return Vec::new();
    }
    mixer.mute(&fresh);
    fresh
}

/// Unmute what a measurement muted and strike it from the record. Sessions that could not
/// be reached stay listed for [`recover`].
pub fn restore(dir: &Path, mixer: &impl Mixer, muted: &[SessionRef]) {
    if muted.is_empty() {
        return;
    }
    let unreached = mixer.unmute(muted);
    let listed: Vec<SessionRef> = load(dir)
        .into_iter()
        .filter(|s| !muted.iter().any(|m| same(m, s)) || unreached.iter().any(|u| same(u, s)))
        .collect();
    let _ = save(dir, &listed);
}

/// Unmute everything still on record (after a crash, or for a program that was not running
/// last time). Returns how many remain out of reach.
pub fn recover(dir: &Path, mixer: &impl Mixer) -> usize {
    let listed = load(dir);
    if listed.is_empty() {
        let _ = save(dir, &listed);
        return 0;
    }
    let unreached = mixer.unmute(&listed);
    let _ = save(dir, &unreached);
    unreached.len()
}

/// Mutes other programs for one measurement and unmutes them when dropped, whatever the
/// way out.
pub struct Quiet<'a> {
    state: &'a AppState,
    muted: Vec<SessionRef>,
}

impl<'a> Quiet<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self {
            state,
            muted: Vec::new(),
        }
    }

    /// Mute the other programs now playing on these devices. Call it again later in the
    /// measurement to catch a program that started in between.
    pub fn silence(&mut self, devices: &[String]) {
        let _journal = self.state.quiet_lock.lock().unwrap();
        let fresh = silence(&self.state.config_dir, &Windows, devices, &self.muted);
        self.muted.extend(fresh);
    }
}

impl Drop for Quiet<'_> {
    fn drop(&mut self) {
        let _journal = self
            .state
            .quiet_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        restore(&self.state.config_dir, &Windows, &self.muted);
    }
}

/// Lift mutes left over from an earlier run. Does nothing while a measurement is running:
/// its mutes are its own to lift.
pub fn recover_leftovers(state: &AppState) {
    let _journal = state.quiet_lock.lock().unwrap();
    if state.calibrating.lock().unwrap().is_some() {
        return;
    }
    recover(&state.config_dir, &Windows);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A pretend mixer: sessions by instance id, each (lasting id, playing, muted).
    struct Fake {
        dir: PathBuf,
        sessions: RefCell<BTreeMap<String, (String, bool, bool)>>,
        /// What the record held at the moment of each mute.
        listed_at_mute: RefCell<Vec<usize>>,
    }

    impl Fake {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("tandem-quiet-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Self {
                dir,
                sessions: RefCell::default(),
                listed_at_mute: RefCell::default(),
            }
        }

        fn play(&self, instance: &str, lasting: &str) {
            self.sessions
                .borrow_mut()
                .insert(instance.into(), (lasting.into(), true, false));
        }

        fn quit(&self, instance: &str) {
            self.sessions.borrow_mut().remove(instance);
        }

        fn muted(&self, instance: &str) -> bool {
            self.sessions.borrow()[instance].2
        }

        fn reference(instance: &str, lasting: &str) -> SessionRef {
            SessionRef {
                device_id: "dev".into(),
                instance_id: instance.into(),
                session_id: lasting.into(),
                process_id: 1,
            }
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    impl Mixer for Fake {
        fn candidates(&self, _device: &str) -> Vec<SessionRef> {
            self.sessions
                .borrow()
                .iter()
                .filter(|(_, (_, playing, muted))| *playing && !*muted)
                .map(|(instance, (lasting, ..))| Fake::reference(instance, lasting))
                .collect()
        }

        fn mute(&self, sessions: &[SessionRef]) {
            self.listed_at_mute.borrow_mut().push(load(&self.dir).len());
            for s in sessions {
                if let Some(entry) = self.sessions.borrow_mut().get_mut(&s.instance_id) {
                    entry.2 = true;
                }
            }
        }

        fn unmute(&self, sessions: &[SessionRef]) -> Vec<SessionRef> {
            let mut live = self.sessions.borrow_mut();
            let mut unreached = Vec::new();
            for s in sessions {
                // Itself, else the same program started again (same lasting id).
                let key = if live.contains_key(&s.instance_id) {
                    Some(s.instance_id.clone())
                } else {
                    live.iter()
                        .find(|(_, (lasting, ..))| *lasting == s.session_id)
                        .map(|(k, _)| k.clone())
                };
                match key {
                    Some(k) => live.get_mut(&k).unwrap().2 = false,
                    None => unreached.push(s.clone()),
                }
            }
            unreached
        }
    }

    /// Nothing muted earlier in this measurement.
    const NONE: &[SessionRef] = &[];

    fn devices() -> Vec<String> {
        vec!["dev".to_string()]
    }

    #[test]
    fn the_record_is_written_before_anything_is_muted() {
        let mixer = Fake::new("order");
        mixer.play("player|1", "player");
        mixer.play("browser|2", "browser");
        let muted = silence(&mixer.dir, &mixer, &devices(), NONE);
        assert_eq!(muted.len(), 2);
        // At the moment of the mute, both were already on record.
        assert_eq!(*mixer.listed_at_mute.borrow(), [2]);
        assert!(mixer.muted("player|1") && mixer.muted("browser|2"));
        assert!(pending(&mixer.dir));
    }

    #[test]
    fn a_normal_end_unmutes_everything_and_leaves_no_record() {
        let mixer = Fake::new("normal");
        mixer.play("player|1", "player");
        let muted = silence(&mixer.dir, &mixer, &devices(), NONE);
        restore(&mixer.dir, &mixer, &muted);
        assert!(!mixer.muted("player|1"));
        assert!(!pending(&mixer.dir));
    }

    #[test]
    fn nothing_playing_means_nothing_muted_and_no_record() {
        let mixer = Fake::new("idle");
        assert!(silence(&mixer.dir, &mixer, &devices(), NONE).is_empty());
        assert!(mixer.listed_at_mute.borrow().is_empty());
        assert!(!pending(&mixer.dir));
    }

    #[test]
    fn a_program_that_starts_mid_measurement_is_caught_by_the_second_look() {
        let mixer = Fake::new("second");
        mixer.play("player|1", "player");
        let mut muted = silence(&mixer.dir, &mixer, &devices(), NONE);
        mixer.play("game|5", "game");
        let later = silence(&mixer.dir, &mixer, &devices(), &muted);
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].instance_id, "game|5");
        muted.extend(later);
        assert_eq!(load(&mixer.dir).len(), 2);
        restore(&mixer.dir, &mixer, &muted);
        assert!(!mixer.muted("player|1") && !mixer.muted("game|5"));
        assert!(!pending(&mixer.dir));
    }

    #[test]
    fn after_a_crash_the_next_launch_gives_the_sound_back() {
        let mixer = Fake::new("crash");
        mixer.play("player|1", "player");
        silence(&mixer.dir, &mixer, &devices(), NONE);
        // Tandem dies here: no restore. The mute and the record both survive.
        assert!(mixer.muted("player|1") && pending(&mixer.dir));
        assert_eq!(recover(&mixer.dir, &mixer), 0);
        assert!(!mixer.muted("player|1"));
        assert!(!pending(&mixer.dir));
    }

    #[test]
    fn a_program_that_quit_while_muted_is_unmuted_when_it_comes_back() {
        let mixer = Fake::new("quit");
        mixer.play("player|1", "player");
        let muted = silence(&mixer.dir, &mixer, &devices(), NONE);
        mixer.quit("player|1");
        // The measurement ends: the program cannot be reached, so it stays on record.
        restore(&mixer.dir, &mixer, &muted);
        assert!(pending(&mixer.dir));
        assert_eq!(recover(&mixer.dir, &mixer), 1);
        // It starts again; Windows gives it back the mute it remembered.
        mixer
            .sessions
            .borrow_mut()
            .insert("player|9".into(), ("player".into(), true, true));
        assert_eq!(recover(&mixer.dir, &mixer), 0);
        assert!(!mixer.muted("player|9"));
        assert!(!pending(&mixer.dir));
    }

    #[test]
    fn one_measurement_ending_leaves_another_record_entry_alone() {
        // Left over from a crash: an entry this measurement did not make.
        let mixer = Fake::new("mixed");
        save(&mixer.dir, &[Fake::reference("old|3", "old")]).unwrap();
        mixer.play("player|1", "player");
        let muted = silence(&mixer.dir, &mixer, &devices(), NONE);
        restore(&mixer.dir, &mixer, &muted);
        let left = load(&mixer.dir);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].instance_id, "old|3");
    }

    #[test]
    fn if_the_record_cannot_be_written_nothing_is_muted() {
        let mixer = Fake::new("readonly");
        mixer.play("player|1", "player");
        // A file where the directory should be: the record cannot be created.
        fs::write(&mixer.dir, b"not a directory").unwrap();
        assert!(silence(&mixer.dir, &mixer, &devices(), NONE).is_empty());
        assert!(!mixer.muted("player|1"));
        let _ = fs::remove_file(&mixer.dir);
    }

    #[test]
    fn an_unreadable_record_is_kept_aside_and_read_as_empty() {
        let mixer = Fake::new("garbled");
        fs::create_dir_all(&mixer.dir).unwrap();
        fs::write(path(&mixer.dir), "{ not json").unwrap();
        assert_eq!(recover(&mixer.dir, &mixer), 0);
        assert!(!pending(&mixer.dir));
        assert!(mixer.dir.join("muted-sessions.unreadable.json").exists());
    }
}
