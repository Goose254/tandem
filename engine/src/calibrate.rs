//! Microphone auto-sync: signal generation and analysis (pure DSP, no audio I/O).
//!
//! Procedure driven by the app:
//! 1. Record the microphone continuously.
//! 2. Emit sweep #1 on the source device while the target output is muted:
//!    the mic hears only the source.
//! 3. `spacing` seconds later (exact, in source-clock samples), emit sweep #2
//!    with the target unmuted: the mic hears the source and the delayed target copy.
//! 4. `analyze` finds the source arrival of sweep #1, predicts the source arrival of
//!    sweep #2 at +spacing, and the remaining strong arrival near it is the target.
//!    The target-minus-source offset is what the target's delay must be reduced by.
//!
//! The mic's own latency cancels out because both arrivals are in the same recording.
//!
//! # How the analysis works
//!
//! - **Weighting: regularized inverse filter (Farina deconvolution).** The recording's
//!   spectrum is multiplied by `conj(T) / (|T|² + λ·max|T|²)`, where `T` is the sweep's
//!   spectrum. Inside the sweep band this whitens the sweep, so each arrival becomes a
//!   sharp impulse instead of a broad matched-filter lobe. Outside the band, where `|T|`
//!   is tiny, the regularization keeps noise from being amplified. Full GCC-PHAT
//!   normalizes by `|X·conj(T)|` instead, which whitens the *noise* outside the sweep band
//!   to the same level as the signal. That falls apart at moderate SNR.
//! - **Envelope.** Negative frequencies are dropped before the inverse FFT, which yields the
//!   analytic signal. Its magnitude is a smooth envelope without carrier ripple, and peaks
//!   are refined with parabolic sub-sample interpolation.
//! - **Source arrival.** For every lag `k`, score `min(env[k], env[k + spacing])`, the
//!   latter dilated by ±1.5 ms to tolerate clock drift. Only the source shows up twice,
//!   `spacing` apart, so this pairs sweep #1 with sweep #2 and ignores the lone target
//!   arrival. The earliest lag within 6 dB of the best score is the direct path, so it wins
//!   over later (even stronger) reflections.
//! - **Target arrival.** The envelope pattern around the source arrival of sweep #1 (direct
//!   path plus its reflections) is aligned onto sweep #2 to within a sub-sample, and then
//!   subtracted. What remains near sweep #2 is the target alone, so even a target only
//!   2 ms away from the source, or sitting on top of the source's reflections, is found.
//!   The earliest residual peak within 6 dB of the strongest one is the target's direct
//!   path.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// Fade length at each end of the sweep.
const FADE_S: f64 = 0.010;
/// Inverse-filter regularization, relative to the sweep's peak spectral power (-30 dB).
const REGULARIZATION: f64 = 1e-3;
/// A detection must stand this far above the median envelope (the noise floor).
const DETECT_RATIO: f64 = 10.0;
/// The target must be at least this strong relative to the source's direct arrival.
const MIN_TARGET_REL: f64 = 0.15;
/// "Within 6 dB": a peak is a direct-path candidate above this fraction of the strongest.
const SIX_DB: f64 = 0.5;
/// How far sweep #2's source arrival may stray from `p1 + spacing` (clock mismatch).
const ALIGN_SEARCH_S: f64 = 0.0015;
/// Envelope pattern compared when aligning sweep #2 onto sweep #1: before / after the peak.
const PATTERN_PRE_S: f64 = 0.001;
const PATTERN_POST_S: f64 = 0.030;
/// After a half-maximum crossing, the peak is the maximum within this window.
const CLIMB_S: f64 = 0.001;

/// One sample of the faded log sweep at continuous time `t` (seconds), unit amplitude.
fn sweep_sample(t: f64, duration_s: f64, f0: f64, f1: f64) -> f64 {
    if !(0.0..duration_s).contains(&t) {
        return 0.0;
    }
    let ratio = f1 / f0;
    let phase = if ratio > 1.000_001 {
        let k = ratio.ln();
        std::f64::consts::TAU * f0 * duration_s / k * ((t * k / duration_s).exp() - 1.0)
    } else {
        std::f64::consts::TAU * f0 * t
    };
    let fade = FADE_S.min(duration_s / 4.0);
    let window = if t < fade {
        0.5 - 0.5 * (std::f64::consts::PI * t / fade).cos()
    } else if t > duration_s - fade {
        0.5 - 0.5 * (std::f64::consts::PI * (duration_s - t) / fade).cos()
    } else {
        1.0
    };
    phase.sin() * window
}

