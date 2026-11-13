//! Silencing the other programs that play on a device, for the length of a measurement.
//!
//! Windows mixes every program's sound into a device through *audio sessions*, each with its
//! own volume and mute (what the Windows volume mixer shows). Muting the other sessions on a
//! device is the only way to keep their sound out of a measurement: it leaves both the
//! speaker and the loopback capture.
//!
//! This writes to the Windows mixer, which nothing else in Tandem does, and Windows **keeps
//! a session's mute across restarts**. So the rule is: record what is about to be muted
//! somewhere that survives a crash, mute, and unmute on every exit path. The functions here
//! do the Windows part; the caller owns the record (see `src-tauri/src/quiet.rs`).
//!
//! What is left alone: Tandem's own sessions, the system-sounds session (Microsoft asks
//! applications not to control it), sessions that are not playing, and sessions the user
//! had already muted (they are not ours to unmute afterwards).
//!
//! A note on the API. Microsoft documents two ways to get a session's `ISimpleAudioVolume`:
//! from one's own stream, or by session GUID within one's own process. Neither reaches
//! another program's session. Asking the session control returned by the session enumerator
//! for that interface does, and is what volume mixers do; it is not spelled out in the
//! reference, so a session that refuses it is skipped rather than treated as an error.
//!
//! # Unsafe code
//!
//! This is one of the two modules of the crate allowed to contain `unsafe` (the other is
//! `sys`). `wasapi` has safe wrappers for listing sessions but none for a session's mute or
//! for telling the system-sounds session apart, and no way from its wrapper to the
//! interface underneath, so the session calls are made here directly. The `windows`
//! bindings mark every COM method `unsafe` because the compiler cannot check that the
//! interface is valid and used in the right apartment. Here that holds by construction:
//! every interface comes from a successful Windows call a few lines earlier, COM interfaces
//! are not `Send`, so [`Live`] cannot leave the thread that obtained it, and each is
//! released by its `Drop`. The four `unsafe` blocks below are kept to the calls themselves,
//! each with its `SAFETY:` note, behind safe functions. What is decided from their answers
//! ([`should_mute`], [`find`]) is plain code with unit tests; the read-only calls run in a
//! test on any machine with an audio device, and the mute round trip is checked by hand
//! with `tandem-cli quiet-probe` on a device nobody hears.
#![allow(unsafe_code)]

use anyhow::Result;
use wasapi::DeviceEnumerator;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, IAudioSessionControl, IAudioSessionControl2, IAudioSessionManager2,
    ISimpleAudioVolume,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::core::{GUID, Interface, PWSTR};

/// Tags Tandem's own mute changes in the events Windows sends to other volume controls.
const EVENT_CONTEXT: GUID = GUID::from_u128(0x7a9d_3c41_52e6_4b0f_9c2a_54616e64656d);

/// A session as Windows identifies it. `instance_id` names this very session (it includes
/// the process id); `session_id` stays the same when the program is started again, which is
/// also what Windows keys the remembered mute on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    pub device_id: String,
    pub instance_id: String,
    pub session_id: String,
    pub process_id: u32,
}

/// What a session looks like without its controls, for listings and for the matching rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub reference: SessionRef,
    pub active: bool,
    pub system_sounds: bool,
    pub muted: bool,
    pub own: bool,
}

/// One session found on a device: what it is, and the control that mutes it. Not `Send`
/// (it holds a COM interface), so it stays on the thread that listed it.
struct Live {
    info: SessionInfo,
    volume: ISimpleAudioVolume,
}

/// Copy a string Windows allocated for us, then free it.
fn take_string(text: windows::core::Result<PWSTR>) -> String {
    let Ok(text) = text else {
        return String::new();
    };
    if text.is_null() {
        return String::new();
    }
    // SAFETY: `text` is the non-null result of a successful `GetSession...Identifier` call,
    // which returns a NUL-terminated wide string allocated with `CoTaskMemAlloc` that the
    // caller must free with `CoTaskMemFree`. It is read once, freed once, and never used
    // after this function, which owns it.
    unsafe {
        let owned = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.as_ptr() as *const _));
        owned
    }
}

