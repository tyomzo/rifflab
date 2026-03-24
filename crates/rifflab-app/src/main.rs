mod decode;

use anyhow::Result;
use eframe::egui;
use rifflab_audio::engine::AudioEngine;
use rifflab_audio::graph::node::StemPlayer;
use rifflab_core::analysis::PitchFrame;
use rifflab_core::audio::AudioConfig;
use rifflab_core::metering::MeterData;
use rifflab_core::song::StemType;
use rifflab_core::transport::TransportState;
use rifflab_practice::compare::Comparator;
use rifflab_practice::scoring::SessionScorer;
use std::sync::{Arc, Mutex};

const WAVEFORM_SIZE: usize = 4096;

fn main() -> Result<()> {
    env_logger::init();
    log::info!("Starting RiffLab");

    // Parse CLI: optional file path
    let file_path = std::env::args().nth(1);

    // Create audio engine
    let config = AudioConfig::default(); // 48kHz, 256 samples
    let target_rate = config.sample_rate.as_u32();
    let (engine, meter_rx, pitch_rx) = AudioEngine::new(config);
    let engine = Arc::new(Mutex::new(engine));

    // Decode file if provided, resample to engine's sample rate
    let decoded = if let Some(ref path) = file_path {
        let path = std::path::Path::new(path);
        match decode::decode_file(path) {
            Ok(d) => {
                log::info!("Loaded: {} ({} frames, {}ch, {}Hz)",
                    path.display(), d.frames, d.channels, d.sample_rate);
                let d = decode::resample(d, target_rate);
                Some(d)
            }
            Err(e) => {
                log::error!("Failed to decode {}: {e}", path.display());
                None
            }
        }
    } else {
        None
    };

    // Load into engine if decoded
    if let Some(ref audio) = decoded {
        let eng = engine.lock().unwrap();
        let player = StemPlayer::new(StemType::Other, audio.data.clone(), audio.channels);
        let total_frames = player.total_frames();
        eng.graph().lock().unwrap().load_stems(vec![player]);
        eng.transport().set_length(total_frames);
    }

    // Start engine
    {
        let mut eng = engine.lock().unwrap();
        match eng.start() {
            Ok(()) => log::info!("Audio engine started"),
            Err(e) => log::error!("Failed to start engine: {e}"),
        }
    }

    // Launch UI
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 600.0])
            .with_title("RiffLab"),
        ..Default::default()
    };

    let app_engine = Arc::clone(&engine);
    eframe::run_native(
        "RiffLab",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(RiffLabApp::new(app_engine, meter_rx, pitch_rx, file_path)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("UI error: {e}"))?;

    Ok(())
}

// ─── App ──────────────────────────────────────────────────────────────────

struct RiffLabApp {
    engine: Arc<Mutex<AudioEngine>>,
    meter_rx: rifflab_core::rtrb::Consumer<MeterData>,
    pitch_rx: rifflab_core::rtrb::Consumer<PitchFrame>,
    file_name: String,
    /// Waveform display buffer.
    waveform: Vec<f32>,
    waveform_pos: usize,
    /// Latest meter readings.
    peak: f32,
    rms: f32,
    /// Seek position (0.0–1.0) for the slider.
    #[allow(dead_code)]
    seek_frac: f32,
    /// Latest detected pitch frame.
    current_pitch: PitchFrame,
    /// Real-time comparator (active when reference notes are loaded).
    comparator: Option<Comparator>,
    /// Session scoring accumulator.
    scorer: SessionScorer,
}

impl RiffLabApp {
    fn new(
        engine: Arc<Mutex<AudioEngine>>,
        meter_rx: rifflab_core::rtrb::Consumer<MeterData>,
        pitch_rx: rifflab_core::rtrb::Consumer<PitchFrame>,
        file_path: Option<String>,
    ) -> Self {
        let file_name = file_path
            .as_ref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("No file loaded")
            .to_string();

        Self {
            engine,
            meter_rx,
            pitch_rx,
            file_name,
            waveform: vec![0.0; WAVEFORM_SIZE],
            waveform_pos: 0,
            peak: 0.0,
            rms: 0.0,
            seek_frac: 0.0,
            current_pitch: PitchFrame::default(),
            comparator: None,
            scorer: SessionScorer::new(),
        }
    }

    /// Load a reference note sequence for practice comparison.
    #[allow(dead_code)]
    pub fn load_reference(&mut self, reference: Vec<rifflab_core::analysis::NoteEvent>) {
        self.comparator = Some(Comparator::new(reference));
        self.scorer = SessionScorer::new();
    }
}

