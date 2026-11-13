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
//!   per output. A measurement can also hand it a signal for the outputs alone, which never
//!   goes through the source device (see [`Engine::play_signal_to_outputs`]).
//! - one **render** thread per output pulls from its FIFO through a drift-controlled
//!   resampler, keeping the buffered amount equal to that output's delay.
//!
//! A failing output only stops itself. A failing source stops the engine; the app's
//! supervisor restarts it when the device comes back.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::control::{Action, DriftController};
use crate::devices::Format;
use crate::mix::Matrix;
use crate::sys::{EndpointVolume, ProAudio};

/// Delays are added on top of each output's minimum path (see [`OutputStatus::min_lag_ms`]),
/// so 0 ms means "as early as this output can play".
pub const MIN_DELAY_MS: u32 = 0;
pub const MAX_DELAY_MS: u32 = 2000;
/// Input frames per resampler call. Small, since it adds directly to the minimum lag.
const CHUNK: usize = 128;
/// Headroom kept in the FIFO at 0 ms on top of one loopback packet, against scheduling jitter.
const SAFETY_S: f64 = 0.003;
/// How often the source's own volume is read, in keepalive periods (about 10 ms each).
const VOLUME_POLL_PERIODS: u32 = 4;
/// Length of the fade applied to the last audio before an underrun's silence.
const UNDERRUN_FADE_S: f64 = 0.003;
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
    /// See [`Engine::set_follow_volume`].
    pub follow_volume: bool,
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
    /// Smoothed delay actually applied on top of the minimum path; tracks the requested delay.
    pub buffered_ms: f64,
    /// The minimum path this output adds after the source's audio leaves Windows: device
    /// buffer, resampler and one loopback packet. At a delay of 0 ms the output plays roughly
    /// this much after the source (assuming both devices have similar hardware latency).
    /// If it still sounds late at 0 ms, the source is too fast for this output.
    pub min_lag_ms: f64,
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
    fmt: Format,
    thread: Option<JoinHandle<()>>,
}