/// Logarithmic sine sweep from `f0` to `f1` Hz, `duration_s` long, with short raised-cosine
/// fades at both ends, peaking at `amplitude`.
pub fn log_sweep(rate: usize, duration_s: f64, f0: f64, f1: f64, amplitude: f32) -> Vec<f32> {
    if rate == 0 || !(duration_s > 0.0) || !duration_s.is_finite() || !(f0 > 0.0) {
        return Vec::new();
    }
    let f1 = if f1.is_finite() { f1.max(f0) } else { f0 };
    let n = (duration_s * rate as f64).round() as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / rate as f64;
            (sweep_sample(t, duration_s, f0, f1) * amplitude as f64) as f32
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// Seconds from where sweep #1 starts in the recording's template correlation to its
    /// source arrival (i.e. position of the source arrival of sweep #1 in the recording).
    pub source_arrival_s: f64,
    /// Target arrival minus source arrival for sweep #2, in seconds.
    /// Positive = target is late = reduce the target's delay by this much.
    pub target_offset_s: f64,
    /// Target peak strength relative to the source peak strength (0..), for diagnostics.
    pub target_strength: f64,
    /// Peak-to-noise ratio of the weaker of the two detections (linear), for diagnostics.
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CalibrationError {
    /// The source sweep was not detected (mic muted, too quiet, wrong mic).
    SourceNotHeard,
    /// The source was heard but no second arrival was found for the target.
    TargetNotHeard,
    /// The recording is too short for the requested spacing and sweep length.
    RecordingTooShort,
}

impl std::fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CalibrationError::SourceNotHeard => "the microphone did not pick up the test sweep",
            CalibrationError::TargetNotHeard => "the output being calibrated was not heard",
            CalibrationError::RecordingTooShort => "the recording is too short",
        })
    }
}

impl std::error::Error for CalibrationError {}

/// Analyze a mono microphone `recording` at `rate` Hz. `template` is the sweep rendered at
/// the same `rate`. `spacing_s` is the exact time between the two emissions. The target
/// arrival is searched within `max_offset_s` (e.g. 1.0) of the predicted source arrival
/// of sweep #2.
pub fn analyze(
    recording: &[f32],
    rate: usize,
    template: &[f32],
    spacing_s: f64,
    max_offset_s: f64,
) -> Result<Measurement, CalibrationError> {
    // Invalid parameters cannot describe a usable recording.
    if rate == 0 || template.is_empty() || !(spacing_s > 0.0) || !spacing_s.is_finite() {
        return Err(CalibrationError::RecordingTooShort);
    }
    let max_offset_s = if max_offset_s.is_finite() { max_offset_s.max(0.0) } else { 1.0 };
    let r = rate as f64;
    let len = recording.len();
    let spacing_n = (spacing_s * r).round() as usize;
    if spacing_n == 0 || len < spacing_n + template.len() {
        return Err(CalibrationError::RecordingTooShort);
    }

    let env = envelope(recording, template);
    let floor = median(&env);
    let detect = DETECT_RATIO * floor;
    let radius = ((ALIGN_SEARCH_S * r).ceil() as usize).max(1);
    let climb_n = ((CLIMB_S * r).ceil() as usize).max(1);

    // --- Source arrival of sweep #1: the lag whose envelope repeats `spacing` later.
    let dilated = sliding_max(&env, radius);
    let pairs = len - spacing_n;
    let score: Vec<f64> = (0..pairs).map(|k| env[k].min(dilated[k + spacing_n])).collect();
    let best = score.iter().copied().fold(0.0, f64::max);
    if !(best > 0.0) || best < detect {
        return Err(CalibrationError::SourceNotHeard);
    }
    let first = score.iter().position(|&s| s >= best * SIX_DB).unwrap_or(0);
    let p1i = climb(&env, first, climb_n, len - 1);
    let p1_val = env[p1i];
    if !(p1_val >= detect) {
        return Err(CalibrationError::SourceNotHeard);
    }
    let p1 = p1i as f64 + parabolic(&env, p1i);
    if p1i + spacing_n + template.len() > len {
        return Err(CalibrationError::RecordingTooShort);
    }

    // --- Align sweep #1's pattern onto sweep #2, within the drift window.
    let pre = (PATTERN_PRE_S * r).round() as isize;
    let post = (PATTERN_POST_S * r).round() as isize;
    let d_max = radius as isize;
    let at = |i: isize| -> f64 {
        if i >= 0 && (i as usize) < len { env[i as usize] } else { 0.0 }
    };
    let align: Vec<f64> = (-d_max..=d_max)
        .map(|d| {
            (-pre..=post)
                .map(|j| at(p1i as isize + j) * at(p1i as isize + spacing_n as isize + d + j))
                .sum()
        })
        .collect();
    let best_d = argmax(&align);
    let shift = spacing_n as f64 + (best_d as isize - d_max) as f64 + parabolic(&align, best_d);
    let s2 = p1 + shift;

    // --- Target: what remains near sweep #2 once sweep #1's pattern is subtracted.
    let max_off_n = (max_offset_s * r).round() as isize;
    let center = s2.round() as isize;
    let lo = (center - max_off_n).max(0) as usize;
    let hi = ((center + max_off_n).max(0) as usize).min(len - 1);
    if lo > hi {
        return Err(CalibrationError::TargetNotHeard);
    }
    let residual: Vec<f64> = (lo..=hi)
        .map(|k| env[k] - interpolate(&env, k as f64 - shift))
        .collect();
    let d_best = residual.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(d_best >= MIN_TARGET_REL * p1_val) || d_best < detect {
        return Err(CalibrationError::TargetNotHeard);
    }
    let t_first = residual.iter().position(|&v| v >= d_best * SIX_DB).unwrap_or(0);
    let t_peak = climb(&residual, t_first, climb_n, residual.len() - 1);
    let t = (lo + t_peak) as f64 + parabolic(&residual, t_peak);
    let target_val = residual[t_peak];

    Ok(Measurement {
        source_arrival_s: p1 / r,
        target_offset_s: (t - s2) / r,
        target_strength: target_val / p1_val,
        confidence: if floor > 0.0 { p1_val.min(target_val) / floor } else { f64::INFINITY },
    })
}

