//! The audio pipeline.
//!
//! One **source** device (what Windows plays on, typically the slowest output such as a
//! Bluetooth amp) and any number of **outputs** that receive a delayed copy.
//!
//! Threads:
//! - **keepalive** plays silence on the source device (plus calibration clicks or injected
//!   test signals). This keeps loopback capture flowing when nothing plays, and keeps a
//!   Bluetooth link awake so its latency stays stable.
//! - **capture** reads the source with WASAPI loopback and fans the samples out to one FIFO
//!   per output.
//! - one **render** thread per output pulls from its FIFO through a drift-controlled
//!   resampler, keeping the buffered amount equal to that output's delay.
//!
//! A failing output only stops itself. A failing source stops the engine; the app's
//! supervisor restarts it when the device comes back.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::control::{Action, DriftController};
use crate::devices::Format;

pub const MIN_DELAY_MS: u32 = 30;
pub const MAX_DELAY_MS: u32 = 2000;
/// Input frames per resampler call.
const CHUNK: usize = 256;
/// Level meter fall time constant, in seconds.
const METER_FALL_S: f32 = 0.35;

fn f32_atomic(v: f32) -> AtomicU32 {
    AtomicU32::new(v.to_bits())
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

// ---------------------------------------------------------------------------------------
// Public configuration and status
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub device_id: String,
    pub fmt: Format,
}

