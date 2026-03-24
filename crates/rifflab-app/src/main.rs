use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui;
use std::sync::{Arc, Mutex};

/// Waveform ring buffer: audio thread writes, UI thread reads.
const WAVEFORM_SIZE: usize = 4096;

/// Available buffer sizes.
const BUFFER_OPTIONS: [u32; 4] = [64, 128, 256, 512];

/// Audio device info for UI display.
#[derive(Clone, Debug)]
struct DeviceInfo {
    name: String,
    index: usize,
    /// For PipeWire sources: the pw/pulse source name (used for selection).
    pw_name: Option<String>,
}

/// A PipeWire source discovered via `pactl`.
#[derive(Clone, Debug)]
struct PwSource {
    id: String,
    name: String,
    description: String,
    channels: u16,
}

/// Query PipeWire/PulseAudio sources via `pactl list sources short`.
fn enumerate_pw_sources() -> Vec<PwSource> {
    let output = std::process::Command::new("pactl")
        .args(["list", "sources"])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Vec::new(),
    };

    let mut sources = Vec::new();
    let mut current_id = String::new();
    let mut current_name = String::new();
    let mut current_desc = String::new();
    let mut current_channels: u16 = 0;

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("Source #") {
            // Save previous source if valid
            if !current_name.is_empty() && !current_name.contains(".monitor") {
                sources.push(PwSource {
                    id: current_id.clone(),
                    name: current_name.clone(),
                    description: current_desc.clone(),
                    channels: current_channels,
                });
            }
            current_id = line.trim_start_matches("Source #").to_string();
            current_name.clear();
            current_desc.clear();
            current_channels = 0;
        } else if line.starts_with("Name: ") {
            current_name = line.trim_start_matches("Name: ").to_string();
        } else if line.starts_with("Description: ") {
            current_desc = line.trim_start_matches("Description: ").to_string();
        } else if line.starts_with("Channel Map: ") {
            let map = line.trim_start_matches("Channel Map: ");
            current_channels = map.split(',').count() as u16;
        }
    }
    // Don't forget the last source
    if !current_name.is_empty() && !current_name.contains(".monitor") {
        sources.push(PwSource {
            id: current_id,
            name: current_name,
            description: current_desc,
            channels: current_channels,
        });
    }

    sources
}

/// Set the default PulseAudio/PipeWire source.
fn set_default_source(name: &str) {
    let _ = std::process::Command::new("pactl")
        .args(["set-default-source", name])
        .status();
}

/// Shared state between audio thread and UI.
struct SharedState {
    /// Circular waveform buffer for visualization.
    waveform: Vec<f32>,
    waveform_write_pos: usize,
    /// Peak level (0.0–1.0) for meter.
    peak: f32,
    /// RMS level.
    rms: f32,
    /// Sample rate from active stream.
    sample_rate: u32,
    /// Whether audio is active.
    active: bool,
}

impl SharedState {
    fn new() -> Self {
        Self {
            waveform: vec![0.0; WAVEFORM_SIZE],
            waveform_write_pos: 0,
            peak: 0.0,
            rms: 0.0,
            sample_rate: 0,
            active: false,
        }
    }
}