/// Envelope of the regularized deconvolution of `recording` by `template`, one value per
/// lag (the position where the template starts in the recording).
fn envelope(recording: &[f32], template: &[f32]) -> Vec<f64> {
    let n = (recording.len() + template.len()).next_power_of_two();
    let to_complex = |s: &[f32]| -> Vec<Complex<f64>> {
        let mut v: Vec<Complex<f64>> = s
            .iter()
            .map(|&x| Complex::new(if x.is_finite() { x as f64 } else { 0.0 }, 0.0))
            .collect();
        v.resize(n, Complex::new(0.0, 0.0));
        v
    };
    let mut planner = FftPlanner::<f64>::new();
    let forward = planner.plan_fft_forward(n);
    let inverse = planner.plan_fft_inverse(n);
    let mut x = to_complex(recording);
    let mut t = to_complex(template);
    forward.process(&mut x);
    forward.process(&mut t);

    let max_power = t.iter().map(|c| c.norm_sqr()).fold(0.0, f64::max);
    if !(max_power > 0.0) {
        return vec![0.0; recording.len()];
    }
    let reg = REGULARIZATION * max_power;
    for (k, (xk, tk)) in x.iter_mut().zip(&t).enumerate() {
        // Analytic signal: keep DC and Nyquist, double positive bins, drop negative ones.
        let w = if k == 0 || k == n / 2 {
            1.0
        } else if k < n / 2 {
            2.0
        } else {
            0.0
        };
        *xk = *xk * tk.conj() * (w / (tk.norm_sqr() + reg));
    }
    inverse.process(&mut x);
    let scale = 1.0 / n as f64;
    x[..recording.len()].iter().map(|c| c.norm() * scale).collect()
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    let mid = v.len() / 2;
    let (_, m, _) = v.select_nth_unstable_by(mid, f64::total_cmp);
    *m
}

/// Maximum of `v` over `[i - radius, i + radius]` for each `i` (monotonic deque, O(n)).
fn sliding_max(v: &[f64], radius: usize) -> Vec<f64> {
    let n = v.len();
    let mut out = vec![0.0; n];
    let mut deque: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let mut next = 0;
    for (i, slot) in out.iter_mut().enumerate() {
        let right = (i + radius).min(n - 1);
        while next <= right {
            while deque.back().is_some_and(|&b| v[b] <= v[next]) {
                deque.pop_back();
            }
            deque.push_back(next);
            next += 1;
        }
        let left = i.saturating_sub(radius);
        while deque.front().is_some_and(|&f| f < left) {
            deque.pop_front();
        }
        *slot = deque.front().map_or(0.0, |&f| v[f]);
    }
    out
}

fn argmax(v: &[f64]) -> usize {
    let mut best = 0;
    for (i, &x) in v.iter().enumerate() {
        if x > v[best] {
            best = i;
        }
    }
    best
}