#[derive(Debug, Clone)]
pub struct OutputConfig {
    /// Stable key chosen by the caller (the app uses the device id).
    pub key: String,
    pub device_id: String,
    pub fmt: Format,
    pub delay_ms: u32,
    pub gain_db: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct OutputStatus {
    /// Audio is flowing (not priming after a start or an underrun).
    pub running: bool,
    /// Smoothed amount of audio buffered between the source and this output.
    pub buffered_ms: f64,
    /// Current resampling correction for clock drift, in ppm.
    pub correction_ppm: f64,
    pub underruns: u64,
    pub resyncs: u64,
    /// Peak level after gain, 0..1, with a falling hold.
    pub level: f32,
}

// ---------------------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------------------

struct OutputShared {
    key: String,
    fifo: Mutex<VecDeque<f32>>,
    delay_ms: AtomicU32,
    gain: AtomicU32,
    muted: AtomicBool,
    stop: AtomicBool,
    finished: AtomicBool,
    status: Mutex<OutputStatus>,
    error: Mutex<Option<String>>,
}

/// A running output. Parameter changes take effect immediately.
pub struct OutputHandle {
    shared: Arc<OutputShared>,
    thread: Option<JoinHandle<()>>,
}

impl OutputHandle {
    pub fn key(&self) -> &str {
        &self.shared.key
    }
    pub fn set_delay_ms(&self, ms: u32) {
        self.shared.delay_ms.store(ms.clamp(MIN_DELAY_MS, MAX_DELAY_MS), Relaxed);
    }
    pub fn delay_ms(&self) -> u32 {
        self.shared.delay_ms.load(Relaxed)
    }
    pub fn set_gain_db(&self, db: f32) {
        self.shared.gain.store(db_to_gain(db).to_bits(), Relaxed);
    }
    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Relaxed);
    }
    pub fn muted(&self) -> bool {
        self.shared.muted.load(Relaxed)
    }
    pub fn status(&self) -> OutputStatus {
        self.shared.status.lock().unwrap().clone()
    }
    /// Set when the output stopped on its own (device lost, driver error).
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }
    pub fn is_alive(&self) -> bool {
        !self.shared.finished.load(Relaxed)
    }
    fn stop(&mut self) {
        self.shared.stop.store(true, Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for OutputHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Injection {
    queue: VecDeque<Arc<Vec<f32>>>,
    playing: Option<(Arc<Vec<f32>>, usize)>,
    /// Keepalive frame index at which each injected signal started, in order.
    started_at: Vec<u64>,
}

struct SourceShared {
    fmt: Format,
    stop: AtomicBool,
    finished: AtomicBool,
    error: Mutex<Option<String>>,
    clicks: AtomicBool,
    outputs: RwLock<Vec<Arc<OutputShared>>>,
    injection: Mutex<Injection>,
    level: AtomicU32,
}

pub struct Engine {
    source: Arc<SourceShared>,
    threads: Vec<JoinHandle<()>>,
    outputs: Vec<OutputHandle>,
}

impl Engine {
    /// Start capturing the source. Outputs are added with [`Engine::add_output`].
    pub fn start(cfg: SourceConfig) -> Result<Self> {
        let source = Arc::new(SourceShared {
            fmt: cfg.fmt,
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
            clicks: AtomicBool::new(false),
            outputs: RwLock::new(Vec::new()),
            injection: Mutex::new(Injection::default()),
            level: f32_atomic(0.0),
        });
        let mut threads = Vec::new();
        for (name, is_capture) in [("keepalive", false), ("capture", true)] {
            let src = Arc::clone(&source);
            let id = cfg.device_id.clone();
            threads.push(
                thread::Builder::new()
                    .name(name.into())
                    .spawn(move || {
                        let _ = initialize_mta();
                        let result = if is_capture {
                            capture_loop(&src, &id)
                        } else {
                            keepalive_loop(&src, &id)
                        };
                        if let Err(e) = result {
                            let mut err = src.error.lock().unwrap();
                            if err.is_none() {
                                *err = Some(format!("{e:#}"));
                            }
                        }
                        src.stop.store(true, Relaxed);
                        src.finished.store(true, Relaxed);
                    })
                    .context("spawning source thread")?,
            );
        }
        Ok(Self { source, threads, outputs: Vec::new() })
    }

    pub fn source_format(&self) -> Format {
        self.source.fmt
    }

    /// False once the source failed (device lost) or the engine was stopped.
    pub fn source_alive(&self) -> bool {
        !self.source.finished.load(Relaxed)
    }

    pub fn source_error(&self) -> Option<String> {
        self.source.error.lock().unwrap().clone()
    }

    pub fn source_level(&self) -> f32 {
        f32::from_bits(self.source.level.load(Relaxed))
    }

    /// Calibration clicks: one short blip per second on the source device, mirrored to the
    /// outputs. Line them up by ear.
    pub fn set_clicks(&self, on: bool) {
        self.source.clicks.store(on, Relaxed);
    }

    pub fn clicks(&self) -> bool {
        self.source.clicks.load(Relaxed)
    }

    /// Queue a mono signal (at the source rate) to play on the source device.
    pub fn play_signal(&self, signal: Arc<Vec<f32>>) {
        self.source.injection.lock().unwrap().queue.push_back(signal);
    }

    /// Source-clock frame index where each signal queued by [`Engine::play_signal`] started.
    pub fn signal_start_frames(&self) -> Vec<u64> {
        self.source.injection.lock().unwrap().started_at.clone()
    }

    pub fn add_output(&mut self, cfg: OutputConfig) -> Result<&OutputHandle> {
        self.remove_output(&cfg.key);
        let shared = Arc::new(OutputShared {
            key: cfg.key.clone(),
            fifo: Mutex::new(VecDeque::new()),
            delay_ms: AtomicU32::new(cfg.delay_ms.clamp(MIN_DELAY_MS, MAX_DELAY_MS)),
            gain: f32_atomic(db_to_gain(cfg.gain_db)),
            muted: AtomicBool::new(cfg.muted),
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            status: Mutex::new(OutputStatus::default()),
            error: Mutex::new(None),
        });
        let out = Arc::clone(&shared);
        let src = Arc::clone(&self.source);
        let src_fmt = self.source.fmt;
        let device_id = cfg.device_id.clone();
        let dst_fmt = cfg.fmt;
        let thread = thread::Builder::new()
            .name(format!("render {}", cfg.key))
            .spawn(move || {
                let _ = initialize_mta();
                if let Err(e) = render_loop(&src, &out, &device_id, src_fmt, dst_fmt) {
                    *out.error.lock().unwrap() = Some(format!("{e:#}"));
                }
                out.finished.store(true, Relaxed);
            })
            .context("spawning render thread")?;
        self.source.outputs.write().unwrap().push(Arc::clone(&shared));
        self.outputs.push(OutputHandle { shared, thread: Some(thread) });
        Ok(self.outputs.last().unwrap())
    }

    pub fn remove_output(&mut self, key: &str) {
        self.source.outputs.write().unwrap().retain(|o| o.key != key);
        // Dropping the handle stops and joins its thread.
        self.outputs.retain(|o| o.key() != key);
    }

    pub fn output(&self, key: &str) -> Option<&OutputHandle> {
        self.outputs.iter().find(|o| o.key() == key)
    }

    pub fn outputs(&self) -> &[OutputHandle] {
        &self.outputs
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.source.stop.store(true, Relaxed);
        self.source.outputs.write().unwrap().clear();
        self.outputs.clear();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------------------------

fn float_format(fmt: Format) -> WaveFormat {
    WaveFormat::new(32, 32, SampleType::Float, fmt.rate, fmt.channels, None)
}

/// A short 2 kHz blip once per second, for lining up the outputs by ear.
fn click_sample(frame: u64, rate: usize) -> f32 {
    let t = (frame % rate as u64) as f32 / rate as f32;
    const LEN: f32 = 0.004;
    if t < LEN {
        (std::f32::consts::TAU * 2000.0 * t).sin() * (1.0 - t / LEN) * 0.3
    } else {
        0.0
    }
}

fn meter_decay(frames: usize, rate: usize) -> f32 {
    (-(frames as f32) / (rate as f32 * METER_FALL_S)).exp()
}

fn keepalive_loop(src: &SourceShared, device_id: &str) -> Result<()> {
    let fmt = src.fmt;
    let dev = DeviceEnumerator::new()?.get_device(device_id)?;
    let mut client = dev.get_iaudioclient()?;
    let (period, _) = client.get_device_period()?;
    client
        .initialize_client(
            &float_format(fmt),
            Direction::Render,
            StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: period },
            None,
        )
        .context("opening the source device for playback")?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    let mut frame: u64 = 0;
    let mut buf = Vec::new();
    client.start_stream()?;
    while !src.stop.load(Relaxed) {
        let avail = client.get_available_space_in_frames()? as usize;
        let clicks = src.clicks.load(Relaxed);
        buf.clear();
        let mut inj = src.injection.lock().unwrap();
        for i in 0..avail {
            let now = frame + i as u64;
            if inj.playing.is_none()
                && let Some(sig) = inj.queue.pop_front()
            {
                inj.started_at.push(now);
                inj.playing = Some((sig, 0));
            }
            let mut s = if clicks { click_sample(now, fmt.rate) } else { 0.0 };
            if let Some((sig, pos)) = inj.playing.as_mut() {
                s += sig[*pos];
                *pos += 1;
                if *pos >= sig.len() {
                    inj.playing = None;
                }
            }
            for _ in 0..fmt.channels {
                buf.extend_from_slice(&s.to_le_bytes());
            }
        }
        drop(inj);
        render.write_to_device(&buf, None)?;
        frame += avail as u64;
        event
            .wait_for_event(1000)
            .map_err(|_| anyhow!("the source device stopped responding"))?;
    }
    client.stop_stream()?;
    Ok(())
}

fn capture_loop(src: &SourceShared, device_id: &str) -> Result<()> {
    let fmt = src.fmt;
    let dev = DeviceEnumerator::new()?.get_device(device_id)?;
    let mut client = dev.get_iaudioclient()?;
    let (period, _) = client.get_device_period()?;
    // Render device + Capture direction = loopback.
    client
        .initialize_client(
            &float_format(fmt),
            Direction::Capture,
            StreamMode::PollingShared { autoconvert: true, buffer_duration_hns: 4 * period },
            None,
        )
        .context("opening loopback capture")?;
    let capture = client.get_audiocaptureclient()?;
    let mut bytes = VecDeque::new();
    let mut samples: Vec<f32> = Vec::new();
    // Never hold more than this per output, in case a render side stalls.
    let max_samples = (MAX_DELAY_MS as usize + 1000) * fmt.rate / 1000 * fmt.channels;
    let mut level = 0f32;
    client.start_stream()?;
    while !src.stop.load(Relaxed) {
        while capture.get_next_packet_size()?.unwrap_or(0) > 0 {
            capture.read_from_device_to_deque(&mut bytes)?;
        }
        if !bytes.is_empty() {
            samples.clear();
            samples.extend(
                bytes
                    .make_contiguous()
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
            );
            bytes.clear();
            let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            level = peak.max(level * meter_decay(samples.len() / fmt.channels, fmt.rate));
            src.level.store(level.to_bits(), Relaxed);
            for out in src.outputs.read().unwrap().iter() {
                let mut fifo = out.fifo.lock().unwrap();
                fifo.extend(&samples);
                if fifo.len() > max_samples {
                    let excess = fifo.len() - max_samples;
                    fifo.drain(..excess - excess % fmt.channels);
                }
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    client.stop_stream()?;
    Ok(())
}

/// Insert (positive) or drop (negative) `frames` of audio at the head of a FIFO.
fn shift_fifo(fifo: &mut VecDeque<f32>, frames: i64, channels: usize) {
    if frames > 0 {
        for _ in 0..frames as usize * channels {
            fifo.push_front(0.0);
        }
    } else {
        let n = ((-frames) as usize * channels).min(fifo.len());
        fifo.drain(..n - n % channels);
    }
}

fn render_loop(
    src_shared: &SourceShared,
    out: &OutputShared,
    device_id: &str,
    src: Format,
    dst: Format,
) -> Result<()> {
    let dev = DeviceEnumerator::new()?.get_device(device_id)?;
    let mut client = dev.get_iaudioclient()?;
    let (period, _) = client.get_device_period()?;
    client
        .initialize_client(
            &float_format(dst),
            Direction::Render,
            StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 2 * period },
            None,
        )
        .context("opening the output device")?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;

    let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
        .oversampling_factor(256)
        .interpolation(SincInterpolationType::Quadratic);
    let nominal_ratio = dst.rate as f64 / src.rate as f64;
    let mut resampler =
        Async::<f32>::new_sinc(nominal_ratio, 1.1, &params, CHUNK, src.channels, FixedAsync::Input)?;
    let out_max = resampler.output_frames_max();
    let mut in_buf = vec![0f32; CHUNK * src.channels];
    let mut out_buf = vec![0f32; out_max * src.channels];
    let mut out_fifo: VecDeque<f32> = VecDeque::new();
    let mut bytes: Vec<u8> = Vec::new();

    let mut ctl = DriftController::default();
    let mut priming = true;
    let mut applied_delay_ms = out.delay_ms.load(Relaxed);
    let mut status = OutputStatus::default();
    let mut current_gain = 0f32; // fade in from silence
    let mut level = 0f32;

    let latency = |fifo_len: usize, out_len: usize, padding: u32, rs_delay: usize| {
        (fifo_len / src.channels) as f64 / src.rate as f64
            + ((out_len / src.channels) + padding as usize + rs_delay) as f64 / dst.rate as f64
    };

    client.start_stream()?;
    while !out.stop.load(Relaxed) && !src_shared.stop.load(Relaxed) {
        let avail = client.get_available_space_in_frames()? as usize;
        let delay_ms = out.delay_ms.load(Relaxed);
        let target = delay_ms as f64 / 1000.0;

        // Delay changed: jump right away so the effect is audible immediately.
        if delay_ms != applied_delay_ms {
            if !priming {
                let diff = delay_ms as i64 - applied_delay_ms as i64;
                let frames = diff * src.rate as i64 / 1000;
                shift_fifo(&mut out.fifo.lock().unwrap(), frames, src.channels);
                ctl.shift(diff as f64 / 1000.0);
            }
            applied_delay_ms = delay_ms;
        }

        let padding = client.get_current_padding()?;

        // Produce enough resampled audio to fill the device's free space.
        let need = avail * src.channels;
        while out_fifo.len() < need {
            let mut fifo = out.fifo.lock().unwrap();
            if priming
                && latency(fifo.len(), out_fifo.len(), padding, resampler.output_delay()) >= target
            {
                priming = false;
                ctl.reset_filter();
            }
            if priming || fifo.len() < in_buf.len() {
                if !priming {
                    priming = true;
                    status.underruns += 1;
                }
                drop(fifo);
                out_fifo.resize(need, 0.0);
                break;
            }
            for (d, s) in in_buf.iter_mut().zip(fifo.drain(..CHUNK * src.channels)) {
                *d = s;
            }
            drop(fifo);
            let input = InterleavedSlice::new(&in_buf, src.channels, CHUNK)?;
            let mut output = InterleavedSlice::new_mut(&mut out_buf, src.channels, out_max)?;
            let (_, produced) = resampler.process_into_buffer(&input, &mut output, None)?;
            out_fifo.extend(&out_buf[..produced * src.channels]);
        }

        // Write to the device: map channels, apply a smoothed gain, measure the peak.
        let target_gain = if out.muted.load(Relaxed) {
            0.0
        } else {
            f32::from_bits(out.gain.load(Relaxed))
        };
        let gain_step = if avail > 0 { (target_gain - current_gain) / avail as f32 } else { 0.0 };
        let mut peak = 0f32;
        bytes.clear();
        for f in 0..avail {
            current_gain += gain_step;
            let base = f * src.channels;
            for c in 0..dst.channels {
                let raw = if src.channels == 1 {
                    out_fifo[base]
                } else if c < src.channels {
                    out_fifo[base + c]
                } else {
                    0.0
                };
                let s = raw * current_gain;
                peak = peak.max(s.abs());
                bytes.extend_from_slice(&s.to_le_bytes());
            }
        }
        current_gain = target_gain;
        out_fifo.drain(..need);
        render.write_to_device(&bytes, None)?;
        level = peak.max(level * meter_decay(avail, dst.rate));

        // Measure what is buffered between the two devices and steer the resampler.
        if !priming && avail > 0 {
            let padding = client.get_current_padding()?;
            let fifo_len = out.fifo.lock().unwrap().len();
            let measured = latency(fifo_len, out_fifo.len(), padding, resampler.output_delay());
            let dt = avail as f64 / dst.rate as f64;
            match ctl.update(measured, target, dt) {
                Action::Ratio(r) => resampler.set_resample_ratio_relative(r, true)?,
                Action::Resync(secs) => {
                    let frames = (secs * src.rate as f64) as i64;
                    shift_fifo(&mut out.fifo.lock().unwrap(), frames, src.channels);
                    status.resyncs += 1;
                }
            }
            status.buffered_ms = ctl.filtered().unwrap_or(measured) * 1000.0;
        }
        status.running = !priming;
        status.correction_ppm = ctl.correction_ppm();
        status.level = level;
        *out.status.lock().unwrap() = status.clone();

        event
            .wait_for_event(1000)
            .map_err(|_| anyhow!("the output device stopped responding"))?;
    }
    client.stop_stream()?;
    Ok(())
}
