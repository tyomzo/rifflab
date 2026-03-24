mod config;
mod decode;
mod import;
mod library;

use anyhow::Result;
use clap::Parser;
use eframe::egui;
use rifflab_audio::engine::AudioEngine;
use rifflab_audio::graph::node::StemPlayer;
use rifflab_core::analysis::{NoteEvent, PitchFrame};
use rifflab_core::metering::MeterData;
use rifflab_core::practice::AccuracyBucket;
use rifflab_core::song::StemType;
use rifflab_core::transport::{LoopRegion, TransportState};
use rifflab_practice::compare::Comparator;
use rifflab_practice::scoring::SessionScorer;
use std::sync::{Arc, Mutex};

use config::AppConfig;
use library::Library;

// ─── CLI Arguments ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "rifflab", about = "Music practice workstation")]
struct Args {
    /// Audio file to load (WAV, FLAC, MP3)
    file: Option<String>,
    /// Audio backend preference
    #[arg(long, default_value = "auto")]
    backend: String,
    /// Buffer size in samples
    #[arg(long, default_value_t = 256)]
    buffer_size: u32,
}

// ─── Constants ───────────────────────────────────────────────────────────────

/// Number of source samples summarized into one overview peak entry.
const OVERVIEW_SAMPLES_PER_PEAK: usize = 128;

/// Default horizontal zoom: how many audio frames one screen pixel represents.
const DEFAULT_FRAMES_PER_PIXEL: f64 = 512.0;

/// Minimum / maximum zoom levels (frames per pixel).
const MIN_FRAMES_PER_PIXEL: f64 = 8.0;
const MAX_FRAMES_PER_PIXEL: f64 = 16384.0;

/// Sidebar width in logical pixels.
const SIDEBAR_WIDTH: f32 = 180.0;

/// Timeline ruler height in logical pixels.
const RULER_HEIGHT: f32 = 24.0;

/// Track lane height in logical pixels.
const TRACK_LANE_HEIGHT: f32 = 120.0;

/// Default bottom drawer height in logical pixels.
const DEFAULT_DRAWER_HEIGHT: f32 = 200.0;
/// Minimum bottom drawer height.
const MIN_DRAWER_HEIGHT: f32 = 80.0;
/// Maximum bottom drawer height.
const MAX_DRAWER_HEIGHT: f32 = 500.0;
/// Width of the piano keyboard strip on the left of the piano roll.
const PIANO_KEY_WIDTH: f32 = 48.0;
/// Height of each semitone row in the piano roll.
const SEMITONE_ROW_HEIGHT: f32 = 12.0;
/// Confidence threshold below which pitch frames are treated as silence.
const PITCH_CONFIDENCE_THRESHOLD: f32 = 0.5;

