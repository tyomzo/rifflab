use std::path::Path;

use rifflab_core::ipc::{WorkerRequest, WorkerResponse};
use rifflab_core::song::{BeatGrid, BeatMarker};
use rifflab_ipc::client::{ClientError, JobHandle, WorkerClient};

/// Beat tracking via madmom Python worker.
pub struct BeatTracker {
    client: WorkerClient,
}

impl BeatTracker {
    pub fn new(client: WorkerClient) -> Self {
        Self { client }
    }

    /// Submit a beat tracking job.
    pub fn detect_beats(
        &self,
        input_path: &Path,
        output_path: &Path,
    ) -> Result<JobHandle, ClientError> {
        let req = WorkerRequest::DetectBeats {
            input_path: input_path.to_string_lossy().into_owned(),
            output_path: output_path.to_string_lossy().into_owned(),
        };
        self.client.submit(req)
    }

    /// Poll for the next response for a given job (non-blocking).
    ///
    /// Returns `Some(response)` if one is available, `None` otherwise.
    pub fn poll_progress(&self, handle: &JobHandle) -> Option<WorkerResponse> {
        self.client.poll(handle)
    }

    /// Drain all pending responses for a given job (non-blocking).
    pub fn poll_all(&self, handle: &JobHandle) -> Vec<WorkerResponse> {
        self.client.poll_all(handle)
    }

    /// Load a beat grid from a JSON output file produced by the worker.
    ///
    /// The expected JSON format is an array of beat timestamps (in seconds):
    /// ```json
    /// { "beats": [0.5, 1.0, 1.5, 2.0, ...], "bpm": 120.0 }
    /// ```
    ///
    /// This converts the flat beat list into a `BeatGrid` with bar/beat
    /// positions assuming 4/4 time signature.
    pub fn load_beat_grid(path: &Path) -> Result<BeatGrid, anyhow::Error> {
        let contents = std::fs::read_to_string(path)?;
        let raw: RawBeatOutput = serde_json::from_str(&contents)?;

        let bpm = raw.bpm.unwrap_or_else(|| {
            // Estimate BPM from beat intervals if not provided
            estimate_bpm(&raw.beats)
        });

        let beats_per_bar = 4u32; // Assume 4/4

        let markers: Vec<BeatMarker> = raw
            .beats
            .iter()
            .enumerate()
            .map(|(i, &time)| {
                let beat_in_bar = (i as u32 % beats_per_bar) + 1;
                let bar = (i as u32 / beats_per_bar) + 1;

                // Compute local BPM from adjacent beats
                let local_bpm = if i > 0 {
                    let interval = time - raw.beats[i - 1];
                    if interval > 0.0 {
                        60.0 / interval
                    } else {
                        bpm
                    }
                } else {
                    bpm
                };

                BeatMarker {
                    time_seconds: time,
                    bar,
                    beat: beat_in_bar,
                    bpm: local_bpm,
                }
            })
            .collect();

        Ok(BeatGrid { beats: markers })
    }
}

/// Raw JSON structure from the Python beat detection worker.
#[derive(serde::Deserialize)]
struct RawBeatOutput {
    beats: Vec<f64>,
    bpm: Option<f64>,
}

/// Estimate BPM from a list of beat timestamps.
fn estimate_bpm(beats: &[f64]) -> f64 {
    if beats.len() < 2 {
        return 120.0; // Default fallback
    }

    let intervals: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    let avg_interval = intervals.iter().sum::<f64>() / intervals.len() as f64;

    if avg_interval > 0.0 {
        60.0 / avg_interval
    } else {
        120.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_beat_grid_with_bpm() {
        let json = r#"{"beats": [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0], "bpm": 120.0}"#;
        let dir = std::env::temp_dir();
        let path = dir.join("test_beats_with_bpm.json");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(json.as_bytes()).unwrap();
        }

        let grid = BeatTracker::load_beat_grid(&path).unwrap();
        assert_eq!(grid.beats.len(), 8);

        // First beat should be bar 1, beat 1
        assert_eq!(grid.beats[0].bar, 1);
        assert_eq!(grid.beats[0].beat, 1);
        assert!((grid.beats[0].time_seconds - 0.5).abs() < 1e-9);

        // Fifth beat should be bar 2, beat 1
        assert_eq!(grid.beats[4].bar, 2);
        assert_eq!(grid.beats[4].beat, 1);

        // Local BPM should be ~120
        for beat in &grid.beats[1..] {
            assert!(
                (beat.bpm - 120.0).abs() < 1.0,
                "Expected ~120 BPM, got {}",
                beat.bpm
            );
        }

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_beat_grid_without_bpm() {
        // 0.5s intervals = 120 BPM
        let json = r#"{"beats": [1.0, 1.5, 2.0, 2.5]}"#;
        let dir = std::env::temp_dir();
        let path = dir.join("test_beats_no_bpm.json");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(json.as_bytes()).unwrap();
        }

        let grid = BeatTracker::load_beat_grid(&path).unwrap();
        assert_eq!(grid.beats.len(), 4);

        // BPM should be estimated as 120
        assert!(
            (grid.beats[1].bpm - 120.0).abs() < 1.0,
            "Expected ~120 BPM, got {}",
            grid.beats[1].bpm
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_beat_grid_invalid_json() {
        let dir = std::env::temp_dir();
        let path = dir.join("test_beats_invalid.json");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"not json").unwrap();
        }

        let result = BeatTracker::load_beat_grid(&path);
        assert!(result.is_err());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_estimate_bpm_empty() {
        assert!((estimate_bpm(&[]) - 120.0).abs() < 1e-9);
    }

    #[test]
    fn test_estimate_bpm_single() {
        assert!((estimate_bpm(&[1.0]) - 120.0).abs() < 1e-9);
    }
}
