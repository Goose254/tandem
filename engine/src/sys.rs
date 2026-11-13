//! Windows services the `wasapi` crate does not wrap: real-time thread scheduling and the
//! endpoint volume.
//!
//! # Unsafe code
//!
//! This is one of the two modules of the crate allowed to contain `unsafe` (the other is
//! `sessions`); the crate denies it everywhere else. The calls here have no safe wrapper in
//! `wasapi` or in `windows`, whose bindings mark every Windows function and COM method
//! `unsafe` because the compiler cannot check what Windows requires of the caller. Each
//! block is kept to the calls themselves, states under `SAFETY:` why those requirements
//! hold, and sits behind a safe function. The decisions taken from what Windows returns
//! are plain functions with unit tests; the calls themselves are exercised by the tests at
//! the bottom, on any machine that has an audio device.
#![allow(unsafe_code)]

use anyhow::Result;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
};
use windows::core::w;

/// `ENDPOINT_HARDWARE_SUPPORT_VOLUME` / `_MUTE`.
const HARDWARE_VOLUME: u32 = 0x1;
const HARDWARE_MUTE: u32 = 0x2;

/// Puts the calling thread in the Windows "Pro Audio" scheduling class until dropped, so a
/// busy CPU (a game, a build) does not starve it and cause dropouts.
///
/// The handle belongs to the thread that asked for it: `HANDLE` is not `Send`, so this guard
/// cannot leave that thread and is always reverted there.
pub struct ProAudio(Option<HANDLE>);

impl ProAudio {
    /// Best effort: without the multimedia scheduler the thread keeps its normal priority.
    pub fn enter() -> Self {
        let mut task_index = 0u32;
        // SAFETY: the task name is a NUL-terminated wide string literal, and `task_index`
        // is a live local the call may write to. The call has no other requirement; it
        // acts on the calling thread only.
        let handle = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) };
        Self(handle.ok())
    }

    pub fn is_active(&self) -> bool {
        self.0.is_some()
    }
}

impl Drop for ProAudio {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            // SAFETY: `handle` came from a successful `AvSetMmThreadCharacteristicsW` on
            // this thread and is reverted exactly once: `take` empties the slot.
            let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
        }
    }
}

/// What Windows reports about a playback device's volume at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
struct VolumeReading {
    muted: bool,
    /// Slider position, 0..1.
    scalar: f32,
    /// The same volume in decibels (0 = full level).
    level_db: f32,
}

impl VolumeReading {
    /// Linear gain of the volume alone, mute aside.
    fn volume_gain(&self) -> f32 {
        if self.scalar <= 0.0 {
            0.0
        } else {
            10f32.powf(self.level_db.min(0.0) / 20.0)
        }
    }
}

/// The gain the device applies after the loopback tap, which the copies do not carry.
fn hidden_gain(hardware_volume: bool, hardware_mute: bool, now: VolumeReading) -> f32 {
    if hardware_mute && now.muted {
        0.0
    } else if hardware_volume {
        now.volume_gain()
    } else {
        1.0
    }
}

/// The gain already in the loopback audio: Windows' software volume and mute.
fn loopback_gain(hardware_volume: bool, hardware_mute: bool, now: VolumeReading) -> f32 {
    if !hardware_mute && now.muted {
        0.0
    } else if hardware_volume {
        1.0
    } else {
        now.volume_gain()
    }
}

/// A playback device's volume, split into the part loopback capture carries and the part it
/// does not.
///
/// Loopback taps the mix after Windows' software volume but before a volume or mute the
/// device applies itself (most Bluetooth devices, some USB ones). With such a device as the
/// source, the volume keys would change the source alone; the copies have to follow by hand.
///
/// The COM interface inside is not `Send`: the value stays on the thread that opened it.
pub struct EndpointVolume {
    volume: IAudioEndpointVolume,
    hardware_volume: bool,
    hardware_mute: bool,
}

impl EndpointVolume {
    /// Must be called on a thread with COM initialised, like `device` itself.
    pub fn open(device: &wasapi::Device) -> Result<Self> {
        // SAFETY: `device` wraps a live `IMMDevice` (wasapi only hands out ones it got from
        // Windows) used here on the thread that owns it. `Activate` with no activation
        // parameters is the documented way to get `IAudioEndpointVolume`, and
        // `QueryHardwareSupport` has no precondition beyond a valid interface.
        let (volume, mask) = unsafe {
            let volume: IAudioEndpointVolume = device.as_immdevice().Activate(CLSCTX_ALL, None)?;
            let mask = volume.QueryHardwareSupport()?;
            (volume, mask)
        };
        Ok(Self {
            volume,
            hardware_volume: mask & HARDWARE_VOLUME != 0,
            hardware_mute: mask & HARDWARE_MUTE != 0,
        })
    }