/// Track colors for stems.
const TRACK_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(0, 180, 120),   // green
    egui::Color32::from_rgb(60, 140, 220),   // blue
    egui::Color32::from_rgb(220, 160, 40),   // amber
    egui::Color32::from_rgb(200, 60, 100),   // rose
    egui::Color32::from_rgb(140, 80, 200),   // purple
    egui::Color32::from_rgb(80, 200, 200),   // cyan
];

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    env_logger::init();
    log::info!("Starting RiffLab");

    // 1. Parse CLI arguments with clap
    let args = Args::parse();

    // 2. Init library (creates ~/.local/share/rifflab/ tree)
    let library = match Library::init() {
        Ok(lib) => {
            log::info!("Library root: {}", lib.root.display());
            Some(lib)
        }
        Err(e) => {
            log::warn!("Library init failed (non-fatal): {e}");
            None
        }
    };

    // 3. Load config (or create defaults), apply CLI overrides
    let mut app_config = AppConfig::load_or_create_default().unwrap_or_else(|e| {
        log::warn!("Config load failed, using defaults: {e}");
        AppConfig::default()
    });
    app_config.apply_cli_overrides(&args.backend, args.buffer_size);

    // 4. Create audio engine from config
    let audio_config = app_config.to_audio_config();
    let target_rate = audio_config.sample_rate.as_u32();
    let buffer_size = audio_config.buffer_size.as_usize();
    let (engine, meter_rx, pitch_rx) = AudioEngine::new(audio_config);
    let engine = Arc::new(Mutex::new(engine));

    // 5. File path from CLI (will be loaded after UI opens via load_file())
    let file_path = args.file.clone();

    // 6. Start engine
    {
        let mut eng = engine.lock().unwrap();
        match eng.start() {
            Ok(()) => log::info!("Audio engine started"),
            Err(e) => log::error!("Failed to start engine: {e}"),
        }
    }

    // 7. Launch UI with all data connected
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 700.0])
            .with_title("RiffLab"),
        ..Default::default()
    };

    let app_engine = Arc::clone(&engine);
    eframe::run_native(
        "RiffLab",
        options,
        Box::new(move |cc| {
            // Use dark visuals
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(RiffLabApp::new(
                app_engine,
                meter_rx,
                pitch_rx,
                file_path,
                Vec::new(),
                Vec::new(),
                target_rate,
                buffer_size,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("UI error: {e}"))?;

    Ok(())
}

// ─── Waveform Overview ──────────────────────────────────────────────────────

/// Pre-computed waveform overview for efficient rendering at any zoom level.
struct WaveformOverview {
    /// (min, max) peak pairs at overview resolution.
    peaks: Vec<(f32, f32)>,
    /// The number of source frames per peak entry.
    samples_per_peak: usize,
    /// Total source frames.
    total_frames: u64,
    /// Color for this track.
    color: egui::Color32,
    /// Track name.
    name: String,
}

impl WaveformOverview {
    /// Build an overview from interleaved audio data.
    fn from_interleaved(
        data: &[f32],
        channels: u16,
        samples_per_peak: usize,
        color: egui::Color32,
        name: String,
    ) -> Self {
        let ch = channels.max(1) as usize;
        let total_frames = data.len() / ch;
        let num_peaks = total_frames.div_ceil(samples_per_peak);
        let mut peaks = Vec::with_capacity(num_peaks);

        for chunk_idx in 0..num_peaks {
            let start_frame = chunk_idx * samples_per_peak;
            let end_frame = (start_frame + samples_per_peak).min(total_frames);
            let mut min_val: f32 = 0.0;
            let mut max_val: f32 = 0.0;
            for frame in start_frame..end_frame {
                // Mono-downmix: average channels
                let mut sample = 0.0f32;
                for c in 0..ch {
                    sample += data[frame * ch + c];
                }
                sample /= ch as f32;
                min_val = min_val.min(sample);
                max_val = max_val.max(sample);
            }
            peaks.push((min_val, max_val));
        }

        Self {
            peaks,
            samples_per_peak,
            total_frames: total_frames as u64,
            color,
            name,
        }
    }

    /// Get the (min, max) peak values for a range of source frames.
    fn peaks_for_range(&self, start_frame: u64, end_frame: u64) -> (f32, f32) {
        let first_peak = (start_frame as usize) / self.samples_per_peak;
        let last_peak = (end_frame as usize).div_ceil(self.samples_per_peak);
        let first_peak = first_peak.min(self.peaks.len());
        let last_peak = last_peak.min(self.peaks.len());

        let mut min_val: f32 = 0.0;
        let mut max_val: f32 = 0.0;
        for i in first_peak..last_peak {
            let (lo, hi) = self.peaks[i];
            min_val = min_val.min(lo);
            max_val = max_val.max(hi);
        }
        (min_val, max_val)
    }
}

// ─── Loop Drag State ─────────────────────────────────────────────────────────

/// Tracks drag state for creating/editing loop regions on the timeline ruler.
#[derive(Default)]
struct LoopDragState {
    /// True if the user is currently dragging on the ruler to create a loop.
    dragging: bool,
    /// Frame position where the drag started.
    start_frame: u64,
    /// Frame position where the drag currently is.
    current_frame: u64,
}

// ─── Bottom Drawer Types ─────────────────────────────────────────────────────

/// Which tab is active in the bottom drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    PianoRoll,
    Accuracy,
    Effects,
}

/// A played note recorded during practice, for piano roll display.
#[derive(Debug, Clone)]
struct PlayedNote {
    midi_note: u8,
    start_seconds: f64,
    end_seconds: f64,
    accuracy: AccuracyBucket,
}

// ─── App ─────────────────────────────────────────────────────────────────────

struct RiffLabApp {
    engine: Arc<Mutex<AudioEngine>>,
    meter_rx: rifflab_core::rtrb::Consumer<MeterData>,
    pitch_rx: rifflab_core::rtrb::Consumer<PitchFrame>,
    file_name: String,

    /// Latest meter readings.
    peak_l: f32,
    peak_r: f32,
    rms_l: f32,
    rms_r: f32,

    /// Latest detected pitch frame.
    current_pitch: PitchFrame,

    /// Real-time comparator (active when reference notes are loaded).
    comparator: Option<Comparator>,
    /// Session scoring accumulator.
    scorer: SessionScorer,

    /// Pre-computed waveform overviews (one per stem).
    waveform_overviews: Vec<WaveformOverview>,

    /// Arrangement view zoom: frames per pixel.
    frames_per_pixel: f64,
    /// Arrangement view horizontal scroll offset in frames.
    scroll_offset_frames: f64,
    /// Whether the view should auto-follow the playhead.
    auto_follow: bool,

    /// Audio config info for status bar.
    sample_rate: u32,
    buffer_size: usize,

    /// Loop drag state for the timeline ruler.
    loop_drag: LoopDragState,

    // ── Piano roll / bottom drawer state ──
    /// Whether the bottom drawer is open.
    drawer_open: bool,
    /// Bottom drawer height in logical pixels.
    drawer_height: f32,
    /// Active tab in the bottom drawer.
    active_tab: BottomTab,
    /// Reference notes for piano roll display.
    reference_notes: Vec<NoteEvent>,
    /// Recorded played notes during practice (for piano roll display).
    played_notes: Vec<PlayedNote>,
    /// Piano roll vertical scroll offset (in MIDI note units from bottom).
    piano_roll_scroll_note: f32,
    /// Tracks the MIDI note currently being played (for note onset/offset detection).
    tracking_midi_note: u8,
    /// Whether the tracker was silent on the previous frame.
    tracking_was_silent: bool,
    /// File to load on first frame (from CLI arg).
    pending_file: Option<std::path::PathBuf>,
}

impl RiffLabApp {
    fn new(
        engine: Arc<Mutex<AudioEngine>>,
        meter_rx: rifflab_core::rtrb::Consumer<MeterData>,
        pitch_rx: rifflab_core::rtrb::Consumer<PitchFrame>,
        file_path: Option<String>,
        waveform_overviews: Vec<WaveformOverview>,
        reference_notes: Vec<NoteEvent>,
        sample_rate: u32,
        buffer_size: usize,
    ) -> Self {
        let file_name = file_path
            .as_ref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("No file loaded")
            .to_string();

        // If reference notes were transcribed, set up the comparator
        let comparator = if !reference_notes.is_empty() {
            log::info!(
                "Loaded {} reference notes into comparator",
                reference_notes.len()
            );
            Some(Comparator::new(reference_notes.clone()))
        } else {
            None
        };

        Self {
            engine,
            meter_rx,
            pitch_rx,
            file_name,
            peak_l: 0.0,
            peak_r: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
            current_pitch: PitchFrame::default(),
            comparator,
            scorer: SessionScorer::new(),
            waveform_overviews,
            frames_per_pixel: DEFAULT_FRAMES_PER_PIXEL,
            scroll_offset_frames: 0.0,
            auto_follow: true,
            sample_rate,
            buffer_size,
            loop_drag: LoopDragState::default(),
            drawer_open: true,
            drawer_height: DEFAULT_DRAWER_HEIGHT,
            active_tab: BottomTab::PianoRoll,
            reference_notes,
            played_notes: Vec::new(),
            piano_roll_scroll_note: 48.0, // C3 at bottom
            tracking_midi_note: 0,
            tracking_was_silent: true,
            pending_file: file_path.map(std::path::PathBuf::from),
        }
    }

    /// Load a reference note sequence for practice comparison.
    #[allow(dead_code)]
    pub fn load_reference(&mut self, reference: Vec<NoteEvent>) {
        self.reference_notes = reference.clone();
        self.comparator = Some(Comparator::new(reference));
        self.scorer = SessionScorer::new();
        self.played_notes.clear();
    }

    /// Convert a frame position to an x coordinate relative to the arrangement rect.
    fn frame_to_x(&self, frame: u64) -> f64 {
        (frame as f64 - self.scroll_offset_frames) / self.frames_per_pixel
    }

    /// Convert an x coordinate (relative to arrangement rect) to a frame position.
    fn x_to_frame(&self, x: f64) -> u64 {
        let frame = x * self.frames_per_pixel + self.scroll_offset_frames;
        frame.max(0.0) as u64
    }

    /// Load an audio file: decode, load into engine, compute waveform overview.
    fn load_file(&mut self, path: &std::path::Path) {
        log::info!("Loading file: {}", path.display());

        let target_rate = {
            let eng = self.engine.lock().unwrap();
            eng.transport().sample_rate()
        };

        // Decode + resample
        let decoded = match decode::decode_file(path) {
            Ok(d) => decode::resample(d, target_rate),
            Err(e) => {
                log::error!("Failed to decode {}: {e}", path.display());
                return;
            }
        };

        log::info!(
            "Loaded: {} frames, {}ch, {}Hz ({:.1}s)",
            decoded.frames, decoded.channels, decoded.sample_rate,
            decoded.frames as f64 / decoded.sample_rate as f64,
        );

        // Update file name
        self.file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        // Build waveform overview
        self.waveform_overviews = vec![WaveformOverview::from_interleaved(
            &decoded.data,
            decoded.channels,
            OVERVIEW_SAMPLES_PER_PEAK,
            TRACK_COLORS[0],
            self.file_name.clone(),
        )];

        // Load into engine
        {
            let mut eng = self.engine.lock().unwrap();
            eng.transport_mut().stop();
            let player = StemPlayer::new(StemType::Other, decoded.data, decoded.channels);
            let total_frames = player.total_frames();
            eng.graph().lock().unwrap().load_stems(vec![player]);
            eng.transport().set_length(total_frames);
        }

        // Reset practice state
        self.comparator = None;
        self.reference_notes.clear();
        self.scorer = SessionScorer::new();
        self.played_notes.clear();
        self.scroll_offset_frames = 0.0;

        log::info!("File loaded successfully: {}", self.file_name);
    }
}

impl eframe::App for RiffLabApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ─── Load pending file (from CLI arg, first frame only) ──
        if let Some(path) = self.pending_file.take() {
            self.load_file(&path);
        }

        // ─── Drain channels ──────────────────────────────────────
        while let Ok(meter) = self.meter_rx.pop() {
            // Smooth with exponential moving average
            self.peak_l = self.peak_l * 0.85 + meter.peak_l * 0.15;
            self.peak_r = self.peak_r * 0.85 + meter.peak_r * 0.15;
            self.rms_l = self.rms_l * 0.85 + meter.rms_l * 0.15;
            self.rms_r = self.rms_r * 0.85 + meter.rms_r * 0.15;
        }

        // Read transport state
        let (state, position, _total_length, is_running, loop_enabled, loop_start, loop_end) = {
            let eng = self.engine.lock().unwrap();
            let transport = eng.transport();
            (
                transport.state(),
                transport.position(),
                transport.length(),
                eng.is_running(),
                transport.loop_enabled(),
                transport.loop_start(),
                transport.loop_end(),
            )
        };
        let position_secs = position.seconds();
        let position_frame = position.frame;

        // Drain pitch frames, run comparison, update score, track played notes
        while let Ok(pitch) = self.pitch_rx.pop() {
            self.current_pitch = pitch.clone();

            // Track played notes for piano roll display
            let is_silent = pitch.frequency_hz <= 0.0
                || pitch.confidence < PITCH_CONFIDENCE_THRESHOLD;
            let current_time = position_secs;

            if is_silent {
                // Close any active note
                self.tracking_was_silent = true;
                self.tracking_midi_note = 0;
            } else {
                let is_new_note =
                    self.tracking_was_silent || pitch.midi_note != self.tracking_midi_note;
                if is_new_note {
                    // Determine accuracy from comparison or raw cents
                    let accuracy = if pitch.cents_deviation.abs() <= 5.0 {
                        AccuracyBucket::Perfect
                    } else if pitch.cents_deviation.abs() <= 15.0 {
                        AccuracyBucket::Good
                    } else if pitch.cents_deviation.abs() <= 25.0 {
                        AccuracyBucket::Acceptable
                    } else {
                        AccuracyBucket::Off
                    };
                    self.played_notes.push(PlayedNote {
                        midi_note: pitch.midi_note,
                        start_seconds: current_time,
                        end_seconds: current_time,
                        accuracy,
                    });
                    self.tracking_midi_note = pitch.midi_note;
                    self.tracking_was_silent = false;
                } else if let Some(last) = self.played_notes.last_mut() {
                    // Extend the current note
                    last.end_seconds = current_time;
                }
            }

            if let Some(ref mut comparator) = self.comparator {
                let comparison = comparator.compare(&pitch, &position);
                // Update the accuracy of the current played note from comparison data
                if comparison.reference_note.is_some() {
                    if let Some(last) = self.played_notes.last_mut() {
                        if !comparison.note_correct {
                            last.accuracy = AccuracyBucket::Off;
                        } else {
                            last.accuracy = comparison.accuracy_bucket;
                        }
                    }
                }
                self.scorer.feed(comparison);
            }
        }

        // ─── Toolbar ─────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar")
            .exact_height(36.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;

                    ui.heading("RiffLab");
                    ui.separator();

                    // Open file button
                    if ui.button("\u{1F4C2} Open").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "flac", "mp3", "ogg", "aac", "m4a"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                        {
                            self.load_file(&path);
                        }
                    }

                    ui.separator();

                    // Transport controls
                    {
                        let mut eng = self.engine.lock().unwrap();

                        let play_label = if state == TransportState::Playing {
                            "\u{23F8} Pause"
                        } else {
                            "\u{23F5} Play"
                        };
                        if ui.button(play_label).clicked() {
                            if state == TransportState::Playing {
                                eng.transport_mut().pause();
                            } else {
                                eng.transport_mut().play();
                            }
                        }

                        if ui.button("\u{23F9} Stop").clicked() {
                            eng.transport_mut().stop();
                        }
                    }

                    ui.separator();

                    // File name
                    ui.label(
                        egui::RichText::new(&self.file_name)
                            .color(egui::Color32::from_rgb(180, 200, 220)),
                    );

                    ui.separator();

                    // Position display (mm:ss.f)
                    let mins = (position_secs / 60.0) as u32;
                    let secs = position_secs % 60.0;
                    ui.monospace(
                        egui::RichText::new(format!("{:02}:{:05.2}", mins, secs))
                            .color(egui::Color32::from_rgb(120, 220, 180))
                            .size(15.0),
                    );

                    ui.separator();

                    // Visual tuner indicator
                    {
                        let note = self.current_pitch.note_name();
                        let cents = self.current_pitch.cents_deviation;
                        let has_pitch = self.current_pitch.frequency_hz > 0.0;
                        let tuner_color = if has_pitch {
                            if cents.abs() <= 5.0 {
                                egui::Color32::from_rgb(80, 220, 80)
                            } else if cents.abs() <= 15.0 {
                                egui::Color32::from_rgb(220, 200, 60)
                            } else {
                                egui::Color32::from_rgb(220, 70, 70)
                            }
                        } else {
                            egui::Color32::from_rgb(80, 80, 80)
                        };

                        // Note name (large)
                        ui.monospace(
                            egui::RichText::new(&note)
                                .size(16.0)
                                .color(tuner_color),
                        );

                        // Cents deviation bar
                        let bar_width = 80.0f32;
                        let bar_height = 8.0f32;
                        let (bar_rect, _) = ui.allocate_exact_size(
                            egui::vec2(bar_width, bar_height),
                            egui::Sense::hover(),
                        );
                        let painter = ui.painter_at(bar_rect);
                        // Bar background
                        painter.rect_filled(
                            bar_rect,
                            2.0,
                            egui::Color32::from_rgb(30, 32, 36),
                        );
                        // Center tick
                        let center_x = bar_rect.center().x;
                        painter.line_segment(
                            [
                                egui::pos2(center_x, bar_rect.top()),
                                egui::pos2(center_x, bar_rect.bottom()),
                            ],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 85, 90)),
                        );
                        if has_pitch {
                            // Deviation indicator: map -50..+50 cents to bar
                            let norm = (cents / 50.0).clamp(-1.0, 1.0);
                            let ind_x = center_x + norm * (bar_width / 2.0);
                            let ind_w = 4.0f32;
                            let ind_rect = egui::Rect::from_center_size(
                                egui::pos2(ind_x, bar_rect.center().y),
                                egui::vec2(ind_w, bar_height),
                            );
                            painter.rect_filled(ind_rect, 1.0, tuner_color);
                        }

                        // Cents text
                        if has_pitch {
                            let cents_str = if cents >= 0.0 {
                                format!("+{:.0}c", cents)
                            } else {
                                format!("{:.0}c", cents)
                            };
                            ui.monospace(
                                egui::RichText::new(cents_str)
                                    .size(10.0)
                                    .color(tuner_color),
                            );
                        }
                    }

                    ui.separator();

                    // Score display
                    ui.label(format!(
                        "Score: {:.0} | {}/{}",
                        self.scorer.score(),
                        self.scorer.notes_correct(),
                        self.scorer.notes_total(),
                    ));

                    // Right-aligned controls
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.auto_follow, "Follow");
                        ui.separator();
                        let drawer_label = if self.drawer_open { "Hide Panel" } else { "Show Panel" };
                        if ui.small_button(drawer_label).clicked() {
                            self.drawer_open = !self.drawer_open;
                        }
                    });
                });
            });

        // ─── Status bar ──────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(24.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;

                    let status_color = egui::Color32::from_rgb(160, 170, 180);

                    // Sample rate
                    ui.label(
                        egui::RichText::new(format!("{}Hz", self.sample_rate))
                            .small()
                            .color(status_color),
                    );

                    // Buffer size + latency
                    let latency_ms =
                        self.buffer_size as f64 / self.sample_rate as f64 * 1000.0;
                    ui.label(
                        egui::RichText::new(format!(
                            "buf {} ({:.1}ms)",
                            self.buffer_size, latency_ms
                        ))
                        .small()
                        .color(status_color),
                    );

                    ui.separator();

                    if is_running {
                        // Peak dB (stereo max)
                        let peak_db = to_db(self.peak_l.max(self.peak_r));
                        let peak_color = if peak_db > -3.0 {
                            egui::Color32::from_rgb(255, 80, 80)
                        } else if peak_db > -12.0 {
                            egui::Color32::from_rgb(220, 200, 60)
                        } else {
                            egui::Color32::from_rgb(80, 200, 120)
                        };
                        ui.label(
                            egui::RichText::new(format!("Peak: {:.1} dB", peak_db))
                                .small()
                                .color(peak_color),
                        );

                        // RMS dB
                        let rms_db = to_db(self.rms_l.max(self.rms_r));
                        ui.label(
                            egui::RichText::new(format!("RMS: {:.1} dB", rms_db))
                                .small()
                                .color(status_color),
                        );

                        ui.separator();

                        // Transport state
                        let (state_label, state_color) = match state {
                            TransportState::Playing => {
                                ("Playing", egui::Color32::from_rgb(80, 220, 120))
                            }
                            TransportState::Paused => {
                                ("Paused", egui::Color32::from_rgb(220, 200, 60))
                            }
                            TransportState::Stopped => {
                                ("Stopped", egui::Color32::from_rgb(160, 160, 160))
                            }
                        };
                        ui.label(
                            egui::RichText::new(state_label)
                                .small()
                                .color(state_color),
                        );

                        // Loop indicator
                        if loop_enabled && loop_end > loop_start {
                            let ls = loop_start as f64 / self.sample_rate as f64;
                            let le = loop_end as f64 / self.sample_rate as f64;
                            ui.label(
                                egui::RichText::new(format!(
                                    "Loop: {:.1}s - {:.1}s",
                                    ls, le
                                ))
                                .small()
                                .color(egui::Color32::from_rgb(180, 120, 220)),
                            );
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("Engine not running")
                                .small()
                                .color(egui::Color32::from_rgb(220, 80, 80)),
                        );
                    }

                    // Zoom info (right side)
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Zoom: {:.0} fr/px",
                                self.frames_per_pixel
                            ))
                            .small()
                            .color(status_color),
                        );
                    });
                });
            });

        // ─── Bottom Drawer (Piano Roll / Accuracy / Effects) ─────
        if self.drawer_open {
            egui::TopBottomPanel::bottom("bottom_drawer")
                .resizable(true)
                .min_height(MIN_DRAWER_HEIGHT)
                .max_height(MAX_DRAWER_HEIGHT)
                .default_height(self.drawer_height)
                .show(ctx, |ui| {
                    // Tab bar
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let tabs = [
                            (BottomTab::PianoRoll, "Piano Roll"),
                            (BottomTab::Accuracy, "Accuracy"),
                            (BottomTab::Effects, "Effects"),
                        ];
                        for (tab, label) in &tabs {
                            let active = self.active_tab == *tab;
                            let btn = egui::Button::new(
                                egui::RichText::new(*label)
                                    .size(11.0)
                                    .color(if active {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_rgb(140, 145, 150)
                                    }),
                            )
                            .fill(if active {
                                egui::Color32::from_rgb(50, 55, 65)
                            } else {
                                egui::Color32::from_rgb(30, 32, 38)
                            })
                            .corner_radius(2)
                            .min_size(egui::vec2(80.0, 20.0));
                            if ui.add(btn).clicked() {
                                self.active_tab = *tab;
                            }
                        }

                        // Close button (right-aligned)
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .small_button(
                                        egui::RichText::new("X")
                                            .size(10.0)
                                            .color(egui::Color32::from_rgb(160, 160, 160)),
                                    )
                                    .clicked()
                                {
                                    self.drawer_open = false;
                                }
                            },
                        );
                    });

                    ui.separator();

                    // Tab content
                    match self.active_tab {
                        BottomTab::PianoRoll => {
                            draw_piano_roll(
                                ui,
                                &self.reference_notes,
                                &self.played_notes,
                                self.scroll_offset_frames,
                                self.frames_per_pixel,
                                self.sample_rate,
                                position_frame,
                                self.piano_roll_scroll_note,
                            );
                        }
                        BottomTab::Accuracy => {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    egui::RichText::new("Accuracy view coming soon")
                                        .color(egui::Color32::from_rgb(100, 110, 120)),
                                );
                            });
                        }
                        BottomTab::Effects => {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    egui::RichText::new("Effects rack coming soon")
                                        .color(egui::Color32::from_rgb(100, 110, 120)),
                                );
                            });
                        }
                    }
                });
        }

        // ─── Sidebar ─────────────────────────────────────────────
        egui::SidePanel::left("sidebar")
            .exact_width(SIDEBAR_WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;

                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Tracks")
                            .strong()
                            .size(13.0)
                            .color(egui::Color32::from_rgb(200, 210, 220)),
                    );
                    ui.separator();

                    // Get stem info from graph
                    let eng = self.engine.lock().unwrap();
                    let mut graph = eng.graph().lock().unwrap();
                    let num_stems = graph.stem_players.len();

                    for i in 0..num_stems {
                        let track_color = TRACK_COLORS[i % TRACK_COLORS.len()];
                        let track_name = if i < self.waveform_overviews.len() {
                            self.waveform_overviews[i].name.clone()
                        } else {
                            format!("Track {}", i + 1)
                        };

                        ui.add_space(4.0);

                        // Track header with color indicator
                        ui.horizontal(|ui| {
                            // Color dot
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter()
                                .circle_filled(rect.center(), 4.0, track_color);

                            ui.label(
                                egui::RichText::new(&track_name)
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(220, 225, 230)),
                            );
                        });

                        // Solo / Mute buttons
                        ui.horizontal(|ui| {
                            let is_solo = graph.stem_solos.get(i).copied().unwrap_or(false);
                            let solo_color = if is_solo {
                                egui::Color32::from_rgb(220, 200, 40)
                            } else {
                                egui::Color32::from_rgb(80, 80, 80)
                            };
                            let solo_btn = egui::Button::new(
                                egui::RichText::new("S")
                                    .size(11.0)
                                    .color(if is_solo {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::from_rgb(160, 160, 160)
                                    }),
                            )
                            .fill(solo_color)
                            .min_size(egui::vec2(24.0, 18.0));
                            if ui.add(solo_btn).clicked() {
                                if let Some(s) = graph.stem_solos.get_mut(i) {
                                    *s = !*s;
                                }
                            }

                            let is_mute = graph.stem_mutes.get(i).copied().unwrap_or(false);
                            let mute_color = if is_mute {
                                egui::Color32::from_rgb(220, 60, 60)
                            } else {
                                egui::Color32::from_rgb(80, 80, 80)
                            };
                            let mute_btn = egui::Button::new(
                                egui::RichText::new("M")
                                    .size(11.0)
                                    .color(if is_mute {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_rgb(160, 160, 160)
                                    }),
                            )
                            .fill(mute_color)
                            .min_size(egui::vec2(24.0, 18.0));
                            if ui.add(mute_btn).clicked() {
                                if let Some(m) = graph.stem_mutes.get_mut(i) {
                                    *m = !*m;
                                }
                            }
                        });

                        // Volume slider
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Vol")
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(140, 140, 140)),
                            );
                            let mut vol = graph.stem_volumes.get(i).copied().unwrap_or(1.0);
                            let slider = egui::Slider::new(&mut vol, 0.0..=1.0)
                                .show_value(false)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
                            if ui.add(slider).changed() {
                                if let Some(v) = graph.stem_volumes.get_mut(i) {
                                    *v = vol;
                                }
                            }
                        });

                        if i + 1 < num_stems {
                            ui.separator();
                        }
                    }

                    // If no stems, show placeholder
                    if num_stems == 0 {
                        ui.add_space(20.0);
                        ui.label(
                            egui::RichText::new("No tracks loaded")
                                .size(11.0)
                                .color(egui::Color32::from_rgb(120, 120, 120)),
                        );
                    }

                    // Push master section to bottom
                    ui.add_space(20.0);
                    ui.separator();

                    // Master volume
                    ui.label(
                        egui::RichText::new("Master")
                            .strong()
                            .size(12.0)
                            .color(egui::Color32::from_rgb(200, 210, 220)),
                    );

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Vol")
                                .size(10.0)
                                .color(egui::Color32::from_rgb(140, 140, 140)),
                        );
                        let mut master_vol = graph.master_volume;
                        let slider = egui::Slider::new(&mut master_vol, 0.0..=1.0)
                            .show_value(false)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
                        if ui.add(slider).changed() {
                            graph.master_volume = master_vol;
                        }
                    });

                    // Stereo peak meter
                    ui.add_space(8.0);
                    draw_stereo_meter(ui, self.peak_l, self.peak_r, self.rms_l, self.rms_r);
                });
            });

        // ─── Arrangement View (Central Panel) ────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_size();

            // Handle zoom with Ctrl+Scroll
            if ui.rect_contains_pointer(ui.max_rect()) {
                let scroll_delta = ctx.input(|i| i.smooth_scroll_delta);
                let modifiers = ctx.input(|i| i.modifiers);

                if modifiers.ctrl && scroll_delta.y.abs() > 0.0 {
                    let zoom_factor = if scroll_delta.y > 0.0 { 0.85 } else { 1.18 };
                    // Zoom around mouse position
                    if let Some(mouse_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                        let arrangement_left = ui.max_rect().left();
                        let mouse_x = (mouse_pos.x - arrangement_left) as f64;
                        let frame_at_mouse = self.x_to_frame(mouse_x);

                        self.frames_per_pixel =
                            (self.frames_per_pixel * zoom_factor).clamp(MIN_FRAMES_PER_PIXEL, MAX_FRAMES_PER_PIXEL);

                        // Adjust scroll so the frame under the mouse stays in place
                        self.scroll_offset_frames =
                            frame_at_mouse as f64 - mouse_x * self.frames_per_pixel;
                        self.scroll_offset_frames = self.scroll_offset_frames.max(0.0);
                    }
                } else if !modifiers.ctrl && scroll_delta.x.abs() > 0.0 {
                    // Horizontal scroll
                    self.scroll_offset_frames -= scroll_delta.x as f64 * self.frames_per_pixel;
                    self.scroll_offset_frames = self.scroll_offset_frames.max(0.0);
                    self.auto_follow = false;
                }
            }

            // Auto-follow playhead during playback
            if self.auto_follow && state == TransportState::Playing {
                let playhead_x = self.frame_to_x(position_frame);
                let view_width = available.x as f64;
                // Keep playhead in the middle third of the view
                if playhead_x > view_width * 0.75 || playhead_x < view_width * 0.1 {
                    self.scroll_offset_frames =
                        position_frame as f64 - view_width * 0.25 * self.frames_per_pixel;
                    self.scroll_offset_frames = self.scroll_offset_frames.max(0.0);
                }
            }

            // ─── Timeline ruler ─────────────────────────────────
            let ruler_rect = {
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(available.x, RULER_HEIGHT),
                    egui::Sense::click_and_drag(),
                );
                let rect = response.rect;

                // Ruler background
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(35, 38, 42));

                // Time markings
                draw_time_ruler(
                    &painter,
                    rect,
                    self.scroll_offset_frames,
                    self.frames_per_pixel,
                    self.sample_rate,
                );

                // Loop region overlay on ruler
                if loop_enabled && loop_end > loop_start {
                    let lx0 = rect.left() + self.frame_to_x(loop_start) as f32;
                    let lx1 = rect.left() + self.frame_to_x(loop_end) as f32;
                    let loop_rect = egui::Rect::from_min_max(
                        egui::pos2(lx0.max(rect.left()), rect.top()),
                        egui::pos2(lx1.min(rect.right()), rect.bottom()),
                    );
                    painter.rect_filled(
                        loop_rect,
                        0.0,
                        egui::Color32::from_rgba_premultiplied(140, 80, 200, 50),
                    );
                    // Loop bracket lines
                    painter.line_segment(
                        [
                            egui::pos2(lx0, rect.top()),
                            egui::pos2(lx0, rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(180, 120, 240)),
                    );
                    painter.line_segment(
                        [
                            egui::pos2(lx1, rect.top()),
                            egui::pos2(lx1, rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(180, 120, 240)),
                    );
                }

                // Playhead on ruler
                let playhead_x = rect.left() + self.frame_to_x(position_frame) as f32;
                if playhead_x >= rect.left() && playhead_x <= rect.right() {
                    // Draw a small triangle at the top
                    let tri_size = 5.0;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(playhead_x - tri_size, rect.top()),
                            egui::pos2(playhead_x + tri_size, rect.top()),
                            egui::pos2(playhead_x, rect.top() + tri_size * 1.5),
                        ],
                        egui::Color32::from_rgb(220, 50, 50),
                        egui::Stroke::NONE,
                    ));
                }

                // Handle ruler click to seek, drag to create loop
                if response.drag_started() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let x = (pos.x - rect.left()) as f64;
                        let frame = self.x_to_frame(x);
                        self.loop_drag.dragging = true;
                        self.loop_drag.start_frame = frame;
                        self.loop_drag.current_frame = frame;
                    }
                }

                if response.dragged() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let x = (pos.x - rect.left()) as f64;
                        let frame = self.x_to_frame(x);
                        self.loop_drag.current_frame = frame;
                    }
                }

                if response.drag_stopped() && self.loop_drag.dragging {
                    let s = self.loop_drag.start_frame;
                    let e = self.loop_drag.current_frame;
                    let (start, end) = if s < e { (s, e) } else { (e, s) };

                    // Only set loop if the region is meaningful (> 0.05 seconds)
                    let min_frames = (self.sample_rate as f64 * 0.05) as u64;
                    if end - start > min_frames {
                        let mut eng = self.engine.lock().unwrap();
                        eng.transport_mut()
                            .set_loop(Some(LoopRegion {
                                start_frame: start,
                                end_frame: end,
                            }));
                    }
                    self.loop_drag.dragging = false;
                }

                // Single click (not drag) = seek
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let x = (pos.x - rect.left()) as f64;
                        let frame = self.x_to_frame(x);
                        let mut eng = self.engine.lock().unwrap();
                        let len = eng.transport().length();
                        eng.transport_mut().seek(frame.min(len));
                    }
                }

                // Right-click to clear loop
                if response.secondary_clicked() {
                    let mut eng = self.engine.lock().unwrap();
                    eng.transport_mut().set_loop(None);
                }

                // Drag preview overlay
                if self.loop_drag.dragging {
                    let s = self.loop_drag.start_frame;
                    let e = self.loop_drag.current_frame;
                    let (start, end) = if s < e { (s, e) } else { (e, s) };
                    let lx0 = rect.left() + self.frame_to_x(start) as f32;
                    let lx1 = rect.left() + self.frame_to_x(end) as f32;
                    let drag_rect = egui::Rect::from_min_max(
                        egui::pos2(lx0.max(rect.left()), rect.top()),
                        egui::pos2(lx1.min(rect.right()), rect.bottom()),
                    );
                    painter.rect_filled(
                        drag_rect,
                        0.0,
                        egui::Color32::from_rgba_premultiplied(180, 120, 240, 40),
                    );
                }

                rect
            };

            // ─── Waveform track lanes ───────────────────────────
            let track_area_height =
                available.y - RULER_HEIGHT - 10.0; // leave some margin
            let num_tracks = self.waveform_overviews.len().max(1);
            let lane_height =
                (track_area_height / num_tracks as f32).clamp(60.0, TRACK_LANE_HEIGHT);

            if self.waveform_overviews.is_empty() {
                // No tracks — show placeholder
                let (_, painter) = ui.allocate_painter(
                    egui::vec2(available.x, lane_height),
                    egui::Sense::hover(),
                );
                let rect = painter.clip_rect();
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(22, 24, 28));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No audio loaded. Usage: rifflab <audio-file>",
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_rgb(100, 110, 120),
                );
            } else {
                for (track_idx, overview) in self.waveform_overviews.iter().enumerate() {
                    let (response, painter) = ui.allocate_painter(
                        egui::vec2(available.x, lane_height),
                        egui::Sense::click(),
                    );
                    let rect = response.rect;

                    // Track background
                    let bg_color = if track_idx % 2 == 0 {
                        egui::Color32::from_rgb(22, 24, 28)
                    } else {
                        egui::Color32::from_rgb(26, 28, 32)
                    };
                    painter.rect_filled(rect, 0.0, bg_color);

                    // Center line
                    let center_y = rect.center().y;
                    painter.line_segment(
                        [
                            egui::pos2(rect.left(), center_y),
                            egui::pos2(rect.right(), center_y),
                        ],
                        egui::Stroke::new(
                            0.5,
                            egui::Color32::from_rgba_premultiplied(255, 255, 255, 20),
                        ),
                    );

                    // Loop region overlay on waveform
                    if loop_enabled && loop_end > loop_start {
                        let lx0 = rect.left() + self.frame_to_x(loop_start) as f32;
                        let lx1 = rect.left() + self.frame_to_x(loop_end) as f32;
                        let loop_rect = egui::Rect::from_min_max(
                            egui::pos2(lx0.max(rect.left()), rect.top()),
                            egui::pos2(lx1.min(rect.right()), rect.bottom()),
                        );
                        painter.rect_filled(
                            loop_rect,
                            0.0,
                            egui::Color32::from_rgba_premultiplied(140, 80, 200, 20),
                        );
                    }

                    // Draw waveform
                    let half_height = (rect.height() * 0.45) as f64;
                    let width = rect.width() as usize;

                    // Build the filled waveform shape as two polylines (top envelope + bottom envelope)
                    let mut top_points: Vec<egui::Pos2> = Vec::with_capacity(width);
                    let mut bottom_points: Vec<egui::Pos2> = Vec::with_capacity(width);

                    for px in 0..width {
                        let frame_start =
                            self.scroll_offset_frames + px as f64 * self.frames_per_pixel;
                        let frame_end = frame_start + self.frames_per_pixel;

                        if frame_start < 0.0 || frame_start >= overview.total_frames as f64 {
                            top_points.push(egui::pos2(rect.left() + px as f32, center_y));
                            bottom_points.push(egui::pos2(rect.left() + px as f32, center_y));
                            continue;
                        }

                        let (min_val, max_val) = overview.peaks_for_range(
                            frame_start.max(0.0) as u64,
                            (frame_end as u64).min(overview.total_frames),
                        );

                        let x = rect.left() + px as f32;
                        let y_top = center_y - (max_val as f64 * half_height) as f32;
                        let y_bot = center_y - (min_val as f64 * half_height) as f32;

                        top_points.push(egui::pos2(x, y_top));
                        bottom_points.push(egui::pos2(x, y_bot));
                    }

                    // Build a filled polygon from top_points + reversed bottom_points
                    if !top_points.is_empty() {
                        // Draw filled area
                        let waveform_fill = egui::Color32::from_rgba_premultiplied(
                            overview.color.r(),
                            overview.color.g(),
                            overview.color.b(),
                            60,
                        );
                        let waveform_stroke_color = egui::Color32::from_rgba_premultiplied(
                            overview.color.r(),
                            overview.color.g(),
                            overview.color.b(),
                            180,
                        );

                        // Draw filled columns using line segments for efficiency
                        for i in 0..top_points.len() {
                            let x = top_points[i].x;
                            let y_top = top_points[i].y;
                            let y_bot = bottom_points[i].y;
                            if (y_bot - y_top).abs() > 0.5 {
                                painter.line_segment(
                                    [egui::pos2(x, y_top), egui::pos2(x, y_bot)],
                                    egui::Stroke::new(1.0, waveform_fill),
                                );
                            }
                        }

                        // Draw top and bottom envelope lines
                        if top_points.len() >= 2 {
                            painter.add(egui::Shape::line(
                                top_points,
                                egui::Stroke::new(0.8, waveform_stroke_color),
                            ));
                            painter.add(egui::Shape::line(
                                bottom_points,
                                egui::Stroke::new(0.8, waveform_stroke_color),
                            ));
                        }
                    }

                    // Playhead line on waveform
                    let playhead_x = rect.left() + self.frame_to_x(position_frame) as f32;
                    if playhead_x >= rect.left() && playhead_x <= rect.right() {
                        painter.line_segment(
                            [
                                egui::pos2(playhead_x, rect.top()),
                                egui::pos2(playhead_x, rect.bottom()),
                            ],
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(220, 50, 50)),
                        );
                    }

                    // Click on waveform to seek
                    if response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let x = (pos.x - rect.left()) as f64;
                            let frame = self.x_to_frame(x);
                            let mut eng = self.engine.lock().unwrap();
                            let len = eng.transport().length();
                            eng.transport_mut().seek(frame.min(len));
                        }
                    }

                    // Track label in top-left corner
                    painter.text(
                        egui::pos2(rect.left() + 6.0, rect.top() + 4.0),
                        egui::Align2::LEFT_TOP,
                        &overview.name,
                        egui::FontId::proportional(11.0),
                        egui::Color32::from_rgba_premultiplied(
                            overview.color.r(),
                            overview.color.g(),
                            overview.color.b(),
                            160,
                        ),
                    );

                    // Thin separator line between tracks
                    if track_idx + 1 < self.waveform_overviews.len() {
                        painter.line_segment(
                            [
                                egui::pos2(rect.left(), rect.bottom()),
                                egui::pos2(rect.right(), rect.bottom()),
                            ],
                            egui::Stroke::new(
                                0.5,
                                egui::Color32::from_rgb(50, 55, 60),
                            ),
                        );
                    }
                }
            }

            // ─── Horizontal scrollbar ─────────────────────────────
            // Compute scrollbar proportions
            let max_total_frames = self
                .waveform_overviews
                .iter()
                .map(|o| o.total_frames)
                .max()
                .unwrap_or(0) as f64;

            if max_total_frames > 0.0 {
                let view_frames = available.x as f64 * self.frames_per_pixel;
                let total_scrollable = max_total_frames + view_frames * 0.1;
                let frac_start = (self.scroll_offset_frames / total_scrollable) as f32;
                let frac_width = (view_frames / total_scrollable) as f32;

                let scrollbar_height = 10.0;
                let (sb_response, sb_painter) = ui.allocate_painter(
                    egui::vec2(available.x, scrollbar_height),
                    egui::Sense::click_and_drag(),
                );
                let sb_rect = sb_response.rect;

                // Background
                sb_painter.rect_filled(sb_rect, 2.0, egui::Color32::from_rgb(30, 32, 36));

                // Thumb
                let thumb_left = sb_rect.left() + frac_start * sb_rect.width();
                let thumb_width = (frac_width * sb_rect.width()).max(20.0);
                let thumb_rect = egui::Rect::from_min_size(
                    egui::pos2(thumb_left, sb_rect.top()),
                    egui::vec2(thumb_width, scrollbar_height),
                );
                let thumb_color = if sb_response.hovered() || sb_response.dragged() {
                    egui::Color32::from_rgb(90, 95, 105)
                } else {
                    egui::Color32::from_rgb(60, 65, 72)
                };
                sb_painter.rect_filled(thumb_rect, 2.0, thumb_color);

                // Handle scrollbar drag
                if sb_response.dragged() {
                    let delta_x = sb_response.drag_delta().x;
                    let delta_frac = delta_x / sb_rect.width();
                    self.scroll_offset_frames += delta_frac as f64 * total_scrollable;
                    self.scroll_offset_frames = self.scroll_offset_frames
                        .max(0.0)
                        .min(max_total_frames);
                    self.auto_follow = false;
                }

                // Click on scrollbar to jump
                if sb_response.clicked() {
                    if let Some(pos) = sb_response.interact_pointer_pos() {
                        let frac = ((pos.x - sb_rect.left()) / sb_rect.width())
                            .clamp(0.0, 1.0) as f64;
                        self.scroll_offset_frames =
                            frac * total_scrollable - view_frames * 0.5;
                        self.scroll_offset_frames = self.scroll_offset_frames
                            .max(0.0)
                            .min(max_total_frames);
                        self.auto_follow = false;
                    }
                }
            }

            // Extend the playhead line from the ruler across the whole arrangement
            let _ = ruler_rect; // used above for playhead triangle
        });

        // Request continuous repaint for smooth animation
        ctx.request_repaint();
    }
}

