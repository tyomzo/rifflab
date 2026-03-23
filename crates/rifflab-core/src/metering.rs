use serde::{Deserialize, Serialize};

/// Per-bus metering data, sent from the audio thread to the UI via ring buffer.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct MeterData {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

impl MeterData {
    /// Compute peak and RMS from an interleaved stereo buffer.
    pub fn from_interleaved(buffer: &[f32]) -> Self {
        if buffer.is_empty() {
            return Self::default();
        }

        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;
        let mut sum_sq_l: f64 = 0.0;
        let mut sum_sq_r: f64 = 0.0;
        let mut count = 0u32;

        for chunk in buffer.chunks_exact(2) {
            let l = chunk[0];
            let r = chunk[1];
            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
            sum_sq_l += (l as f64) * (l as f64);
            sum_sq_r += (r as f64) * (r as f64);
            count += 1;
        }

        let rms_l = if count > 0 {
            (sum_sq_l / count as f64).sqrt() as f32
        } else {
            0.0
        };
        let rms_r = if count > 0 {
            (sum_sq_r / count as f64).sqrt() as f32
        } else {
            0.0
        };

        Self {
            peak_l,
            peak_r,
            rms_l,
            rms_r,
        }
    }

    /// Compute from a mono buffer.
    pub fn from_mono(buffer: &[f32]) -> Self {
        if buffer.is_empty() {
            return Self::default();
        }

        let mut peak: f32 = 0.0;
        let mut sum_sq: f64 = 0.0;

        for &s in buffer {
            peak = peak.max(s.abs());
            sum_sq += (s as f64) * (s as f64);
        }

        let rms = (sum_sq / buffer.len() as f64).sqrt() as f32;

        Self {
            peak_l: peak,
            peak_r: peak,
            rms_l: rms,
            rms_r: rms,
        }
    }

    /// Convert peak to dBFS.
    pub fn peak_db_l(&self) -> f32 {
        to_db(self.peak_l)
    }

    pub fn peak_db_r(&self) -> f32 {
        to_db(self.peak_r)
    }

    pub fn rms_db_l(&self) -> f32 {
        to_db(self.rms_l)
    }

    pub fn rms_db_r(&self) -> f32 {
        to_db(self.rms_r)
    }
}

fn to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -f32::INFINITY
    } else {
        20.0 * linear.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meter_from_mono() {
        let buffer = vec![0.5, -0.5, 0.5, -0.5];
        let meter = MeterData::from_mono(&buffer);
        assert!((meter.peak_l - 0.5).abs() < 1e-6);
        assert!((meter.rms_l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_meter_silence() {
        let buffer = vec![0.0; 256];
        let meter = MeterData::from_mono(&buffer);
        assert_eq!(meter.peak_l, 0.0);
        assert_eq!(meter.rms_l, 0.0);
    }

    #[test]
    fn test_db_conversion() {
        assert!((to_db(1.0)).abs() < 1e-6); // 0 dBFS
        assert!((to_db(0.5) - (-6.0206)).abs() < 0.01); // ~-6 dBFS
    }
}