fn main() -> Result<()> {
    env_logger::init();
    log::info!("Starting RiffLab Audio Demo");

    // Use default host (ALSA on Linux — works through PipeWire's ALSA emulation)
    let host = cpal::default_host();
    log::info!("Available audio hosts: {:?}", cpal::available_hosts());

    let input_devices = enumerate_input_devices(&host);
    let output_devices = enumerate_output_devices(&host);
    let pw_sources = enumerate_pw_sources();

    log::info!("Host: {}", host.id().name());
    for d in &input_devices {
        log::info!("  Input: [{}] {}", d.index, d.name);
    }
    for d in &output_devices {
        log::info!("  Output: [{}] {}", d.index, d.name);
    }
    for s in &pw_sources {
        log::info!("  PW Source: {} — {} ({}ch)", s.id, s.description, s.channels);
    }

    // Shared state for audio ↔ UI communication
    let shared = Arc::new(Mutex::new(SharedState::new()));

    // Launch UI
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 600.0])
            .with_title("RiffLab — Audio Demo"),
        ..Default::default()
    };

    let app_shared = Arc::clone(&shared);
    eframe::run_native(
        "RiffLab",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(AudioDemoApp::new(
                host,
                input_devices,
                output_devices,
                pw_sources,
                app_shared,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("UI error: {e}"))?;

    Ok(())
}

fn enumerate_input_devices(host: &cpal::Host) -> Vec<DeviceInfo> {
    let mut devices: Vec<DeviceInfo> = host
        .input_devices()
        .map(|devs| {
            devs.enumerate()
                .filter_map(|(i, d)| {
                    d.name().ok().map(|name| DeviceInfo { name, index: i, pw_name: None })
                })
                .collect()
        })
        .unwrap_or_default();

    // If no enumerated devices (JACK mode), add the default device
    if devices.is_empty() {
        if let Some(d) = host.default_input_device() {
            let name = d.name().unwrap_or_else(|_| "Default Input".into());
            devices.push(DeviceInfo { name, index: 0, pw_name: None });
        }
    }
    devices
}

fn enumerate_output_devices(host: &cpal::Host) -> Vec<DeviceInfo> {
    let mut devices: Vec<DeviceInfo> = host
        .output_devices()
        .map(|devs| {
            devs.enumerate()
                .filter_map(|(i, d)| {
                    d.name().ok().map(|name| DeviceInfo { name, index: i, pw_name: None })
                })
                .collect()
        })
        .unwrap_or_default();

    if devices.is_empty() {
        if let Some(d) = host.default_output_device() {
            let name = d.name().unwrap_or_else(|_| "Default Output".into());
            devices.push(DeviceInfo { name, index: 0, pw_name: None });
        }
    }
    devices
}

/// Get the Nth input device, falling back to default.
fn get_input_device(host: &cpal::Host, index: usize) -> Option<cpal::Device> {
    host.input_devices()
        .ok()
        .and_then(|mut devs| devs.nth(index))
        .or_else(|| host.default_input_device())
}

/// Get the Nth output device, falling back to default.
fn get_output_device(host: &cpal::Host, index: usize) -> Option<cpal::Device> {
    host.output_devices()
        .ok()
        .and_then(|mut devs| devs.nth(index))
        .or_else(|| host.default_output_device())
}

// ─── Audio Demo App ───────────────────────────────────────────────────────

struct AudioDemoApp {
    host: cpal::Host,
    input_devices: Vec<DeviceInfo>,
    output_devices: Vec<DeviceInfo>,
    pw_sources: Vec<PwSource>,
    selected_input: usize,
    selected_output: usize,
    selected_pw_source: usize,
    selected_channel: usize,
    buffer_size: u32,
    input_channel_count: usize,
    shared: Arc<Mutex<SharedState>>,
    input_stream: Option<cpal::Stream>,
    output_stream: Option<cpal::Stream>,
    /// Ring buffer for passing audio from input callback to output callback.
    passthrough_tx: Option<rtrb::Producer<f32>>,
    passthrough_rx: Option<rtrb::Consumer<f32>>,
    error_msg: Option<String>,
}

impl AudioDemoApp {
    fn new(
        host: cpal::Host,
        input_devices: Vec<DeviceInfo>,
        output_devices: Vec<DeviceInfo>,
        pw_sources: Vec<PwSource>,
        shared: Arc<Mutex<SharedState>>,
    ) -> Self {
        Self {
            host,
            selected_input: 0,
            selected_output: 0,
            selected_pw_source: 0,
            selected_channel: 0,
            buffer_size: 256,
            input_channel_count: 2,
            input_devices,
            output_devices,
            pw_sources,
            shared,
            input_stream: None,
            output_stream: None,
            passthrough_tx: None,
            passthrough_rx: None,
            error_msg: None,
        }
    }

    fn start_audio(&mut self) {
        self.stop_audio();
        self.error_msg = None;

        // Set PipeWire default source to the selected one before opening cpal
        if let Some(pw_src) = self.pw_sources.get(self.selected_pw_source) {
            log::info!("Setting PipeWire source to: {} ({})", pw_src.description, pw_src.name);
            set_default_source(&pw_src.name);
            self.input_channel_count = pw_src.channels as usize;
            // Small delay for PipeWire to apply
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Get devices
        let input_device = match get_input_device(&self.host, self.selected_input) {
            Some(d) => d,
            None => {
                self.error_msg = Some("Input device not found".into());
                return;
            }
        };
        let output_device = match get_output_device(&self.host, self.selected_output) {
            Some(d) => d,
            None => {
                self.error_msg = Some("Output device not found".into());
                return;
            }
        };

        // Get default configs
        let input_config = match input_device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                self.error_msg = Some(format!("Input config error: {e}"));
                return;
            }
        };
        let output_config = match output_device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                self.error_msg = Some(format!("Output config error: {e}"));
                return;
            }
        };

        let sample_rate = input_config.sample_rate().0;
        let input_channels = input_config.channels() as usize;
        let output_channels = output_config.channels() as usize;
        self.input_channel_count = input_channels;

        // Clamp selected channel to available range
        let selected_ch = self.selected_channel.min(input_channels.saturating_sub(1));

        let desired_buffer = self.buffer_size;

        log::info!(
            "Starting audio: input={} ({}ch @ {}Hz, using ch {}), output={} ({}ch), buffer={}",
            input_device.name().unwrap_or_default(),
            input_channels,
            sample_rate,
            selected_ch + 1,
            output_device.name().unwrap_or_default(),
            output_channels,
            desired_buffer,
        );

        // Ring buffer for passthrough — small! Just a few buffers worth (not 1 second)
        let ring_size = (desired_buffer as usize) * 4;
        let (tx, rx) = rtrb::RingBuffer::new(ring_size);
        self.passthrough_tx = Some(tx);
        self.passthrough_rx = Some(rx);

        // --- Input stream ---
        let shared_input = Arc::clone(&self.shared);
        let mut passthrough_tx = {
            // Move the producer into the input callback
            let (new_tx, _) = rtrb::RingBuffer::new(1); // dummy
            std::mem::replace(&mut self.passthrough_tx, Some(new_tx)).unwrap()
        };

        let mut input_stream_config: cpal::StreamConfig = input_config.clone().into();
        input_stream_config.buffer_size = cpal::BufferSize::Fixed(desired_buffer);
        let input_stream = match input_config.sample_format() {
            cpal::SampleFormat::F32 => input_device.build_input_stream(
                &input_stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    process_input(data, input_channels, selected_ch, &shared_input, &mut passthrough_tx);
                },
                |err| log::error!("Input stream error: {err}"),
                None,
            ),
            cpal::SampleFormat::I16 => input_device.build_input_stream(
                &input_stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let floats: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                    process_input(&floats, input_channels, selected_ch, &shared_input, &mut passthrough_tx);
                },
                |err| log::error!("Input stream error: {err}"),
                None,
            ),
            fmt => {
                self.error_msg = Some(format!("Unsupported input format: {fmt:?}"));
                return;
            }
        };

        let input_stream = match input_stream {
            Ok(s) => s,
            Err(e) => {
                self.error_msg = Some(format!("Failed to build input stream: {e}"));
                return;
            }
        };

        // --- Output stream ---
        let mut passthrough_rx = self.passthrough_rx.take().unwrap();
        let mut output_stream_config: cpal::StreamConfig = output_config.clone().into();
        output_stream_config.buffer_size = cpal::BufferSize::Fixed(desired_buffer);
        let output_stream = match output_config.sample_format() {
            cpal::SampleFormat::F32 => output_device.build_output_stream(
                &output_stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    process_output(data, output_channels, &mut passthrough_rx);
                },
                |err| log::error!("Output stream error: {err}"),
                None,
            ),
            cpal::SampleFormat::I16 => output_device.build_output_stream(
                &output_stream_config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut floats = vec![0.0f32; data.len()];
                    process_output(&mut floats, output_channels, &mut passthrough_rx);
                    for (out, &f) in data.iter_mut().zip(floats.iter()) {
                        *out = (f * 32767.0) as i16;
                    }
                },
                |err| log::error!("Output stream error: {err}"),
                None,
            ),
            fmt => {
                self.error_msg = Some(format!("Unsupported output format: {fmt:?}"));
                return;
            }
        };

        let output_stream = match output_stream {
            Ok(s) => s,
            Err(e) => {
                self.error_msg = Some(format!("Failed to build output stream: {e}"));
                return;
            }
        };

        // Start both streams
        if let Err(e) = input_stream.play() {
            self.error_msg = Some(format!("Failed to start input: {e}"));
            return;
        }
        if let Err(e) = output_stream.play() {
            self.error_msg = Some(format!("Failed to start output: {e}"));
            return;
        }

        if let Ok(mut state) = self.shared.lock() {
            state.sample_rate = sample_rate;
            state.active = true;
        }

        self.input_stream = Some(input_stream);
        self.output_stream = Some(output_stream);

        log::info!("Audio streams started");
    }

    fn stop_audio(&mut self) {
        self.input_stream = None;
        self.output_stream = None;
        self.passthrough_tx = None;
        self.passthrough_rx = None;
        if let Ok(mut state) = self.shared.lock() {
            state.active = false;
            state.peak = 0.0;
            state.rms = 0.0;
        }
    }

    #[allow(dead_code)]
    fn is_active(&self) -> bool {
        self.shared.lock().map(|s| s.active).unwrap_or(false)
    }
}

