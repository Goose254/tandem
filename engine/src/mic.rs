//! Microphone recording for auto-sync.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::devices::mix_format;

/// A recording in progress. Call [`Recording::finish`] to stop and collect the samples.
pub struct Recording {
    rate: usize,
    stop: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    thread: JoinHandle<Result<Vec<f32>>>,
}

impl Recording {
    /// Start recording `device_id` (a capture device), downmixed to mono, at its own rate.
    pub fn start(device_id: &str) -> Result<Self> {
        let fmt = mix_format(device_id).context("reading the microphone format")?;
        let stop = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let id = device_id.to_string();
        let (stop2, started2) = (Arc::clone(&stop), Arc::clone(&started));
        let thread = thread::Builder::new().name("mic".into()).spawn(move || {
            let _ = initialize_mta();
            let dev = DeviceEnumerator::new()?.get_device(&id)?;
            let mut client = dev.get_iaudioclient()?;
            let (period, _) = client.get_device_period()?;
            let wf = WaveFormat::new(32, 32, SampleType::Float, fmt.rate, fmt.channels, None);
            client
                .initialize_client(
                    &wf,
                    Direction::Capture,
                    StreamMode::PollingShared {
                        autoconvert: true,
                        buffer_duration_hns: 4 * period,
                    },
                    None,
                )
                .context("opening the microphone")?;
            let capture = client.get_audiocaptureclient()?;
            let mut bytes = VecDeque::new();
            let mut mono = Vec::with_capacity(fmt.rate * 10);
            client.start_stream()?;
            started2.store(true, Relaxed);
            while !stop2.load(Relaxed) {
                while capture.get_next_packet_size()?.unwrap_or(0) > 0 {
                    capture.read_from_device_to_deque(&mut bytes)?;
                }
                let frame_bytes = 4 * fmt.channels;
                let whole = bytes.len() - bytes.len() % frame_bytes;
                for frame in bytes.make_contiguous()[..whole].chunks_exact(frame_bytes) {
                    let sum: f32 = frame
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .sum();
                    mono.push(sum / fmt.channels as f32);
                }
                bytes.drain(..whole);
                thread::sleep(Duration::from_millis(3));
            }
            client.stop_stream()?;
            Ok(mono)
        })?;
        Ok(Self {
            rate: fmt.rate,
            stop,
            started,
            thread,
        })
    }

    pub fn rate(&self) -> usize {
        self.rate
    }

    /// Block until the stream is actually running (or failed).
    pub fn wait_started(&self, timeout: Duration) -> bool {
        let step = Duration::from_millis(5);
        let mut waited = Duration::ZERO;
        while !self.started.load(Relaxed) && !self.thread.is_finished() && waited < timeout {
            thread::sleep(step);
            waited += step;
        }
        self.started.load(Relaxed)
    }

    pub fn finish(self) -> Result<Vec<f32>> {
        self.stop.store(true, Relaxed);
        self.thread
            .join()
            .map_err(|_| anyhow!("the microphone thread panicked"))?
    }
}