// ─── Piano Roll ──────────────────────────────────────────────────────────────

/// Returns the note name for a MIDI note number (e.g. 60 -> "C4").
fn midi_note_name(midi: u8) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (midi as i32 / 12) - 1;
    let idx = (midi % 12) as usize;
    format!("{}{}", names[idx], octave)
}

/// Returns true if the MIDI note is a black key.
fn is_black_key(midi: u8) -> bool {
    matches!(midi % 12, 1 | 3 | 6 | 8 | 10)
}

/// Get the color for an accuracy bucket (played notes).
fn accuracy_color(bucket: AccuracyBucket) -> egui::Color32 {
    match bucket {
        AccuracyBucket::Perfect => egui::Color32::from_rgb(60, 200, 80),
        AccuracyBucket::Good => egui::Color32::from_rgb(220, 200, 50),
        AccuracyBucket::Acceptable => egui::Color32::from_rgb(220, 140, 40),
        AccuracyBucket::Off => egui::Color32::from_rgb(220, 60, 60),
    }
}

/// Draw the piano roll view.
#[allow(clippy::too_many_arguments)]
fn draw_piano_roll(
    ui: &mut egui::Ui,
    reference_notes: &[NoteEvent],
    played_notes: &[PlayedNote],
    scroll_offset_frames: f64,
    frames_per_pixel: f64,
    sample_rate: u32,
    position_frame: u64,
    scroll_note: f32,
) {
    let available = ui.available_size();
    if available.x < 10.0 || available.y < 10.0 {
        return;
    }

    let sr = sample_rate as f64;

    // Allocate painter for the full area
    let (response, painter) =
        ui.allocate_painter(available, egui::Sense::click_and_drag());
    let full_rect = response.rect;

    // Piano keyboard area (left strip)
    let key_rect = egui::Rect::from_min_size(
        full_rect.min,
        egui::vec2(PIANO_KEY_WIDTH, full_rect.height()),
    );

    // Note grid area (right of keyboard)
    let grid_rect = egui::Rect::from_min_max(
        egui::pos2(full_rect.left() + PIANO_KEY_WIDTH, full_rect.top()),
        full_rect.max,
    );

    // Background
    painter.rect_filled(full_rect, 0.0, egui::Color32::from_rgb(18, 20, 24));

    // Compute visible note range
    let visible_rows = (grid_rect.height() / SEMITONE_ROW_HEIGHT).ceil() as i32;
    let bottom_note = scroll_note.floor() as i32;
    let top_note = (bottom_note + visible_rows).min(127);

    // Helper: convert MIDI note to Y position (higher notes = higher on screen = lower Y)
    let note_to_y = |midi: i32| -> f32 {
        let rows_from_bottom = midi as f32 - scroll_note;
        grid_rect.bottom() - rows_from_bottom * SEMITONE_ROW_HEIGHT
    };

    // Helper: convert time (seconds) to x position in the grid
    let time_to_x = |seconds: f64| -> f32 {
        let frame = seconds * sr;
        let x = (frame - scroll_offset_frames) / frames_per_pixel;
        grid_rect.left() + x as f32
    };

    // Draw grid rows (semitone lanes)
    for midi in bottom_note.max(0)..=top_note.min(127) {
        let y_top = note_to_y(midi + 1);
        let y_bot = note_to_y(midi);
        if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
            continue;
        }
        let y_top = y_top.max(grid_rect.top());
        let y_bot = y_bot.min(grid_rect.bottom());

        let row_rect = egui::Rect::from_min_max(
            egui::pos2(grid_rect.left(), y_top),
            egui::pos2(grid_rect.right(), y_bot),
        );

        // Alternating row colors (darker for black keys)
        let bg = if is_black_key(midi as u8) {
            egui::Color32::from_rgb(14, 16, 20)
        } else {
            egui::Color32::from_rgb(22, 24, 28)
        };
        painter.rect_filled(row_rect, 0.0, bg);

        // Grid line at each semitone boundary
        painter.line_segment(
            [
                egui::pos2(grid_rect.left(), y_bot),
                egui::pos2(grid_rect.right(), y_bot),
            ],
            egui::Stroke::new(
                if midi % 12 == 0 { 0.8 } else { 0.3 },
                egui::Color32::from_rgb(40, 44, 50),
            ),
        );
    }

    // Draw reference notes (semi-transparent colored rectangles)
    let ref_color = egui::Color32::from_rgba_premultiplied(60, 120, 200, 60);
    let ref_border = egui::Color32::from_rgba_premultiplied(80, 150, 230, 120);
    for note in reference_notes {
        let x0 = time_to_x(note.onset_seconds);
        let x1 = time_to_x(note.end_seconds());
        let y_top = note_to_y(note.midi_note as i32 + 1);
        let y_bot = note_to_y(note.midi_note as i32);

        // Clip to visible grid
        if x1 < grid_rect.left() || x0 > grid_rect.right() {
            continue;
        }
        if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
            continue;
        }

        let note_rect = egui::Rect::from_min_max(
            egui::pos2(x0.max(grid_rect.left()), y_top.max(grid_rect.top())),
            egui::pos2(x1.min(grid_rect.right()), y_bot.min(grid_rect.bottom())),
        );
        painter.rect_filled(note_rect, 2.0, ref_color);
        painter.rect_stroke(note_rect, 2.0, egui::Stroke::new(1.0, ref_border), egui::StrokeKind::Inside);
    }

    // Check which reference notes were missed (no overlapping played note)
    let missed_color = egui::Color32::from_rgba_premultiplied(120, 120, 120, 100);
    for note in reference_notes {
        let has_match = played_notes.iter().any(|p| {
            p.midi_note == note.midi_note
                && p.start_seconds < note.end_seconds()
                && p.end_seconds > note.onset_seconds
        });
        if !has_match {
            let x0 = time_to_x(note.onset_seconds);
            let x1 = time_to_x(note.end_seconds());
            let y_top = note_to_y(note.midi_note as i32 + 1);
            let y_bot = note_to_y(note.midi_note as i32);

            if x1 < grid_rect.left() || x0 > grid_rect.right() {
                continue;
            }
            if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
                continue;
            }

            let note_rect = egui::Rect::from_min_max(
                egui::pos2(x0.max(grid_rect.left()), y_top.max(grid_rect.top())),
                egui::pos2(x1.min(grid_rect.right()), y_bot.min(grid_rect.bottom())),
            );
            painter.rect_stroke(
                note_rect,
                2.0,
                egui::Stroke::new(1.5, missed_color),
                egui::StrokeKind::Inside,
            );
        }
    }

    // Draw played notes (solid rectangles, color-coded by accuracy)
    for pn in played_notes {
        let x0 = time_to_x(pn.start_seconds);
        let x1 = time_to_x(pn.end_seconds);
        let y_top = note_to_y(pn.midi_note as i32 + 1);
        let y_bot = note_to_y(pn.midi_note as i32);

        // Clip
        if x1 < grid_rect.left() || x0 > grid_rect.right() {
            continue;
        }
        if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
            continue;
        }

        // Ensure minimum width of 2px so short notes are visible
        let draw_x1 = x1.max(x0 + 2.0);

        let note_rect = egui::Rect::from_min_max(
            egui::pos2(x0.max(grid_rect.left()), y_top.max(grid_rect.top()) + 1.0),
            egui::pos2(
                draw_x1.min(grid_rect.right()),
                y_bot.min(grid_rect.bottom()) - 1.0,
            ),
        );
        let color = accuracy_color(pn.accuracy);
        painter.rect_filled(note_rect, 1.0, color);
    }

    // Draw piano keyboard strip
    painter.rect_filled(key_rect, 0.0, egui::Color32::from_rgb(25, 28, 32));
    for midi in bottom_note.max(0)..=top_note.min(127) {
        let y_top = note_to_y(midi + 1);
        let y_bot = note_to_y(midi);
        if y_top > key_rect.bottom() || y_bot < key_rect.top() {
            continue;
        }
        let y_top = y_top.max(key_rect.top());
        let y_bot = y_bot.min(key_rect.bottom());

        let is_black = is_black_key(midi as u8);
        let key_bg = if is_black {
            egui::Color32::from_rgb(30, 32, 38)
        } else {
            egui::Color32::from_rgb(55, 58, 65)
        };
        let kr = egui::Rect::from_min_max(
            egui::pos2(key_rect.left(), y_top),
            egui::pos2(key_rect.right() - 1.0, y_bot),
        );
        painter.rect_filled(kr, 0.0, key_bg);

        // Note label for C notes and every octave
        if midi % 12 == 0 || (y_bot - y_top) > 10.0 {
            let name = midi_note_name(midi as u8);
            let text_color = if midi % 12 == 0 {
                egui::Color32::from_rgb(180, 185, 190)
            } else {
                egui::Color32::from_rgb(110, 115, 120)
            };
            let font = egui::FontId::monospace(9.0);
            painter.text(
                egui::pos2(key_rect.left() + 3.0, (y_top + y_bot) / 2.0),
                egui::Align2::LEFT_CENTER,
                name,
                font,
                text_color,
            );
        }

        // Separator line
        painter.line_segment(
            [
                egui::pos2(key_rect.left(), y_bot),
                egui::pos2(key_rect.right(), y_bot),
            ],
            egui::Stroke::new(0.3, egui::Color32::from_rgb(40, 44, 50)),
        );
    }

    // Right border of keyboard
    painter.line_segment(
        [
            egui::pos2(key_rect.right(), key_rect.top()),
            egui::pos2(key_rect.right(), key_rect.bottom()),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 55, 62)),
    );

    // Playhead line
    let playhead_x = time_to_x(position_frame as f64 / sr);
    if playhead_x >= grid_rect.left() && playhead_x <= grid_rect.right() {
        painter.line_segment(
            [
                egui::pos2(playhead_x, grid_rect.top()),
                egui::pos2(playhead_x, grid_rect.bottom()),
            ],
            egui::Stroke::new(1.5, egui::Color32::from_rgb(220, 50, 50)),
        );
    }
}

