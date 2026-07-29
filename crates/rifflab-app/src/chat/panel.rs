//! The right-side chat panel UI.

use super::tools;
use super::types::{ChatMessage, Role};
use super::worker::{self, RequestInput, WorkerEvent};
use crate::preset_graph::PresetGraph;
use eframe::egui;
use rifflab_core::audio::ParamKind;
use rifflab_fx::registry::EffectRegistry;

/// Outward-facing actions the panel asks main.rs to take.
#[derive(Debug)]
pub enum ChatAction {
    WidthChanged(f32),
    VisibilityChanged(bool),
    HistoryDirty,
    /// Tool calls produced a new preset graph that should replace `preset_nav`.
    /// The previous graph has already been pushed onto the undo stack.
    PresetGraphReplaced(Box<PresetGraph>),
    /// User clicked Undo; apply this snapshot to `preset_nav`.
    UndoRequested(Box<PresetGraph>),
}

pub struct ChatPanel {
    pub visible: bool,
    pub width: f32,
    pub messages: Vec<ChatMessage>,
    /// Snapshots of preset_nav taken just before each AI-applied mutation.
    /// Bounded to avoid memory growth.
    undo_stack: Vec<PresetGraph>,
    input: String,
    pending: Option<std::sync::mpsc::Receiver<WorkerEvent>>,
    /// Whether the effects catalog viewer is expanded.
    catalog_open: bool,
    last_reported_width: f32,
}

const MAX_UNDO_DEPTH: usize = 20;

impl ChatPanel {
    pub fn new(visible: bool, width: f32, messages: Vec<ChatMessage>) -> Self {
        Self {
            visible,
            width,
            messages,
            undo_stack: Vec::new(),
            input: String::new(),
            pending: None,
            catalog_open: false,
            last_reported_width: width,
        }
    }

    /// Drain background worker events. `preset_nav_snapshot` is the *current*
    /// graph captured by the UI thread — used to push onto the undo stack
    /// the first time a mutation comes back this turn.
    ///
    /// Returns the action list for main.rs to apply.
    pub fn poll(&mut self, current_preset_nav: &PresetGraph) -> Vec<ChatAction> {
        let mut actions = Vec::new();
        // Temporarily take the receiver so we can mutate `self` inside the loop.
        let Some(rx) = self.pending.take() else {
            return actions;
        };

        let mut done = false;
        loop {
            match rx.try_recv() {
                Ok(WorkerEvent::Activity(line)) => {
                    self.messages.push(ChatMessage::now(Role::System, line));
                    actions.push(ChatAction::HistoryDirty);
                }
                Ok(WorkerEvent::ToolCalled { name, summary, is_error }) => {
                    let prefix = if is_error { "⚠ " } else { "🔧 " };
                    self.messages.push(ChatMessage::now(
                        Role::System, format!("{prefix}{name}: {summary}")));
                    actions.push(ChatAction::HistoryDirty);
                }
                Ok(WorkerEvent::PresetMutation(new_nav)) => {
                    self.push_undo(current_preset_nav.clone());
                    actions.push(ChatAction::PresetGraphReplaced(new_nav));
                }
                Ok(WorkerEvent::Reply(text)) => {
                    self.messages.push(ChatMessage::now(Role::Assistant, text));
                    actions.push(ChatAction::HistoryDirty);
                }
                Ok(WorkerEvent::Error(err)) => {
                    self.messages.push(ChatMessage::now(Role::System, format!("⚠ {err}")));
                    actions.push(ChatAction::HistoryDirty);
                }
                Ok(WorkerEvent::Done) => {
                    done = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.messages.push(ChatMessage::now(Role::System,
                        "⚠ chat worker disconnected unexpectedly"));
                    actions.push(ChatAction::HistoryDirty);
                    done = true;
                    break;
                }
            }
        }
        // Restore the receiver unless the request is finished.
        if !done {
            self.pending = Some(rx);
        }
        actions
    }

