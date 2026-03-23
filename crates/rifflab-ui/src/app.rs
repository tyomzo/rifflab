use eframe::egui;

/// Top-level RiffLab application state.
pub struct RiffLabApp {
    // TODO: hold references/handles to audio engine, effects, practice state
}

impl RiffLabApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {}
    }
}

impl eframe::App for RiffLabApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Toolbar
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("RiffLab");
                ui.separator();
                if ui.button("⏵ Play").clicked() {
                    // TODO
                }
                if ui.button("⏸ Pause").clicked() {
                    // TODO
                }
                if ui.button("⏹ Stop").clicked() {
                    // TODO
                }
            });
        });

        // Status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Latency: --ms");
                ui.separator();
                ui.label("CPU: --%");
                ui.separator();
                ui.label("Pitch: --");
                ui.separator();
                ui.label("Score: --");
            });
        });

        // Sidebar
        egui::SidePanel::left("sidebar").resizable(true).default_width(200.0).show(ctx, |ui| {
            ui.heading("Tracks");
            ui.label("No song loaded");
        });

        // Main canvas
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Arrangement View");
            ui.label("Import a song to get started.");
        });

        // Request repaint for real-time updates
        ctx.request_repaint();
    }
}

/// Launch the UI.
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("RiffLab"),
        ..Default::default()
    };
    eframe::run_native(
        "RiffLab",
        options,
        Box::new(|cc| Ok(Box::new(RiffLabApp::new(cc)))),
    )
}