/// The session controls of every session on `device_id`. Must run on a thread with COM
/// initialised (`wasapi` returns an error otherwise).
fn session_controls(device_id: &str) -> Result<Vec<IAudioSessionControl>> {
    let device = DeviceEnumerator::new()?.get_device(device_id)?;
    // SAFETY: `device` wraps a live `IMMDevice` that wasapi just got from Windows on this
    // thread. `Activate` with no activation parameters is the documented way to get the
    // session manager; the enumerator and each control come from the successful call
    // before them, and `GetSession` is only asked for indexes below `GetCount`.
    unsafe {
        let manager: IAudioSessionManager2 = device.as_immdevice().Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        // A session can vanish between the count and the read: skip it.
        Ok((0..sessions.GetCount()?)
            .filter_map(|i| sessions.GetSession(i).ok())
            .collect())
    }
}

/// Read one session. `None` when it refuses an interface or has no identity: it is left
/// alone, the others still matter.
fn read_session(device_id: &str, control: &IAudioSessionControl) -> Option<Live> {
    // `cast` is a checked `QueryInterface`: an error here is a session that cannot be muted
    // this way, not a fault.
    let control2 = control.cast::<IAudioSessionControl2>().ok()?;
    let volume = control.cast::<ISimpleAudioVolume>().ok()?;
    // SAFETY: the three interfaces belong to one session control just obtained on this
    // thread from the enumerator, alive for the whole block. These six calls only read.
    let (instance_id, session_id, process_id, state, system_sounds, muted) = unsafe {
        (
            control2.GetSessionInstanceIdentifier(),
            control2.GetSessionIdentifier(),
            control2.GetProcessId(),
            control.GetState(),
            control2.IsSystemSoundsSession(),
            volume.GetMute(),
        )
    };
    let instance_id = take_string(instance_id);
    let session_id = take_string(session_id);
    if instance_id.is_empty() {
        return None;
    }
    let process_id = process_id.unwrap_or(0);
    Some(Live {
        info: SessionInfo {
            reference: SessionRef {
                device_id: device_id.to_string(),
                instance_id,
                session_id,
                process_id,
            },
            active: state.is_ok_and(|s| s == AudioSessionStateActive),
            system_sounds: system_sounds == S_OK,
            muted: muted.is_ok_and(|m| m.as_bool()),
            own: process_id == std::process::id(),
        },
        volume,
    })
}

/// Every session on `device_id` that answers.
fn live_sessions(device_id: &str) -> Result<Vec<Live>> {
    Ok(session_controls(device_id)?
        .iter()
        .filter_map(|control| read_session(device_id, control))
        .collect())
}

impl Live {
    /// Mute or unmute the session. An error means it could not be reached.
    fn set_muted(&self, muted: bool) -> windows::core::Result<()> {
        // SAFETY: `self.volume` is this session's volume control, obtained on this thread
        // and alive as long as `self`. The event context is a `'static` constant, so the
        // pointer handed to Windows is valid for as long as it could be read.
        unsafe { self.volume.SetMute(muted, &EVENT_CONTEXT) }
    }
}

/// Whether a measurement should silence this session: another program's, playing right
/// now, not the system sounds, and not already muted by the user.
pub fn should_mute(session: &SessionInfo) -> bool {
    session.active && !session.own && !session.system_sounds && !session.muted
}

/// Which live session a remembered one is now: itself if it still exists, else the same
/// program's new session (it was started again, and Windows gave it back the old mute).
pub fn find<'a>(live: &'a [SessionInfo], wanted: &SessionRef) -> Option<&'a SessionInfo> {
    live.iter()
        .find(|s| s.reference.instance_id == wanted.instance_id)
        .or_else(|| {
            live.iter().find(|s| {
                !wanted.session_id.is_empty() && s.reference.session_id == wanted.session_id
            })
        })
}

/// Every session on a device, for diagnostics. Changes nothing.
pub fn list(device_id: &str) -> Result<Vec<SessionInfo>> {
    Ok(live_sessions(device_id)?
        .into_iter()
        .map(|l| l.info)
        .collect())
}

/// The sessions a measurement on `device_id` would silence. Changes nothing: the caller
/// records them before calling [`mute`].
pub fn candidates(device_id: &str) -> Result<Vec<SessionRef>> {
    Ok(live_sessions(device_id)?
        .into_iter()
        .map(|l| l.info)
        .filter(should_mute)
        .map(|s| s.reference)
        .collect())
}

/// Mute the given sessions. Returns how many were muted; one that is gone is not an error.
pub fn mute(sessions: &[SessionRef]) -> usize {
    set_mute(sessions, true).0
}