    fn push_undo(&mut self, snapshot: PresetGraph) {
        self.undo_stack.push(snapshot);
        if self.undo_stack.len() > MAX_UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
    }

    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Render the panel. Caller passes the current preset_nav so tools can
    /// snapshot it, and the registry for the catalog viewer.
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        preset_nav: &PresetGraph,
        registry: &EffectRegistry,
        current_song: Option<&str>,
    ) -> Vec<ChatAction> {
        let mut actions = Vec::new();
        if !self.visible {
            return actions;
        }

        let response = egui::SidePanel::right("chat_panel")
            .resizable(true)
            .default_width(self.width)
            .min_width(280.0)
            .max_width(700.0)
            .show(ctx, |ui| {
                self.draw_inner(ui, preset_nav, registry, current_song, &mut actions);
            });

        let new_width = response.response.rect.width();
        if (new_width - self.last_reported_width).abs() > 0.5 {
            self.width = new_width;
            self.last_reported_width = new_width;
            actions.push(ChatAction::WidthChanged(new_width));
        }

        actions
    }

    fn draw_inner(
        &mut self,
        ui: &mut egui::Ui,
        preset_nav: &PresetGraph,
        registry: &EffectRegistry,
        current_song: Option<&str>,
        actions: &mut Vec<ChatAction>,
    ) {
        // Header bar.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🤖 Preset Assistant")
                .strong().size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✖").on_hover_text("Close panel").clicked() {
                    self.visible = false;
                    actions.push(ChatAction::VisibilityChanged(false));
                }
                let can_undo = !self.undo_stack.is_empty();
                let undo_btn = ui.add_enabled(can_undo, egui::Button::new("↶ Undo"))
                    .on_hover_text("Revert the last AI-applied change");
                if undo_btn.clicked() {
                    if let Some(snap) = self.undo_stack.pop() {
                        actions.push(ChatAction::UndoRequested(Box::new(snap)));
                        self.messages.push(ChatMessage::now(Role::System,
                            "↶ Reverted last AI change."));
                        actions.push(ChatAction::HistoryDirty);
                    }
                }
                if ui.small_button("Clear").on_hover_text("Reset conversation").clicked() {
                    self.messages.clear();
                    actions.push(ChatAction::HistoryDirty);
                }
            });
        });
        ui.separator();

        // Status row.
        if self.is_busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("Thinking…")
                    .color(egui::Color32::from_rgb(160, 200, 240)));
            });
            ui.separator();
        }

        // Collapsible effect catalog.
        egui::CollapsingHeader::new("📚 Effects catalog")
            .default_open(self.catalog_open)
            .show(ui, |ui| {
                self.catalog_open = true;
                draw_effect_catalog(ui, registry);
            });
        ui.separator();

        let available_height = ui.available_height();
        let input_block_height = 110.0;
        let history_height = (available_height - input_block_height).max(80.0);

        // Scrollback.
        egui::ScrollArea::vertical()
            .max_height(history_height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if self.messages.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(
                            "Ask me to build a preset, or tweak one you already have.")
                            .color(egui::Color32::from_rgb(140, 150, 160)));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(
                            "Try: \"Make a preset for Blind by Korn, bass tone.\"")
                            .italics()
                            .color(egui::Color32::from_rgb(120, 130, 140))
                            .size(11.0));
                    });
                } else {
                    for msg in &self.messages {
                        draw_message(ui, msg);
                        ui.add_space(4.0);
                    }
                }
            });

        ui.separator();
        ui.add_space(4.0);

        // Input box.
        let busy = self.is_busy();
        let response = ui.add_enabled(
            !busy,
            egui::TextEdit::multiline(&mut self.input)
                .desired_width(f32::INFINITY)
                .desired_rows(3)
                .hint_text("Press Ctrl+Enter to send"),
        );

        let send_via_shortcut = response.has_focus()
            && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter));

        ui.horizontal(|ui| {
            let can_send = !busy && !self.input.trim().is_empty();
            let send_clicked = ui.add_enabled(can_send, egui::Button::new("Send")).clicked();
            ui.label(egui::RichText::new(if busy { "" } else { "Ctrl+Enter" })
                .color(egui::Color32::from_rgb(110, 120, 130))
                .size(10.0));

            if (send_clicked || send_via_shortcut) && can_send {
                self.dispatch_user_message(preset_nav, registry, current_song);
                actions.push(ChatAction::HistoryDirty);
            }
        });
    }

    fn dispatch_user_message(
        &mut self,
        preset_nav: &PresetGraph,
        registry: &EffectRegistry,
        current_song: Option<&str>,
    ) {
        let prompt = std::mem::take(&mut self.input).trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.messages.push(ChatMessage::now(Role::User, prompt.clone()));
        let req = RequestInput {
            transcript: self.messages.clone(),
            prompt,
            system_prompt: worker::build_system_prompt(registry, current_song),
            preset_nav: preset_nav.clone(),
            tools: tools::tool_set(),
        };
        self.pending = Some(worker::spawn(req));
    }
}

fn draw_message(ui: &mut egui::Ui, msg: &ChatMessage) {
    let (label, label_color, bubble_color, text_color) = match msg.role {
        Role::User => (
            "You",
            egui::Color32::from_rgb(120, 180, 240),
            egui::Color32::from_rgb(36, 48, 64),
            egui::Color32::from_rgb(230, 235, 240),
        ),
        Role::Assistant => (
            "Claude",
            egui::Color32::from_rgb(160, 220, 170),
            egui::Color32::from_rgb(34, 46, 38),
            egui::Color32::from_rgb(225, 235, 225),
        ),
        Role::System => (
            "System",
            egui::Color32::from_rgb(200, 170, 110),
            egui::Color32::from_rgb(52, 44, 30),
            egui::Color32::from_rgb(230, 220, 200),
        ),
    };

    ui.label(egui::RichText::new(label).color(label_color).strong().size(11.0));
    egui::Frame::group(ui.style())
        .fill(bubble_color)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(&msg.text).color(text_color));
        });
}

fn draw_effect_catalog(ui: &mut egui::Ui, registry: &EffectRegistry) {
    ui.label(egui::RichText::new(
        "Quick reference; the assistant sees the full catalog automatically.")
        .italics().size(10.0).color(egui::Color32::from_rgb(140, 145, 150)));
    ui.add_space(2.0);
    for (type_id, name, category) in registry.list_effects() {
        egui::CollapsingHeader::new(format!("{name} — {category}"))
            .id_salt(&type_id)
            .default_open(false)
            .show(ui, |ui| {
                ui.label(egui::RichText::new(&type_id)
                    .monospace().size(10.0).color(egui::Color32::from_rgb(160, 170, 180)));
                if let Some(effect) = registry.create_effect(&type_id) {
                    for d in effect.param_descriptors() {
                        let kind = match &d.kind {
                            ParamKind::Float => "float".to_string(),
                            ParamKind::Int => "int".to_string(),
                            ParamKind::Bool => "bool".to_string(),
                            ParamKind::Enum(v) => format!("enum [{}]", v.join(", ")),
                        };
                        let unit = if d.unit.is_empty() { String::new() } else { format!(" {}", d.unit) };
                        ui.label(egui::RichText::new(format!(
                            "  {}  {} ({kind}) — {}{unit}…{}{unit} (default {}{unit})",
                            d.id.0, d.name, d.min, d.max, d.default,
                        )).size(11.0));
                    }
                }
            });
    }
}