/// Input callback: write mono samples to waveform buffer and passthrough ring buffer.
fn process_input(
    data: &[f32],
    channels: usize,
    selected_channel: usize,
    shared: &Arc<Mutex<SharedState>>,
    passthrough: &mut rtrb::Producer<f32>,
) {
    if let Ok(mut state) = shared.lock() {
        let mut peak: f32 = 0.0;
        let mut sum_sq: f64 = 0.0;
        let mut count = 0u32;
        let ch = selected_channel.min(channels.saturating_sub(1));

        for frame in data.chunks(channels.max(1)) {
            let sample = frame[ch];
            peak = peak.max(sample.abs());
            sum_sq += (sample as f64) * (sample as f64);
            count += 1;

            // Write to waveform display buffer
            let pos = state.waveform_write_pos;
            state.waveform[pos] = sample;
            state.waveform_write_pos = (pos + 1) % WAVEFORM_SIZE;

            // Write to passthrough ring buffer (non-blocking, drop if full)
            let _ = passthrough.push(sample);
        }

        // Smoothed peak/RMS
        if count > 0 {
            let new_peak = peak;
            let new_rms = (sum_sq / count as f64).sqrt() as f32;
            state.peak = state.peak * 0.8 + new_peak * 0.2;
            state.rms = state.rms * 0.8 + new_rms * 0.2;
        }
    }
}

