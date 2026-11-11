//! Windows services the `wasapi` crate does not wrap: real-time thread scheduling and the
//! endpoint volume.

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
pub struct ProAudio(Option<HANDLE>);

impl ProAudio {
    /// Best effort: without the multimedia scheduler the thread keeps its normal priority.
    pub fn enter() -> Self {
        let mut task_index = 0u32;
        Self(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) }.ok())
    }

    pub fn is_active(&self) -> bool {
        self.0.is_some()
    }
}

impl Drop for ProAudio {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
        }
    }
}

/// The part of a playback device's volume that loopback capture does not see.
///
/// Loopback taps the mix after Windows' software volume but before a volume or mute the
/// device applies itself (most Bluetooth devices, some USB ones). With such a device as the
/// source, the volume keys would change the source alone; the copies have to follow by hand.
pub struct EndpointVolume {
    volume: IAudioEndpointVolume,
    hardware_volume: bool,
    hardware_mute: bool,
}

impl EndpointVolume {
    /// Must be used on the thread (COM apartment) that opened `device`.
    pub fn open(device: &wasapi::Device) -> Result<Self> {
        let volume: IAudioEndpointVolume =
            unsafe { device.as_immdevice().Activate(CLSCTX_ALL, None)? };
        let mask = unsafe { volume.QueryHardwareSupport()? };
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

    /// Linear gain the device applies after the loopback tap (1.0 when Windows does it all
    /// in software, since loopback already carries that).
    pub fn hidden_gain(&self) -> f32 {
        unsafe {
            if self.hardware_mute && self.volume.GetMute().is_ok_and(|m| m.as_bool()) {
                return 0.0;
            }
            if !self.hardware_volume {
                return 1.0;
            }
            if self
                .volume
                .GetMasterVolumeLevelScalar()
                .is_ok_and(|s| s <= 0.0)
            {
                return 0.0;
            }
            match self.volume.GetMasterVolumeLevel() {
                Ok(db) => 10f32.powf(db.min(0.0) / 20.0),
                Err(_) => 1.0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro_audio_class_is_granted() {
        // Hosted build machines have no audio stack to ask.
        if std::env::var_os("CI").is_some() {
            return;
        }
        assert!(ProAudio::enter().is_active());
    }
}