/// Unmute sessions muted by [`mute`]. Returns the ones that could not be reached (their
/// program is not running, or the device is away): they still carry the mute Windows
/// remembers, so the caller keeps them and tries again later. A session found already
/// unmuted counts as restored.
pub fn unmute(sessions: &[SessionRef]) -> Vec<SessionRef> {
    set_mute(sessions, false).1
}

/// Returns (how many were changed, the ones not found).
fn set_mute(sessions: &[SessionRef], muted: bool) -> (usize, Vec<SessionRef>) {
    let mut changed = 0;
    let mut missing = Vec::new();
    let mut devices: Vec<&str> = sessions.iter().map(|s| s.device_id.as_str()).collect();
    devices.sort_unstable();
    devices.dedup();
    for device_id in devices {
        let wanted = sessions.iter().filter(|s| s.device_id == device_id);
        let Ok(live) = live_sessions(device_id) else {
            missing.extend(wanted.cloned());
            continue;
        };
        let infos: Vec<SessionInfo> = live.iter().map(|l| l.info.clone()).collect();
        for session in wanted {
            let Some(found) = find(&infos, session) else {
                missing.push(session.clone());
                continue;
            };
            if found.muted == muted {
                continue;
            }
            let reached = live
                .iter()
                .find(|l| l.info.reference.instance_id == found.reference.instance_id)
                .is_some_and(|l| l.set_muted(muted).is_ok());
            if reached {
                changed += 1;
            } else {
                missing.push(session.clone());
            }
        }
    }
    (changed, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(instance: &str, id: &str) -> SessionInfo {
        SessionInfo {
            reference: SessionRef {
                device_id: "dev".into(),
                instance_id: instance.into(),
                session_id: id.into(),
                process_id: 4242,
            },
            active: true,
            system_sounds: false,
            muted: false,
            own: false,
        }
    }

    /// Exercises the real calls that list sessions, read-only, on the default playback
    /// device. Muting is not tried here: it would change this machine's mixer.
    #[test]
    fn the_default_device_lists_its_sessions() {
        // Hosted build machines have no audio stack to ask.
        if std::env::var_os("CI").is_some() {
            return;
        }
        let _ = wasapi::initialize_mta();
        let Ok(device) = DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(wasapi::Direction::Render))
            .and_then(|d| d.get_id())
        else {
            return; // a machine without any playback device
        };
        let listed = list(&device).expect("listing sessions");
        for s in &listed {
            assert_eq!(s.reference.device_id, device);
            assert!(!s.reference.instance_id.is_empty());
            // A candidate is always another program's audible, playing session.
            if should_mute(s) {
                assert!(s.active && !s.muted && !s.system_sounds && !s.own);
            }
        }
    }

    #[test]
    fn only_other_programs_that_are_playing_and_unmuted_are_silenced() {
        let playing = session("a|1", "a");
        assert!(should_mute(&playing));
        assert!(!should_mute(&SessionInfo {
            own: true,
            ..playing.clone()
        }));
        assert!(!should_mute(&SessionInfo {
            system_sounds: true,
            ..playing.clone()
        }));
        assert!(!should_mute(&SessionInfo {
            active: false,
            ..playing.clone()
        }));
        // Already muted by the user: not ours to touch, so never ours to unmute.
        assert!(!should_mute(&SessionInfo {
            muted: true,
            ..playing
        }));
    }

    #[test]
    fn a_remembered_session_is_found_as_itself_first() {
        let live = [
            session("player|10", "player"),
            session("player|11", "player"),
        ];
        let wanted = live[1].reference.clone();
        assert_eq!(
            find(&live, &wanted).unwrap().reference.instance_id,
            "player|11"
        );
    }

    #[test]
    fn a_program_started_again_is_found_by_its_lasting_id() {
        let live = [session("other|7", "other"), session("player|99", "player")];
        let before_restart = session("player|10", "player").reference;
        assert_eq!(
            find(&live, &before_restart).unwrap().reference.instance_id,
            "player|99"
        );
    }

    #[test]
    fn a_program_that_is_not_running_is_not_found() {
        let live = [session("other|7", "other")];
        assert!(find(&live, &session("player|10", "player").reference).is_none());
        // An empty lasting id never matches by accident.
        let nameless = [session("x|1", "")];
        assert!(find(&nameless, &session("y|2", "").reference).is_none());
    }
}