/// Output callback: read from passthrough ring buffer, duplicate mono to all output channels.
fn process_output(
    data: &mut [f32],
    channels: usize,
    passthrough: &mut rtrb::Consumer<f32>,
) {
    let channels = channels.max(1);
    for frame in data.chunks_mut(channels) {
        let sample = passthrough.pop().unwrap_or(0.0);
        for ch in frame.iter_mut() {
            *ch = sample;
        }
    }
}

impl eframe::App for AudioDemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Grab state snapshot
        let (waveform, peak, rms, sample_rate, active) = {
            let state = self.shared.lock().unwrap();
            (
                state.waveform.clone(),
                state.peak,
                state.rms,
                state.sample_rate,
                state.active,
            )
        };

        // ─── Toolbar ──────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("RiffLab");
                ui.separator();

                // PipeWire source selector (input)
                ui.label("Input:");
                let pw_label = self
                    .pw_sources
                    .get(self.selected_pw_source)
                    .map(|s| s.description.as_str())
                    .unwrap_or("None");
                egui::ComboBox::from_id_salt("pw_source")
                    .selected_text(truncate(pw_label, 35))
                    .show_ui(ui, |ui| {
                        for (i, src) in self.pw_sources.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.selected_pw_source,
                                i,
                                format!("{} ({}ch)", &src.description, src.channels),
                            );
                        }
                    });

                ui.separator();

                // Output device selector
                ui.label("Output:");
                let output_label = self
                    .output_devices
                    .get(self.selected_output)
                    .map(|d| d.name.as_str())
                    .unwrap_or("None");
                egui::ComboBox::from_id_salt("output_device")
                    .selected_text(truncate(output_label, 30))
                    .show_ui(ui, |ui| {
                        for dev in &self.output_devices {
                            ui.selectable_value(
                                &mut self.selected_output,
                                dev.index,
                                &dev.name,
                            );
                        }
                    });

                ui.separator();

                // Buffer size selector
                ui.label("Buffer:");
                let buf_label = format!("{} ({:.1}ms)", self.buffer_size, self.buffer_size as f32 / 48.0);
                egui::ComboBox::from_id_salt("buffer_size")
                    .selected_text(&buf_label)
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for &size in &BUFFER_OPTIONS {
                            let label = format!("{} ({:.1}ms)", size, size as f32 / 48.0);
                            ui.selectable_value(&mut self.buffer_size, size, label);
                        }
                    });

                ui.separator();

                // Start/Stop button
                if active {
                    if ui.button("Stop").clicked() {
                        self.stop_audio();
                    }
                } else if ui.button("Start").clicked() {
                    self.start_audio();
                }
            });
        });

        // ─── Status bar ───────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if active {
                    let latency_ms = if sample_rate > 0 {
                        2.0 * self.buffer_size as f32 / sample_rate as f32 * 1000.0
                    } else {
                        0.0
                    };
                    ui.label(format!("{}Hz | {:.1}ms RTL", sample_rate, latency_ms));
                    ui.separator();
                    ui.label(format!("Peak: {:.1} dB", to_db(peak)));
                    ui.separator();
                    ui.label(format!("RMS: {:.1} dB", to_db(rms)));
                } else {
                    ui.label("Audio stopped");
                }
                if let Some(ref err) = self.error_msg {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, err);
                }
            });
        });

        // ─── Main canvas: waveform ────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            if !active {
                ui.centered_and_justified(|ui| {
                    ui.label("Select input/output devices and click Start");
                });
                return;
            }

            ui.heading("Live Waveform");
            ui.add_space(8.0);

            // Waveform plot
            let available = ui.available_size();
            let (response, painter) =
                ui.allocate_painter(egui::vec2(available.x, available.y - 20.0), egui::Sense::hover());
            let rect = response.rect;

            // Background
            painter.rect_filled(rect, 4.0, egui::Color32::from_gray(20));

            // Center line
            let center_y = rect.center().y;
            painter.line_segment(
                [
                    egui::pos2(rect.left(), center_y),
                    egui::pos2(rect.right(), center_y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
            );

            // Draw waveform
            let width = rect.width();
            let height = rect.height();
            let samples_to_show = WAVEFORM_SIZE.min(width as usize);
            let step = WAVEFORM_SIZE as f32 / samples_to_show as f32;

            let points: Vec<egui::Pos2> = (0..samples_to_show)
                .map(|i| {
                    let idx = ((i as f32 * step) as usize) % WAVEFORM_SIZE;
                    let sample = waveform[idx];
                    let x = rect.left() + (i as f32 / samples_to_show as f32) * width;
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

            // Peak meter bar at the right edge
            let meter_width = 12.0;
            let meter_rect = egui::Rect::from_min_size(
                egui::pos2(rect.right() - meter_width - 4.0, rect.top() + 4.0),
                egui::vec2(meter_width, height - 8.0),
            );
            painter.rect_filled(meter_rect, 2.0, egui::Color32::from_gray(30));

            // RMS fill
            let rms_height = (rms.min(1.0) * (meter_rect.height())) as f32;
            let rms_rect = egui::Rect::from_min_size(
                egui::pos2(meter_rect.left(), meter_rect.bottom() - rms_height),
                egui::vec2(meter_width, rms_height),
            );
            painter.rect_filled(rms_rect, 2.0, egui::Color32::from_rgb(0, 150, 80));

            // Peak line
            let peak_y = meter_rect.bottom() - peak.min(1.0) * meter_rect.height();
            painter.line_segment(
                [
                    egui::pos2(meter_rect.left(), peak_y),
                    egui::pos2(meter_rect.right(), peak_y),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 255, 120)),
            );
        });

        // Request continuous repaint for real-time display
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