/// Index of the maximum of `v` within `[from, min(from + window, last)]`.
fn climb(v: &[f64], from: usize, window: usize, last: usize) -> usize {
    let end = (from + window).min(last);
    let mut best = from.min(last);
    for i in best..=end {
        if v[i] > v[best] {
            best = i;
        }
    }
    best
}

/// Sub-sample offset of the peak at `i` from a parabola through its neighbours, in [-0.5, 0.5].
fn parabolic(v: &[f64], i: usize) -> f64 {
    if i == 0 || i + 1 >= v.len() {
        return 0.0;
    }
    let (a, b, c) = (v[i - 1], v[i], v[i + 1]);
    let denom = a - 2.0 * b + c;
    if denom.abs() < f64::EPSILON || !denom.is_finite() {
        return 0.0;
    }
    (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
}

/// Linear interpolation of `v` at fractional index `x`; zero outside the slice.
fn interpolate(v: &[f64], x: f64) -> f64 {
    if !(x >= 0.0) {
        return 0.0;
    }
    let i = x.floor() as usize;
    if i + 1 >= v.len() {
        return if i < v.len() { v[i] } else { 0.0 };
    }
    let f = x - i as f64;
    v[i] * (1.0 - f) + v[i + 1] * f
}

#[cfg(test)]
mod tests {
    use super::*;

    const F0: f64 = 100.0;
    const F1: f64 = 10_000.0;
    const DUR: f64 = 1.0;
    const AMP: f64 = 0.3;
    const SPACING: f64 = 2.0;

    /// Tiny deterministic RNG (xorshift64*) with Box-Muller Gaussian.
    struct Rng(u64);
    impl Rng {
        fn next_f64(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
            ((x >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        }
        fn gauss(&mut self) -> f64 {
            let (u1, u2) = (self.next_f64(), self.next_f64());
            (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
        }
    }

    /// Add one sweep starting at `start_s` (fractional, continuous time) scaled by `gain`.
    fn add_sweep(buf: &mut [f32], rate: usize, start_s: f64, gain: f64) {
        let r = rate as f64;
        let first = (start_s * r).floor().max(0.0) as usize;
        let last = (((start_s + DUR) * r).ceil() as usize + 1).min(buf.len());
        for (i, s) in buf.iter_mut().enumerate().take(last).skip(first) {
            let t = i as f64 / r - start_s;
            *s += (gain * AMP * sweep_sample(t, DUR, F0, F1)) as f32;
        }
    }

    struct Scenario {
        rate: usize,
        t0: f64,
        /// Clock mismatch: the real spacing is `SPACING * (1 + drift_ppm * 1e-6)`.
        drift_ppm: f64,
        src_gain: f64,
        /// `None` = the target stays silent in sweep #2 too.
        tgt_gain: Option<f64>,
        offset: f64,
        noise: f64,
        /// (delay after direct path in seconds, relative gain).
        src_reflections: Vec<(f64, f64)>,
        tgt_reflections: Vec<(f64, f64)>,
        total_s: f64,
        seed: u64,
    }

    impl Default for Scenario {
        fn default() -> Self {
            Self {
                rate: 48_000,
                t0: 0.4137,
                drift_ppm: 60.0,
                src_gain: 1.0,
                tgt_gain: Some(0.3),
                offset: 0.18,
                noise: 0.05,
                src_reflections: vec![(0.0062, 0.45), (0.0187, 0.3)],
                tgt_reflections: vec![(0.0091, 0.4), (0.0233, 0.25)],
                total_s: 4.8,
                seed: 0x9E37_79B9_7F4A_7C15,
            }
        }
    }

    impl Scenario {
        fn render(&self) -> Vec<f32> {
            let mut buf = vec![0.0f32; (self.total_s * self.rate as f64) as usize];
            let spacing = SPACING * (1.0 + self.drift_ppm * 1e-6);
            let direct = [(0.0, 1.0)];
            for &(d, g) in direct.iter().chain(&self.src_reflections) {
                add_sweep(&mut buf, self.rate, self.t0 + d, self.src_gain * g);
                add_sweep(&mut buf, self.rate, self.t0 + spacing + d, self.src_gain * g);
            }
            if let Some(tg) = self.tgt_gain {
                for &(d, g) in direct.iter().chain(&self.tgt_reflections) {
                    add_sweep(&mut buf, self.rate, self.t0 + spacing + self.offset + d, tg * g);
                }
            }
            let mut rng = Rng(self.seed);
            for s in &mut buf {
                *s += (self.noise * rng.gauss()) as f32;
            }
            buf
        }

        fn analyze(&self) -> Result<Measurement, CalibrationError> {
            let template = log_sweep(self.rate, DUR, F0, F1, AMP as f32);
            analyze(&self.render(), self.rate, &template, SPACING, 1.0)
        }

        /// Run and assert the offset and the source arrival are within 0.25 ms.
        fn check(&self) -> Measurement {
            let m = self.analyze().expect("measurement");
            let err = (m.target_offset_s - self.offset).abs();
            assert!(err < 0.000_25, "offset {} vs {} (err {err})", m.target_offset_s, self.offset);
            let src_err = (m.source_arrival_s - self.t0).abs();
            assert!(src_err < 0.000_25, "source {} vs {}", m.source_arrival_s, self.t0);
            m
        }
    }

    #[test]
    fn sweep_shape() {
        let s = log_sweep(48_000, 1.0, F0, F1, 0.5);
        assert_eq!(s.len(), 48_000);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
        assert!(s[0].abs() < 1e-6 && s[s.len() - 1].abs() < 0.01);
        assert!(log_sweep(0, 1.0, F0, F1, 0.5).is_empty());
        assert!(log_sweep(48_000, -1.0, F0, F1, 0.5).is_empty());
    }

    #[test]
    fn late_target_180ms_48k() {
        Scenario::default().check();
    }

    #[test]
    fn late_target_35ms_44k() {
        Scenario { rate: 44_100, offset: 0.035, drift_ppm: -90.0, seed: 7, ..Default::default() }
            .check();
    }

    #[test]
    fn early_target_minus_60ms_48k() {
        Scenario { offset: -0.060, seed: 11, ..Default::default() }.check();
    }

    #[test]
    fn nearly_synced_target_2ms_44k() {
        Scenario { rate: 44_100, offset: 0.002, tgt_gain: Some(0.5), seed: 13, ..Default::default() }
            .check();
    }

    #[test]
    fn target_louder_than_source() {
        Scenario { offset: 0.120, tgt_gain: Some(2.0), seed: 17, ..Default::default() }.check();
    }

    #[test]
    fn reflection_stronger_than_direct_path() {
        // The direct path is partly blocked: a reflection 8 ms later is louder. The direct
        // path still counts, for both the source and the target.
        Scenario {
            offset: 0.075,
            src_reflections: vec![(0.008, 1.3), (0.021, 0.4)],
            tgt_reflections: vec![(0.011, 1.25)],
            seed: 19,
            ..Default::default()
        }
        .check();
    }

    #[test]
    fn heavy_noise_quiet_target() {
        // Per-sample the target sweep is ~10 dB under the noise; the sweep's processing gain
        // still pulls it out.
        let m = Scenario { noise: 0.3, tgt_gain: Some(0.3), offset: 0.250, seed: 23, ..Default::default() }
            .check();
        assert!(m.confidence > DETECT_RATIO);
    }

    #[test]
    fn missing_target_is_reported() {
        let r = Scenario { tgt_gain: None, seed: 29, ..Default::default() }.analyze();
        assert_eq!(r, Err(CalibrationError::TargetNotHeard));
    }

    #[test]
    fn pure_noise_is_not_a_source() {
        let rate = 48_000;
        let mut rng = Rng(31);
        let rec: Vec<f32> = (0..rate * 5).map(|_| (0.1 * rng.gauss()) as f32).collect();
        let template = log_sweep(rate, DUR, F0, F1, AMP as f32);
        assert_eq!(analyze(&rec, rate, &template, SPACING, 1.0), Err(CalibrationError::SourceNotHeard));
        let silence = vec![0.0f32; rate * 5];
        assert_eq!(
            analyze(&silence, rate, &template, SPACING, 1.0),
            Err(CalibrationError::SourceNotHeard)
        );
    }

    #[test]
    fn too_short_recordings() {
        let rate = 48_000;
        let template = log_sweep(rate, DUR, F0, F1, AMP as f32);
        let short = vec![0.0f32; rate * 2];
        assert_eq!(analyze(&short, rate, &template, SPACING, 1.0), Err(CalibrationError::RecordingTooShort));
        assert_eq!(analyze(&[], rate, &template, SPACING, 1.0), Err(CalibrationError::RecordingTooShort));
        assert_eq!(analyze(&short, rate, &[], SPACING, 1.0), Err(CalibrationError::RecordingTooShort));
        assert_eq!(analyze(&short, 0, &template, SPACING, 1.0), Err(CalibrationError::RecordingTooShort));
        // Sweep #2 cut off by the end of the recording.
        let cut = Scenario { total_s: 3.0, seed: 37, ..Default::default() };
        assert_eq!(cut.analyze(), Err(CalibrationError::RecordingTooShort));
    }
}