impl OutputHandle {
    pub fn key(&self) -> &str {
        &self.shared.key
    }
    /// The device's shared-mode format, which the source is resampled to.
    pub fn format(&self) -> Format {
        self.fmt
    }
    pub fn set_delay_ms(&self, ms: u32) {
        self.shared
            .delay_ms
            .store(ms.clamp(MIN_DELAY_MS, MAX_DELAY_MS), Relaxed);
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
    /// Ask the output to stop without waiting for it. Dropping the handle waits.
    pub fn signal_stop(&self) {
        self.shared.stop.store(true, Relaxed);
    }
    fn stop(&mut self) {
        self.signal_stop();
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

/// What the capture thread adds to a batch on its way to the outputs.
#[derive(Default)]
struct FanOut {
    /// `started_at` counts frames handed to the outputs, not frames played on the source.
    injection: Injection,
    /// Frames handed to the outputs so far.
    frame: u64,
}

struct SourceShared {
    fmt: Format,
    stop: AtomicBool,
    finished: AtomicBool,
    error: Mutex<Option<String>>,
    clicks: AtomicBool,
    outputs: RwLock<Vec<Arc<OutputShared>>>,
    injection: Mutex<Injection>,
    /// Signals for the outputs alone, added after capture, and the clock they are timed on.
    fan_out: Mutex<FanOut>,
    /// Drop the captured audio before it reaches the outputs (a measurement is running).
    isolated: AtomicBool,
    /// Loopback packets flagged as discontinuous: capture lost audio there.
    discontinuities: AtomicU64,
    level: AtomicU32,
    /// Highest captured level since [`Engine::take_source_peak`] last asked.
    peak: AtomicU32,
    /// The Windows software volume that loopback audio already carries (1.0 when the device
    /// applies its volume itself), kept current by the keepalive thread.
    loopback_gain: AtomicU32,
    /// Apply the source's hardware volume and mute to the outputs.
    follow_volume: AtomicBool,
    /// The gain to apply for that, kept current by the keepalive thread.
    volume_gain: AtomicU32,
    /// The source applies its volume or mute itself, after the loopback tap.
    hardware_volume: AtomicBool,
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
            fan_out: Mutex::new(FanOut::default()),
            isolated: AtomicBool::new(false),
            discontinuities: AtomicU64::new(0),
            level: f32_atomic(0.0),
            peak: f32_atomic(0.0),
            loopback_gain: f32_atomic(1.0),
            follow_volume: AtomicBool::new(cfg.follow_volume),
            volume_gain: f32_atomic(1.0),
            hardware_volume: AtomicBool::new(false),
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
                        let _priority = ProAudio::enter();
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
        Ok(Self {
            source,
            threads,
            outputs: Vec::new(),
        })
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

    /// Loopback capture does not see a volume or mute that the source device applies itself
    /// (most Bluetooth devices). When on, the outputs follow it, so the Windows volume keys
    /// act on every speaker. A software volume is already in the captured audio and is left
    /// alone either way.
    pub fn set_follow_volume(&self, on: bool) {
        self.source.follow_volume.store(on, Relaxed);
    }

    /// True when the source applies its volume itself, so following it makes a difference.
    pub fn source_volume_is_hardware(&self) -> bool {
        self.source.hardware_volume.load(Relaxed)
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
        self.source
            .injection
            .lock()
            .unwrap()
            .queue
            .push_back(signal);
    }

    /// Source-clock frame index where each signal queued by [`Engine::play_signal`] started.
    pub fn signal_start_frames(&self) -> Vec<u64> {
        self.source.injection.lock().unwrap().started_at.clone()
    }

    /// Queue a mono signal (at the source rate) for the outputs alone: it is added after
    /// capture, so the source device never plays it. It is scaled by the Windows software
    /// volume, like everything else the outputs copy, and a muted output still discards it.
    pub fn play_signal_to_outputs(&self, signal: Arc<Vec<f32>>) {
        self.source
            .fan_out
            .lock()
            .unwrap()
            .injection
            .queue
            .push_back(signal);
    }

    /// Where each signal queued by [`Engine::play_signal_to_outputs`] started, counted in
    /// frames handed to the outputs. The difference between two entries is the time between
    /// them as long as capture lost nothing in between (see
    /// [`Engine::capture_discontinuities`]).
    pub fn output_signal_start_frames(&self) -> Vec<u64> {
        self.source
            .fan_out
            .lock()
            .unwrap()
            .injection
            .started_at
            .clone()
    }

    /// While on, the outputs get none of the captured audio: only what
    /// [`Engine::play_signal_to_outputs`] queues. The source's own level meter is unaffected.
    /// Whoever turns this on must turn it off on every exit path.
    pub fn set_outputs_isolated(&self, on: bool) {
        self.source.isolated.store(on, Relaxed);
    }

    pub fn outputs_isolated(&self) -> bool {
        self.source.isolated.load(Relaxed)
    }

    /// How many loopback packets arrived flagged as discontinuous since the engine started.
    /// A measurement that sees this number move lost audio and cannot trust its timing.
    pub fn capture_discontinuities(&self) -> u64 {
        self.source.discontinuities.load(Relaxed)
    }

    /// The highest level captured from the source since the last call, 0..1, then reset.
    /// Unlike [`Engine::source_level`] it has no fall time, so a silent stretch reads as zero.
    pub fn take_source_peak(&self) -> f32 {
        f32::from_bits(self.source.peak.swap(0, Relaxed))
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
                let _priority = ProAudio::enter();
                if let Err(e) = render_loop(&src, &out, &device_id, src_fmt, dst_fmt) {
                    *out.error.lock().unwrap() = Some(format!("{e:#}"));
                }
                out.finished.store(true, Relaxed);
            })
            .context("spawning render thread")?;
        self.source
            .outputs
            .write()
            .unwrap()
            .push(Arc::clone(&shared));
        self.outputs.push(OutputHandle {
            shared,
            fmt: dst_fmt,
            thread: Some(thread),
        });
        Ok(self.outputs.last().unwrap())
    }

    pub fn remove_output(&mut self, key: &str) {
        self.source
            .outputs
            .write()
            .unwrap()
            .retain(|o| o.key != key);
        // Dropping the handle stops and joins its thread.
        self.outputs.retain(|o| o.key() != key);
    }

    /// Detach an output and ask it to stop, without waiting for its thread: the caller drops
    /// the handle where blocking is fine (a thread on a vanished device can take a second).
    pub fn take_output(&mut self, key: &str) -> Option<OutputHandle> {
        self.source
            .outputs
            .write()
            .unwrap()
            .retain(|o| o.key != key);
        let i = self.outputs.iter().position(|o| o.key() == key)?;
        let handle = self.outputs.swap_remove(i);
        handle.signal_stop();
        Some(handle)
    }

    /// Ask every thread to stop without waiting. Dropping the engine waits.
    pub fn signal_stop(&self) {
        self.source.stop.store(true, Relaxed);
        for o in &self.outputs {
            o.signal_stop();
        }
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
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: period,
            },
            None,
        )
        .context("opening the source device for playback")?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    let volume = EndpointVolume::open(&dev).ok();
    src.hardware_volume
        .store(volume.as_ref().is_some_and(|v| v.is_hardware()), Relaxed);
    let mut periods: u32 = 0;
    let mut frame: u64 = 0;
    let mut buf = Vec::new();
    client.start_stream()?;
    while !src.stop.load(Relaxed) {
        if periods % VOLUME_POLL_PERIODS == 0 {
            let gain = match &volume {
                Some(v) if src.follow_volume.load(Relaxed) => v.hidden_gain(),
                _ => 1.0,
            };
            src.volume_gain.store(gain.to_bits(), Relaxed);
            let carried = volume.as_ref().map_or(1.0, |v| v.loopback_gain());
            src.loopback_gain.store(carried.to_bits(), Relaxed);
        }
        periods = periods.wrapping_add(1);
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
            let mut s = if clicks {
                click_sample(now, fmt.rate)
            } else {
                0.0
            };
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
            StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: 4 * period,
            },
            None,
        )
        .context("opening loopback capture")?;
    let capture = client.get_audiocaptureclient()?;
    let mut bytes = VecDeque::new();
    let mut samples: Vec<f32> = Vec::new();
    // Never hold more than this per output, in case a render side stalls.
    let max_samples = (MAX_DELAY_MS as usize + 1000) * fmt.rate / 1000 * fmt.channels;
    let mut level = 0f32;
    let mut started = false;
    client.start_stream()?;
    while !src.stop.load(Relaxed) {
        while capture.get_next_packet_size()?.unwrap_or(0) > 0 {
            let info = capture.read_from_device_to_deque(&mut bytes)?;
            // The very first packet of a stream is always flagged; later ones mean lost audio.
            if info.flags.data_discontinuity && started {
                src.discontinuities.fetch_add(1, Relaxed);
            }
            started = true;
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
            // Non-negative floats order like their bit patterns, so this keeps the maximum.
            src.peak.fetch_max(peak.to_bits(), Relaxed);
            prepare_fan_out(
                &mut samples,
                fmt.channels,
                src.isolated.load(Relaxed),
                f32::from_bits(src.loopback_gain.load(Relaxed)),
                &mut src.fan_out.lock().unwrap(),
            );
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

/// Turn one captured batch (interleaved, `channels` wide) into what the outputs get.
///
/// Outside a measurement this leaves the batch exactly as captured. When `isolated`, the
/// captured audio is dropped. A queued signal is then added on every channel, scaled by
/// `gain`, starting on the first frame after it was queued; the frame it started on is
/// recorded on the fan-out clock, which advances by the batch's frame count either way.
fn prepare_fan_out(
    samples: &mut [f32],
    channels: usize,
    isolated: bool,
    gain: f32,
    fan: &mut FanOut,
) {
    let frames = samples.len() / channels;
    if isolated {
        samples.fill(0.0);
    }
    let inj = &mut fan.injection;
    if inj.playing.is_some() || !inj.queue.is_empty() {
        for (i, frame) in samples.chunks_exact_mut(channels).enumerate() {
            if inj.playing.is_none() {
                let Some(signal) = inj.queue.pop_front() else {
                    break;
                };
                inj.started_at.push(fan.frame + i as u64);
                inj.playing = Some((signal, 0));
            }
            if let Some((signal, pos)) = inj.playing.as_mut() {
                let s = signal.get(*pos).copied().unwrap_or(0.0) * gain;
                for sample in frame.iter_mut() {
                    *sample += s;
                }
                *pos += 1;
                if *pos >= signal.len() {
                    inj.playing = None;
                }
            }
        }
    }
    fan.frame += frames as u64;
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

/// Fade the last `frames` of interleaved audio in `buf` down to silence.
fn fade_out_tail(buf: &mut VecDeque<f32>, channels: usize, frames: usize) {
    let have = buf.len() / channels;
    let n = frames.min(have);
    for i in 0..n {
        let gain = (n - 1 - i) as f32 / n as f32;
        let frame = have - n + i;
        for c in 0..channels {
            buf[frame * channels + c] *= gain;
        }
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
            // One period: the smallest shared-mode buffer, since it adds to the minimum lag.
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: period,
            },
            None,
        )
        .context("opening the output device")?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;

    let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
        .oversampling_factor(256)
        .interpolation(SincInterpolationType::Quadratic);
    let nominal_ratio = dst.rate as f64 / src.rate as f64;
    let mut resampler = Async::<f32>::new_sinc(
        nominal_ratio,
        1.1,
        &params,
        CHUNK,
        src.channels,
        FixedAsync::Input,
    )?;
    let out_max = resampler.output_frames_max();
    let mut in_buf = vec![0f32; CHUNK * src.channels];
    let mut out_buf = vec![0f32; out_max * src.channels];
    let mut out_fifo: VecDeque<f32> = VecDeque::new();
    let mut bytes: Vec<u8> = Vec::new();

    let matrix = Matrix::new(src.channels, dst.channels);
    let mut ctl = DriftController::default();
    let mut priming = true;
    let mut applied_delay_ms = out.delay_ms.load(Relaxed);
    let mut status = OutputStatus::default();
    let mut current_gain = 0f32; // fade in from silence
    let mut level = 0f32;

    // The least that can stay buffered without dropouts: the device buffer (refilled every
    // period), one resampler chunk plus its lookahead, and one loopback packet of capture
    // jitter. The user's delay is added on top, so 0 ms is the earliest this output can play.
    let buffer_frames = client.get_buffer_size()? as usize;
    let floor_s = buffer_frames as f64 / dst.rate as f64
        + CHUNK as f64 / src.rate as f64
        + resampler.output_delay() as f64 / dst.rate as f64
        + period as f64 / 10_000_000.0
        + SAFETY_S;
    status.min_lag_ms = floor_s * 1000.0;

    let latency = |fifo_len: usize, out_len: usize, padding: u32, rs_delay: usize| {
        (fifo_len / src.channels) as f64 / src.rate as f64
            + ((out_len / src.channels) + padding as usize + rs_delay) as f64 / dst.rate as f64
    };

    client.start_stream()?;
    while !out.stop.load(Relaxed) && !src_shared.stop.load(Relaxed) {
        let avail = client.get_available_space_in_frames()? as usize;
        let delay_ms = out.delay_ms.load(Relaxed);
        let target = floor_s + delay_ms as f64 / 1000.0;

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
            if priming {
                let buffered = latency(
                    fifo.len(),
                    out_fifo.len(),
                    padding,
                    resampler.output_delay(),
                );
                if buffered >= target {
                    // Start exactly on target: trim what priming overshot (it fills in whole
                    // capture packets), so the drift controller never has to unwind it slowly.
                    let excess = ((buffered - target) * src.rate as f64) as i64;
                    let keep = (fifo.len() / src.channels) as i64 - CHUNK as i64;
                    shift_fifo(&mut fifo, -excess.min(keep.max(0)), src.channels);
                    priming = false;
                    ctl.reset_filter();
                    // Come back from silence with a ramp, not a step.
                    current_gain = 0.0;
                }
            }
            if priming || fifo.len() < in_buf.len() {
                if !priming {
                    priming = true;
                    status.underruns += 1;
                    // Out of audio: ease what is left into the silence that follows, so the
                    // dropout is a dip instead of a click.
                    let fade = (UNDERRUN_FADE_S * dst.rate as f64) as usize;
                    fade_out_tail(&mut out_fifo, src.channels, fade);
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

        // Write to the device: mix channels, apply a smoothed gain, measure the peak.
        let target_gain = if out.muted.load(Relaxed) {
            0.0
        } else {
            f32::from_bits(out.gain.load(Relaxed))
                * f32::from_bits(src_shared.volume_gain.load(Relaxed))
        };
        let gain_step = if avail > 0 {
            (target_gain - current_gain) / avail as f32
        } else {
            0.0
        };
        let mut peak = 0f32;
        bytes.clear();
        let pcm: &[f32] = out_fifo.make_contiguous();
        for f in 0..avail {
            current_gain += gain_step;
            let frame = &pcm[f * src.channels..(f + 1) * src.channels];
            for c in 0..dst.channels {
                let s = matrix.mix(frame, c) * current_gain;
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
            status.buffered_ms = (ctl.filtered().unwrap_or(measured) - floor_s) * 1000.0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifting_a_fifo_inserts_silence_or_drops_whole_frames() {
        let mut fifo: VecDeque<f32> = (1..=8).map(|v| v as f32).collect();
        shift_fifo(&mut fifo, 1, 2);
        assert_eq!(
            fifo.iter().copied().collect::<Vec<_>>(),
            [0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
        );
        shift_fifo(&mut fifo, -2, 2);
        assert_eq!(
            fifo.iter().copied().collect::<Vec<_>>(),
            [3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
        );
        // Dropping more than there is empties it without panicking.
        shift_fifo(&mut fifo, -100, 2);
        assert!(fifo.is_empty());
    }

    #[test]
    fn the_tail_fades_to_silence_and_the_head_is_untouched() {
        let mut buf: VecDeque<f32> = std::iter::repeat_n(1.0, 16).collect();
        fade_out_tail(&mut buf, 2, 4);
        let v: Vec<f32> = buf.iter().copied().collect();
        assert!(v[..8].iter().all(|s| *s == 1.0));
        assert_eq!(&v[8..], [0.75, 0.75, 0.5, 0.5, 0.25, 0.25, 0.0, 0.0]);
        // Shorter than the fade: still ends on silence.
        let mut short: VecDeque<f32> = std::iter::repeat_n(1.0, 4).collect();
        fade_out_tail(&mut short, 2, 100);
        assert_eq!(short.back().copied(), Some(0.0));
        // Empty: nothing to do.
        fade_out_tail(&mut VecDeque::new(), 2, 4);
    }

    fn fan_with(signals: &[&[f32]]) -> FanOut {
        let mut fan = FanOut::default();
        for s in signals {
            fan.injection.queue.push_back(Arc::new(s.to_vec()));
        }
        fan
    }

    #[test]
    fn outside_a_measurement_the_fan_out_passes_audio_through_untouched() {
        // The normal path: nothing queued, not isolated. Bit for bit what was captured.
        let captured: Vec<f32> = (0..64).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut batch = captured.clone();
        let mut fan = FanOut::default();
        prepare_fan_out(&mut batch, 2, false, 0.5, &mut fan);
        assert_eq!(
            batch.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            captured.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
        assert_eq!(fan.frame, 32);
        assert!(fan.injection.started_at.is_empty());
    }

    #[test]
    fn an_isolated_fan_out_carries_the_signal_and_nothing_captured() {
        let mut batch = vec![0.8f32; 12];
        let mut fan = fan_with(&[&[0.1, 0.2, 0.3]]);
        prepare_fan_out(&mut batch, 2, true, 1.0, &mut fan);
        // The signal on both channels, then silence where the captured audio was.
        assert_eq!(
            batch,
            [0.1, 0.1, 0.2, 0.2, 0.3, 0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(fan.injection.started_at, [0]);
        // Isolation alone, with nothing queued, is plain silence.
        let mut quiet = vec![0.8f32; 4];
        prepare_fan_out(&mut quiet, 2, true, 1.0, &mut fan);
        assert_eq!(quiet, [0.0; 4]);
    }

    #[test]
    fn a_signal_is_mixed_over_captured_audio_when_not_isolated() {
        let mut batch = vec![0.5f32; 6];
        let mut fan = fan_with(&[&[0.25, -0.25]]);
        prepare_fan_out(&mut batch, 2, false, 1.0, &mut fan);
        assert_eq!(batch, [0.75, 0.75, 0.25, 0.25, 0.5, 0.5]);
    }

    #[test]
    fn the_signal_follows_the_windows_volume_the_copies_carry() {
        let mut batch = vec![0.0f32; 4];
        let mut fan = fan_with(&[&[0.4, 0.4]]);
        prepare_fan_out(&mut batch, 1, true, 0.5, &mut fan);
        assert_eq!(batch, [0.2, 0.2, 0.0, 0.0]);
    }

    #[test]
    fn two_signals_are_timed_on_the_frames_handed_to_the_outputs() {
        // Batches of uneven sizes, as loopback delivers them. The second signal is queued
        // while the stream runs; its start is counted in frames, whatever the batching.
        let signal: Vec<f32> = (1..=50).map(|i| i as f32).collect();
        let mut fan = fan_with(&[&signal]);
        let mut stream: Vec<f32> = Vec::new();
        for (n, size) in [7usize, 13, 48, 5, 31, 64, 9, 40].into_iter().enumerate() {
            if n == 4 {
                fan.injection.queue.push_back(Arc::new(signal.clone()));
            }
            let mut batch = vec![0.0f32; size * 2];
            prepare_fan_out(&mut batch, 2, true, 1.0, &mut fan);
            stream.extend(batch.chunks_exact(2).map(|f| f[0]));
        }
        let starts = fan.injection.started_at.clone();
        assert_eq!(starts, [0, 73]);
        assert_eq!(fan.frame, 217);
        // Each copy sits exactly where its recorded start says, whole and in order.
        for start in starts {
            let at = start as usize;
            assert_eq!(&stream[at..at + 50], signal.as_slice());
        }
        assert!(stream[50..73].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_signal_queued_behind_another_starts_right_after_it() {
        let mut fan = fan_with(&[&[1.0, 1.0], &[2.0]]);
        let mut batch = vec![0.0f32; 5];
        prepare_fan_out(&mut batch, 1, true, 1.0, &mut fan);
        assert_eq!(batch, [1.0, 1.0, 2.0, 0.0, 0.0]);
        assert_eq!(fan.injection.started_at, [0, 2]);
    }

    #[test]
    fn clicks_are_one_short_blip_per_second() {
        let rate = 48_000;
        let loud = (0..rate as u64 * 2)
            .filter(|f| click_sample(*f, rate).abs() > 0.01)
            .count();
        // Two blips of 4 ms each, minus the zero crossings.
        assert!(loud > 0 && loud < 2 * rate * 4 / 1000);
        assert_eq!(click_sample(rate as u64 / 2, rate), 0.0);
        assert!(click_sample(6, rate).abs() <= 0.3);
    }
}
