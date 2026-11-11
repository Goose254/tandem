use anyhow::{Result, bail};
use wasapi::{DeviceEnumerator, Direction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub rate: usize,
    pub channels: usize,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} Hz, {} ch", self.rate, self.channels)
    }
}

fn list(direction: Direction) -> Result<Vec<DeviceInfo>> {
    let en = DeviceEnumerator::new()?;
    let default_id = en
        .get_default_device(direction)
        .ok()
        .and_then(|d| d.get_id().ok());
    let mut out = Vec::new();
    for dev in &en.get_device_collection(direction)? {
        // A device that cannot say who it is (a half-removed driver, a virtual device in a
        // bad state) is skipped: it must not hide every other device.
        let Ok(dev) = dev else { continue };
        let (Ok(id), Ok(name)) = (dev.get_id(), dev.get_friendlyname()) else {
            continue;
        };
        out.push(DeviceInfo {
            is_default: Some(&id) == default_id.as_ref(),
            name,
            id,
        });
    }
    Ok(out)
}

/// All active playback devices.
pub fn render_devices() -> Result<Vec<DeviceInfo>> {
    list(Direction::Render)
}

/// All active recording devices (microphones, line inputs).
pub fn capture_devices() -> Result<Vec<DeviceInfo>> {
    list(Direction::Capture)
}

/// Find a playback device by (case-insensitive) name or name fragment.
/// `None` means the Windows default output.
pub fn find(query: Option<&str>) -> Result<DeviceInfo> {
    let devices = render_devices()?;
    let Some(q) = query else {
        return match devices.into_iter().find(|d| d.is_default) {
            Some(d) => Ok(d),
            None => bail!("no default playback device"),
        };
    };
    let q = q.to_lowercase();
    if let Some(d) = devices.iter().find(|d| d.name.to_lowercase() == q) {
        return Ok(d.clone());
    }
    let matches: Vec<_> = devices
        .iter()
        .filter(|d| d.name.to_lowercase().contains(&q))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("no playback device matches \"{q}\""),
        many => {
            let names: Vec<_> = many.iter().map(|d| format!("  - {}", d.name)).collect();
            bail!(
                "\"{q}\" matches several devices, be more specific:\n{}",
                names.join("\n")
            )
        }
    }
}

/// The shared-mode mix format of a device (playback or recording).
pub fn mix_format(id: &str) -> Result<Format> {
    let dev = DeviceEnumerator::new()?.get_device(id)?;
    let fmt = dev.get_iaudioclient()?.get_mixformat()?;
    Ok(Format {
        rate: fmt.get_samplespersec() as usize,
        channels: fmt.get_nchannels() as usize,
    })
}