// ─── Drawing helpers ─────────────────────────────────────────────────────────

/// Draw time markings on the ruler.
fn draw_time_ruler(
    painter: &egui::Painter,
    rect: egui::Rect,
    scroll_offset_frames: f64,
    frames_per_pixel: f64,
    sample_rate: u32,
) {
    let sr = sample_rate as f64;
    let view_duration_secs = rect.width() as f64 * frames_per_pixel / sr;

    // Choose a nice tick interval based on zoom level
    let target_pixels_per_tick = 80.0;
    let secs_per_tick_target = target_pixels_per_tick * frames_per_pixel / sr;

    // Snap to nice intervals: 0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60...
    let nice_intervals = [
        0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
    ];
    let secs_per_tick = nice_intervals
        .iter()
        .find(|&&i| i >= secs_per_tick_target)
        .copied()
        .unwrap_or(300.0);

    let start_secs = scroll_offset_frames / sr;
    let end_secs = start_secs + view_duration_secs;

    // Start at the first tick >= start_secs
    let first_tick = (start_secs / secs_per_tick).ceil() * secs_per_tick;

    let text_color = egui::Color32::from_rgb(160, 170, 180);
    let tick_color = egui::Color32::from_rgb(80, 85, 90);
    let minor_tick_color = egui::Color32::from_rgb(50, 55, 60);

    let mut t = first_tick;
    while t <= end_secs + secs_per_tick {
        let frame = t * sr;
        let x = rect.left() + ((frame - scroll_offset_frames) / frames_per_pixel) as f32;

        if x < rect.left() - 10.0 || x > rect.right() + 10.0 {
            t += secs_per_tick;
            continue;
        }

        // Major tick
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - 8.0),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(1.0, tick_color),
        );

        // Time label
        let label = if t >= 60.0 {
            let mins = (t / 60.0) as u32;
            let secs = t % 60.0;
            if secs_per_tick >= 1.0 {
                format!("{}:{:02.0}", mins, secs)
            } else {
                format!("{}:{:04.1}", mins, secs)
            }
        } else if secs_per_tick >= 1.0 {
            format!("{:.0}s", t)
        } else if secs_per_tick >= 0.1 {
            format!("{:.1}s", t)
        } else {
            format!("{:.2}s", t)
        };

        painter.text(
            egui::pos2(x + 3.0, rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::monospace(10.0),
            text_color,
        );

        // Minor ticks (subdivisions)
        let subdivs = 4;
        let minor_step = secs_per_tick / subdivs as f64;
        for s in 1..subdivs {
            let mt = t + s as f64 * minor_step;
            let mx_frame = mt * sr;
            let mx =
                rect.left() + ((mx_frame - scroll_offset_frames) / frames_per_pixel) as f32;
            if mx >= rect.left() && mx <= rect.right() {
                painter.line_segment(
                    [
                        egui::pos2(mx, rect.bottom() - 4.0),
                        egui::pos2(mx, rect.bottom()),
                    ],
                    egui::Stroke::new(0.5, minor_tick_color),
                );
            }
        }

        t += secs_per_tick;
    }

    // Bottom border of ruler
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 55, 60)),
    );
}

