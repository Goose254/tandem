//! Clock-drift controller.
//!
//! The source and target devices run on independent crystals, so a fixed delay slowly
//! slips (typically tens of ppm, i.e. a few ms per minute). This PI controller watches how
//! much audio is buffered between the two devices and nudges the resampling ratio so the
//! buffered amount stays at the requested delay.

/// What the render loop should do after a measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Set the resampler's relative ratio (output/input) to this value.
    Ratio(f64),
    /// The error is too large to correct smoothly: insert (positive) or drop (negative)
    /// this many seconds of audio. Positive means "too little buffered".
    Resync(f64),
}

pub struct DriftController {
    kp: f64,
    ki: f64,
    tau: f64,
    max_correction: f64,
    resync_threshold: f64,
    filtered: Option<f64>,
    integral: f64,
    last_correction: f64,
}

impl Default for DriftController {
    fn default() -> Self {
        Self {
            // Proportional gain: 10 ms of error -> 1000 ppm. Closed-loop time constant ~10 s.
            kp: 0.1,
            // Integral gain, chosen for critical damping (kp^2 / 4). Learns the steady drift.
            ki: 0.0025,
            // Low-pass on the measurement, to smooth packet-sized sawtooth jitter.
            tau: 1.0,
            // +/-1000 ppm is under 2 cents of pitch shift: inaudible.
            max_correction: 1e-3,
            // Beyond this, jump instead of slowly resampling.
            resync_threshold: 0.05,
            filtered: None,
            integral: 0.0,
            last_correction: 0.0,
        }
    }
}

impl DriftController {
    /// Feed one measurement of the buffered latency (seconds), the target (seconds)
    /// and the time elapsed since the previous call (seconds).
    pub fn update(&mut self, measured: f64, target: f64, dt: f64) -> Action {
        let raw_err = measured - target;
        if raw_err.abs() > self.resync_threshold {
            self.filtered = None;
            return Action::Resync(-raw_err);
        }
        let f = match self.filtered {
            None => measured,
            Some(prev) => prev + (measured - prev) * (dt / self.tau).min(1.0),
        };
        self.filtered = Some(f);
        let err = f - target;
        self.integral = (self.integral + self.ki * err * dt)
            .clamp(-self.max_correction, self.max_correction);
        let correction = (self.kp * err + self.integral)
            .clamp(-self.max_correction, self.max_correction);
        self.last_correction = correction;
        // Too much buffered (err > 0) -> consume input faster -> fewer output frames per input.
        Action::Ratio(1.0 - correction)
    }

    /// Tell the controller the buffer was shifted on purpose (delay change or resync).
    pub fn shift(&mut self, seconds: f64) {
        if let Some(f) = self.filtered.as_mut() {
            *f += seconds;
        }
    }

    /// Forget the filtered measurement but keep the learned drift.
    pub fn reset_filter(&mut self) {
        self.filtered = None;
    }

    /// Current rate correction in ppm (positive = consuming the source faster).
    pub fn correction_ppm(&self) -> f64 {
        self.last_correction * 1e6
    }

    /// Smoothed buffered latency in seconds, if known.
    pub fn filtered(&self) -> Option<f64> {
        self.filtered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulate a source clock that runs `drift_ppm` faster than the target, with
    /// packet-sized sawtooth noise on the measurement, and check the buffer settles.
    fn simulate(drift_ppm: f64, start_err: f64) -> (f64, f64) {
        let mut ctl = DriftController::default();
        let target = 0.2;
        let dt = 0.01;
        let drift = drift_ppm * 1e-6;
        let mut latency = target + start_err;
        let mut ratio = 1.0;
        for step in 0..60_000 {
            // Buffer grows with the drift and shrinks with how fast we consume.
            latency += (drift - (1.0 / ratio - 1.0)) * dt;
            let sawtooth = (step % 3) as f64 * 0.005;
            match ctl.update(latency + sawtooth, target + 0.005, dt) {
                Action::Ratio(r) => ratio = r,
                Action::Resync(s) => latency += s,
            }
        }
        (latency - target, ctl.correction_ppm())
    }

    #[test]
    fn locks_onto_positive_drift() {
        let (err, ppm) = simulate(100.0, 0.0);
        assert!(err.abs() < 0.0005, "residual error {err}");
        assert!((ppm - 100.0).abs() < 5.0, "correction {ppm} ppm");
    }

    #[test]
    fn locks_onto_negative_drift() {
        let (err, ppm) = simulate(-80.0, 0.02);
        assert!(err.abs() < 0.0005, "residual error {err}");
        assert!((ppm + 80.0).abs() < 5.0, "correction {ppm} ppm");
    }

    #[test]
    fn large_error_triggers_resync() {
        let mut ctl = DriftController::default();
        let action = ctl.update(0.3, 0.2, 0.01);
        assert!(
            matches!(action, Action::Resync(s) if (s + 0.1).abs() < 1e-9),
            "{action:?}"
        );
    }
}