    /// True when the device applies its volume or mute itself, after the loopback tap.
    pub fn is_hardware(&self) -> bool {
        self.hardware_volume || self.hardware_mute
    }

    /// The device's volume right now. A value Windows will not give reads as "full level,
    /// not muted", which leaves the audio untouched.
    fn read(&self) -> VolumeReading {
        // SAFETY: `self.volume` is the interface `open` obtained, alive as long as `self`
        // and still on its thread (it is not `Send`). These three getters only read.
        let (muted, scalar, level_db) = unsafe {
            (
                self.volume.GetMute(),
                self.volume.GetMasterVolumeLevelScalar(),
                self.volume.GetMasterVolumeLevel(),
            )
        };
        VolumeReading {
            muted: muted.is_ok_and(|m| m.as_bool()),
            scalar: scalar.unwrap_or(1.0),
            level_db: level_db.unwrap_or(0.0),
        }
    }

    /// Linear gain the device applies after the loopback tap (1.0 when Windows does it all
    /// in software, since loopback already carries that).
    pub fn hidden_gain(&self) -> f32 {
        hidden_gain(self.hardware_volume, self.hardware_mute, self.read())
    }

    /// Linear gain already in the loopback audio: Windows' software volume and mute. A signal
    /// handed straight to the outputs has to be scaled by it to sit at the level of
    /// everything else they copy. 1.0 when the device applies its volume itself, since
    /// loopback is tapped before that (that part is [`EndpointVolume::hidden_gain`]).
    pub fn loopback_gain(&self) -> f32 {
        loopback_gain(self.hardware_volume, self.hardware_mute, self.read())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hosted build machines have no audio stack to ask.
    fn no_audio_here() -> bool {
        std::env::var_os("CI").is_some()
    }

    const HALF: VolumeReading = VolumeReading {
        muted: false,
        scalar: 0.5,
        level_db: -6.0,
    };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_software_volume_is_in_the_copies_and_never_applied_twice() {
        assert_eq!(hidden_gain(false, false, HALF), 1.0);
        assert!(close(loopback_gain(false, false, HALF), 0.501));
    }

    #[test]
    fn a_hardware_volume_is_missing_from_the_copies_and_applied_by_hand() {
        assert!(close(hidden_gain(true, true, HALF), 0.501));
        assert_eq!(loopback_gain(true, true, HALF), 1.0);
    }

    #[test]
    fn a_mute_counts_on_the_side_that_applies_it() {
        let muted = VolumeReading {
            muted: true,
            ..HALF
        };
        // Software mute: loopback is already silent, nothing more to hide.
        assert_eq!(loopback_gain(false, false, muted), 0.0);
        assert_eq!(hidden_gain(false, false, muted), 1.0);
        // Hardware mute: loopback still carries the sound, so the copies are silenced.
        assert_eq!(hidden_gain(true, true, muted), 0.0);
        assert_eq!(loopback_gain(true, true, muted), 1.0);
    }

    #[test]
    fn a_slider_at_zero_is_silence_whatever_the_decibels_say() {
        let zero = VolumeReading {
            muted: false,
            scalar: 0.0,
            level_db: -65.0,
        };
        assert_eq!(loopback_gain(false, false, zero), 0.0);
        assert_eq!(hidden_gain(true, false, zero), 0.0);
        // And a level above 0 dB never amplifies.
        let hot = VolumeReading {
            level_db: 3.0,
            ..HALF
        };
        assert_eq!(loopback_gain(false, false, hot), 1.0);
    }

    #[test]
    fn pro_audio_class_is_granted() {
        if no_audio_here() {
            return;
        }
        assert!(ProAudio::enter().is_active());
    }

    /// Exercises the real calls behind [`EndpointVolume`], read-only, on the default
    /// playback device.
    #[test]
    fn the_default_device_reports_a_sane_volume() {
        if no_audio_here() {
            return;
        }
        let _ = wasapi::initialize_mta();
        let Ok(device) = wasapi::DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(wasapi::Direction::Render))
        else {
            return; // a machine without any playback device
        };
        let volume = EndpointVolume::open(&device).expect("endpoint volume");
        let now = volume.read();
        assert!((0.0..=1.0).contains(&now.scalar), "{now:?}");
        assert!(now.level_db.is_finite(), "{now:?}");
        for gain in [volume.hidden_gain(), volume.loopback_gain()] {
            assert!((0.0..=1.0).contains(&gain), "{gain}");
        }
        // The two parts never both attenuate: a volume is applied on one side only.
        if !now.muted {
            assert!(volume.hidden_gain() == 1.0 || volume.loopback_gain() == 1.0);
        }
    }
}
