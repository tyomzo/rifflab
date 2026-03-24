/// A parameter value that smoothly interpolates toward a target.
///
/// Used for click-free parameter changes in the audio thread.
/// Call `set_target()` to start a fade, then call `next()` once per sample
/// to get the smoothed value.
#[derive(Debug, Clone)]
pub struct SmoothedParam {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl SmoothedParam {
    /// Create a new smoothed param at the given initial value.
    pub fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }

    /// Set a new target value with a fade duration in samples.
    pub fn set_target(&mut self, target: f32, fade_samples: u32) {
        if fade_samples == 0 || (target - self.current).abs() < 1e-8 {
            self.current = target;
            self.target = target;
            self.step = 0.0;
            self.remaining = 0;
        } else {
            self.target = target;
            self.step = (target - self.current) / fade_samples as f32;
            self.remaining = fade_samples;
        }
    }

    /// Set target with fade duration in milliseconds at the given sample rate.
    pub fn set_target_ms(&mut self, target: f32, fade_ms: f32, sample_rate: u32) {
        let samples = (fade_ms * sample_rate as f32 / 1000.0).round() as u32;
        self.set_target(target, samples.max(1));
    }

    /// Get the current smoothed value and advance by one sample.
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current += self.step;
            if self.remaining == 0 {
                self.current = self.target;
            }
        }
        self.current
    }

    /// Get the current value without advancing.
    pub fn current(&self) -> f32 {
        self.current
    }

    /// Whether the param is still fading.
    pub fn is_smoothing(&self) -> bool {
        self.remaining > 0
    }

    /// Jump immediately to the target (no fade).
    pub fn snap(&mut self) {
        self.current = self.target;
        self.remaining = 0;
        self.step = 0.0;
    }
}

/// Default fade duration in milliseconds for parameter changes.
pub const DEFAULT_FADE_MS: f32 = 10.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_immediate_when_zero_samples() {
        let mut p = SmoothedParam::new(0.0);
        p.set_target(1.0, 0);
        assert_eq!(p.current(), 1.0);
        assert!(!p.is_smoothing());
    }

    #[test]
    fn test_linear_ramp() {
        let mut p = SmoothedParam::new(0.0);
        p.set_target(1.0, 4);
        assert!(p.is_smoothing());

        let v1 = p.next();
        assert!((v1 - 0.25).abs() < 0.01);

        let v2 = p.next();
        assert!((v2 - 0.50).abs() < 0.01);

        let v3 = p.next();
        assert!((v3 - 0.75).abs() < 0.01);

        let v4 = p.next();
        assert!((v4 - 1.0).abs() < 0.01);

        assert!(!p.is_smoothing());
    }

    #[test]
    fn test_snap() {
        let mut p = SmoothedParam::new(0.0);
        p.set_target(1.0, 1000);
        assert!(p.is_smoothing());
        p.snap();
        assert!(!p.is_smoothing());
        assert_eq!(p.current(), 1.0);
    }

    #[test]
    fn test_fade_ms() {
        let mut p = SmoothedParam::new(0.0);
        p.set_target_ms(1.0, 10.0, 48000);
        // 10ms at 48kHz = 480 samples
        assert!(p.is_smoothing());
        for _ in 0..480 {
            p.next();
        }
        assert!(!p.is_smoothing());
        assert!((p.current() - 1.0).abs() < 0.001);
    }
}
