use rifflab_analysis::pitch::yin::YinDetector;
use rifflab_core::analysis::PitchFrame;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Analysis thread that runs pitch detection off the real-time audio path.
pub struct AnalysisThread {
    handle: Option<std::thread::JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    /// Shared noise floor (RMS, stored as f32 bits via AtomicU32).
    noise_floor: Arc<AtomicU32>,
}

impl AnalysisThread {
    /// Spawn the analysis thread.
    ///
    /// Returns `(Self, Producer<f32>)` — the producer is moved into the audio callback.
    pub fn new(
        sample_rate: u32,
        pitch_tx: rifflab_core::rtrb::Producer<PitchFrame>,
    ) -> (Self, rifflab_core::rtrb::Producer<f32>) {
        let (audio_tx, audio_rx) = rifflab_core::rtrb::RingBuffer::<f32>::new(8192);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let noise_floor = Arc::new(AtomicU32::new(0.02f32.to_bits()));
        let stop = Arc::clone(&stop_flag);
        let nf = Arc::clone(&noise_floor);

        let handle = std::thread::Builder::new()
            .name("rifflab-analysis".into())
            .spawn(move || {
                analysis_loop(audio_rx, pitch_tx, sample_rate, stop, nf);
            })
            .expect("Failed to spawn analysis thread");

        (
            Self {
                handle: Some(handle),
                stop_flag,
                noise_floor,
            },
            audio_tx,
        )
    }

    /// Update the noise floor threshold (called from UI thread).
    pub fn set_noise_floor(&self, rms: f32) {
        self.noise_floor.store(rms.to_bits(), Ordering::Relaxed);
    }

    /// Stop the analysis thread and wait for it to finish.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AnalysisThread {
    fn drop(&mut self) {
        self.stop();
    }
}

fn analysis_loop(
    mut audio_rx: rifflab_core::rtrb::Consumer<f32>,
    mut pitch_tx: rifflab_core::rtrb::Producer<PitchFrame>,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    noise_floor: Arc<AtomicU32>,
) {
    let mut detector = YinDetector::new(sample_rate);
    let hop_size = 1024;
    let mut buf = vec![0.0f32; hop_size];

    while !stop.load(Ordering::Relaxed) {
        if audio_rx.slots() >= hop_size {
            for s in &mut buf {
                *s = audio_rx.pop().unwrap();
            }

            let nf = f32::from_bits(noise_floor.load(Ordering::Relaxed));
            let rms = (buf.iter().map(|s| s * s).sum::<f32>() / hop_size as f32).sqrt();
            let frame = if rms < nf {
                PitchFrame {
                    frequency_hz: 0.0,
                    confidence: 0.0,
                    midi_note: 0,
                    cents_deviation: 0.0,
                }
            } else {
                detector.detect(&buf)
            };
            let _ = pitch_tx.push(frame);
        } else {
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
    }
}