impl eframe::App for RiffLabApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain meter data
        while let Ok(meter) = self.meter_rx.pop() {
            self.peak = self.peak * 0.85 + meter.peak_l * 0.15;
            self.rms = self.rms * 0.85 + meter.rms_l * 0.15;
            // Feed waveform from peak data (simplified — real waveform would use a separate buffer)
            self.waveform[self.waveform_pos] = meter.peak_l;
            self.waveform_pos = (self.waveform_pos + 1) % WAVEFORM_SIZE;
        }

        // Read transport state
        let (state, position, is_running) = {
            let eng = self.engine.lock().unwrap();
            let state = eng.transport().state();
            let position = eng.transport().position();
            let running = eng.is_running();
            (state, position, running)
        };
        let position_secs = position.seconds();

        // Drain pitch frames, run comparison, update score
        while let Ok(pitch) = self.pitch_rx.pop() {
            self.current_pitch = pitch.clone();
            if let Some(ref mut comparator) = self.comparator {
                let comparison = comparator.compare(&pitch, &position);
                self.scorer.feed(comparison);
            }
        }

        // ─── Toolbar ──────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("RiffLab");
                ui.separator();
                ui.label(&self.file_name);
                ui.separator();

                let mut eng = self.engine.lock().unwrap();

                // Play
                let play_text = if state == TransportState::Playing { "⏸ Pause" } else { "⏵ Play" };
                if ui.button(play_text).clicked() {
                    if state == TransportState::Playing {
                        eng.transport_mut().pause();
                    } else {
                        eng.transport_mut().play();
                    }
                }

                // Stop
                if ui.button("⏹ Stop").clicked() {
                    eng.transport_mut().stop();
                }

                ui.separator();

                // Position display
                let mins = (position_secs / 60.0) as u32;
                let secs = position_secs % 60.0;
                ui.monospace(format!("{:02}:{:05.2}", mins, secs));
            });
        });

        // ─── Status bar ───────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if is_running {
                    ui.label(format!("Peak: {:.1} dB", to_db(self.peak)));
                    ui.separator();
                    ui.label(format!("RMS: {:.1} dB", to_db(self.rms)));
                    ui.separator();
                    match state {
                        TransportState::Playing => ui.label("Playing"),
                        TransportState::Paused => ui.label("Paused"),
                        TransportState::Stopped => ui.label("Stopped"),
                    };
                    ui.separator();
                    // Pitch display
                    let note = self.current_pitch.note_name();
                    let cents = self.current_pitch.cents_deviation;
                    let cents_str = if cents >= 0.0 {
                        format!("+{:.0}", cents)
                    } else {
                        format!("{:.0}", cents)
                    };
                    ui.monospace(format!("Pitch: {} ({}c)", note, cents_str));
                    ui.separator();
                    // Score display
                    ui.label(format!(
                        "Score: {:.0} | Notes: {}/{}",
                        self.scorer.score(),
                        self.scorer.notes_correct(),
                        self.scorer.notes_total(),
                    ));
                } else {
                    ui.label("Engine not running");
                }
            });
        });

        // ─── Main canvas ──────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            // Waveform
            let available = ui.available_size();
            let waveform_height = (available.y * 0.6).max(100.0);

            let (response, painter) =
                ui.allocate_painter(egui::vec2(available.x, waveform_height), egui::Sense::click());
            let rect = response.rect;

            // Background
            painter.rect_filled(rect, 4.0, egui::Color32::from_gray(20));

            // Center line
            let center_y = rect.center().y;
            painter.line_segment(
                [egui::pos2(rect.left(), center_y), egui::pos2(rect.right(), center_y)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
            );

            // Draw waveform
            let width = rect.width();
            let height = rect.height();
            let step = WAVEFORM_SIZE as f32 / width;

            let points: Vec<egui::Pos2> = (0..width as usize)
                .map(|i| {
                    let idx = ((i as f32 * step) as usize) % WAVEFORM_SIZE;
                    let sample = self.waveform[idx];
                    let x = rect.left() + i as f32;
                    let y = center_y - sample * (height * 0.45);
                    egui::pos2(x, y)
                })
                .collect();

            if points.len() >= 2 {
                painter.add(egui::Shape::line(
                    points,
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(0, 200, 100)),
                ));
            }

            // Click to seek
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let frac = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    // TODO: seek to position (need total length)
                    log::info!("Seek to {:.1}%", frac * 100.0);
                }
            }

            // Peak meter bar at right edge
            let meter_width = 12.0;
            let meter_rect = egui::Rect::from_min_size(
                egui::pos2(rect.right() - meter_width - 4.0, rect.top() + 4.0),
                egui::vec2(meter_width, height - 8.0),
            );
            painter.rect_filled(meter_rect, 2.0, egui::Color32::from_gray(30));

            let rms_height = self.rms.min(1.0) * meter_rect.height();
            let rms_rect = egui::Rect::from_min_size(
                egui::pos2(meter_rect.left(), meter_rect.bottom() - rms_height),
                egui::vec2(meter_width, rms_height),
            );
            painter.rect_filled(rms_rect, 2.0, egui::Color32::from_rgb(0, 150, 80));

            let peak_y = meter_rect.bottom() - self.peak.min(1.0) * meter_rect.height();
            painter.line_segment(
                [egui::pos2(meter_rect.left(), peak_y), egui::pos2(meter_rect.right(), peak_y)],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 255, 120)),
            );

            // Transport state overlay
            ui.add_space(8.0);
            if self.file_name == "No file loaded" {
                ui.label("Usage: rifflab <audio-file.wav>");
                ui.label("Run with: pw-jack cargo run -p rifflab-app -- song.wav");
            }
        });

        ctx.request_repaint();
    }
}

fn to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -f32::INFINITY
    } else {
        20.0 * linear.log10()
    }
}
