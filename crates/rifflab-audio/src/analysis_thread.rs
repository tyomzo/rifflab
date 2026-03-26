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
    /// Shared sample rate (can be updated after backend starts).
    sample_rate: Arc<AtomicU32>,
}

impl AnalysisThread {
    pub fn new(
        sample_rate: u32,
        pitch_tx: rifflab_core::rtrb::Producer<PitchFrame>,
    ) -> (Self, rifflab_core::rtrb::Producer<f32>) {
        let (audio_tx, audio_rx) = rifflab_core::rtrb::RingBuffer::<f32>::new(8192);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let noise_floor = Arc::new(AtomicU32::new(0.02f32.to_bits()));
        let shared_rate = Arc::new(AtomicU32::new(sample_rate));
        let stop = Arc::clone(&stop_flag);
        let nf = Arc::clone(&noise_floor);
        let sr = Arc::clone(&shared_rate);

        let handle = std::thread::Builder::new()
            .name("rifflab-analysis".into())
            .spawn(move || {
                analysis_loop(audio_rx, pitch_tx, sr, stop, nf);
            })
            .expect("Failed to spawn analysis thread");

        (
            Self {
                handle: Some(handle),
                stop_flag,
                noise_floor,
                sample_rate: shared_rate,
            },
            audio_tx,
        )
    }

    pub fn set_noise_floor(&self, rms: f32) {
        self.noise_floor.store(rms.to_bits(), Ordering::Relaxed);
    }

    /// Update the sample rate (e.g., after backend reports actual hardware rate).
    /// The analysis loop will recreate the YIN detector on the next iteration.
    pub fn set_sample_rate(&self, rate: u32) {
        log::info!("Analysis thread: sample rate updated to {}Hz", rate);
        self.sample_rate.store(rate, Ordering::Relaxed);
    }

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
    sample_rate: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    noise_floor: Arc<AtomicU32>,
) {
    let mut current_rate = sample_rate.load(Ordering::Relaxed);
    let mut detector = YinDetector::new(current_rate);
    // Larger hop = more stable pitch detection (more periods in the window).
    // 4096 at 44100Hz = 93ms = ~11 updates/sec. Very stable for tuning,
    // slight lag acceptable since tuner doesn't need real-time response.
    let hop_size = 4096;
    let mut buf = vec![0.0f32; hop_size];

    while !stop.load(Ordering::Relaxed) {
        // Check if sample rate changed — recreate detector if so
        let new_rate = sample_rate.load(Ordering::Relaxed);
        if new_rate != current_rate {
            log::info!("Analysis: recreating YIN detector for {}Hz (was {}Hz)", new_rate, current_rate);
            current_rate = new_rate;
            detector = YinDetector::new(current_rate);
        }

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