/// Draw a stereo peak/RMS meter in the sidebar.
fn draw_stereo_meter(ui: &mut egui::Ui, peak_l: f32, peak_r: f32, rms_l: f32, rms_r: f32) {
    let available_width = ui.available_width();
    let meter_height = 100.0;
    let bar_width = (available_width - 12.0) / 2.0;

    let (_, painter) =
        ui.allocate_painter(egui::vec2(available_width, meter_height), egui::Sense::hover());
    let rect = painter.clip_rect();

    // Left meter
    let l_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 2.0, rect.top()),
        egui::vec2(bar_width, meter_height),
    );
    draw_single_meter(&painter, l_rect, peak_l, rms_l);

    // Right meter
    let r_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + bar_width + 10.0, rect.top()),
        egui::vec2(bar_width, meter_height),
    );
    draw_single_meter(&painter, r_rect, peak_r, rms_r);

    // Labels
    painter.text(
        egui::pos2(l_rect.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        "L",
        egui::FontId::proportional(9.0),
        egui::Color32::from_rgb(120, 120, 120),
    );
    painter.text(
        egui::pos2(r_rect.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        "R",
        egui::FontId::proportional(9.0),
        egui::Color32::from_rgb(120, 120, 120),
    );
}

fn draw_single_meter(painter: &egui::Painter, rect: egui::Rect, peak: f32, rms: f32) {
    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(20, 22, 26));

    // RMS bar (bottom-up)
    let rms_height = rms.min(1.0) * rect.height();
    let rms_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.bottom() - rms_height),
        egui::vec2(rect.width(), rms_height),
    );

    // Gradient-like coloring: green -> yellow -> red
    let rms_db = to_db(rms);
    let rms_color = if rms_db > -6.0 {
        egui::Color32::from_rgb(220, 60, 60)
    } else if rms_db > -18.0 {
        egui::Color32::from_rgb(60, 180, 80)
    } else {
        egui::Color32::from_rgb(40, 140, 60)
    };
    painter.rect_filled(rms_rect, 2.0, rms_color);

    // Peak indicator line
    let peak_y = rect.bottom() - peak.min(1.0) * rect.height();
    let peak_db = to_db(peak);
    let peak_color = if peak_db > -3.0 {
        egui::Color32::from_rgb(255, 60, 60)
    } else {
        egui::Color32::from_rgb(120, 255, 140)
    };
    painter.line_segment(
        [
            egui::pos2(rect.left(), peak_y),
            egui::pos2(rect.right(), peak_y),
        ],
        egui::Stroke::new(2.0, peak_color),
    );

    // dB scale markers
    for &db in &[-6.0f32, -12.0, -24.0, -36.0] {
        let linear = 10.0f32.powf(db / 20.0);
        let y = rect.bottom() - linear * rect.height();
        if y > rect.top() && y < rect.bottom() {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), y),
                    egui::pos2(rect.left() + 3.0, y),
                ],
                egui::Stroke::new(0.5, egui::Color32::from_rgb(80, 80, 80)),
            );
        }
    }
}

fn to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -f32::INFINITY
    } else {
        20.0 * linear.log10()
    }
}
