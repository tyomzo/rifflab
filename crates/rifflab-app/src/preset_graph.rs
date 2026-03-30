//! Preset navigation graph: nodes are effect presets, wires define next/prev paths.

use crate::node_editor::FxGraph;
use eframe::egui;
use serde::{Deserialize, Serialize};

const PRESET_NODE_WIDTH: f32 = 150.0;
const PRESET_NODE_HEIGHT: f32 = 80.0;
const PORT_RADIUS: f32 = 5.0;

/// A MIDI binding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MidiBinding {
    ControlChange { channel: u8, cc: u8 },
    NoteOn { channel: u8, note: u8 },
    ProgramChange { channel: u8, program: u8 },
}

impl MidiBinding {
    pub fn label(&self) -> String {
        match self {
            Self::ControlChange { cc, .. } => format!("CC#{}", cc),
            Self::NoteOn { note, .. } => format!("Note {}", note),
            Self::ProgramChange { program, .. } => format!("PC#{}", program),
        }
    }

    pub fn matches_event(&self, event: &crate::midi_input::MidiEvent) -> bool {
        match (self, event) {
            (Self::ControlChange { channel: ch, cc: c }, crate::midi_input::MidiEvent::ControlChange { channel, cc, value }) => {
                *value > 0 && ch == channel && c == cc
            }
            (Self::NoteOn { channel: ch, note: n }, crate::midi_input::MidiEvent::NoteOn { channel, note, .. }) => {
                ch == channel && n == note
            }
            (Self::ProgramChange { channel: ch, program: p }, crate::midi_input::MidiEvent::ProgramChange { channel, program }) => {
                ch == channel && p == program
            }
            _ => false,
        }
    }
}

/// A preset node in the navigation graph.
#[derive(Clone, Serialize, Deserialize)]
pub struct PresetNode {
    pub id: u64,
    pub pos: [f32; 2],
    pub name: String,
    pub pipeline: FxGraph,
    #[serde(default)]
    pub midi_binding: Option<MidiBinding>,
}

/// A wire connecting two preset nodes (defines navigation order).
#[derive(Clone, Serialize, Deserialize)]
pub struct PresetWire {
    pub from_id: u64,
    pub to_id: u64,
}

/// What we're currently learning a MIDI binding for.
#[derive(Debug, Clone, PartialEq)]
pub enum MidiLearnTarget {
    None,
    PresetNode(u64),
    GlobalNext,
    GlobalPrev,
}

/// The full preset navigation graph.
#[derive(Clone, Serialize, Deserialize)]
pub struct PresetGraph {
    pub nodes: Vec<PresetNode>,
    pub wires: Vec<PresetWire>,
    pub active_id: Option<u64>,
    pub next_id: u64,
    #[serde(default)]
    pub midi_next: Option<MidiBinding>,
    #[serde(default)]
    pub midi_prev: Option<MidiBinding>,
    #[serde(default)]
    pub pan: [f32; 2],
}

impl Default for PresetGraph {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            wires: Vec::new(),
            active_id: None,
            next_id: 1,
            midi_next: None,
            midi_prev: None,
            pan: [0.0, 0.0],
        }
    }
}

impl PresetGraph {
    pub fn add_node(&mut self, name: String, pipeline: FxGraph, pos: [f32; 2]) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(PresetNode { id, pos, name, pipeline, midi_binding: None });
        id
    }

    /// Add a node with auto-positioned placement.
    pub fn add_node_with_pipeline(&mut self, name: String, pipeline: FxGraph) -> u64 {
        let x = 50.0 + self.nodes.len() as f32 * 180.0;
        self.add_node(name, pipeline, [x, 50.0])
    }

    pub fn remove_node(&mut self, id: u64) {
        self.wires.retain(|w| w.from_id != id && w.to_id != id);
        self.nodes.retain(|n| n.id != id);
        if self.active_id == Some(id) { self.active_id = None; }
    }

    pub fn add_wire(&mut self, from: u64, to: u64) {
        if from != to && !self.wires.iter().any(|w| w.from_id == from && w.to_id == to) {
            self.wires.push(PresetWire { from_id: from, to_id: to });
        }
    }

    pub fn find_node(&self, id: u64) -> Option<&PresetNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn find_node_mut(&mut self, id: u64) -> Option<&mut PresetNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Find the next node from the active node (follow outgoing wire).
    pub fn next_from(&self, id: u64) -> Option<u64> {
        self.wires.iter().find(|w| w.from_id == id).map(|w| w.to_id)
    }

    /// Find the previous node from the active node (follow incoming wire).
    pub fn prev_from(&self, id: u64) -> Option<u64> {
        self.wires.iter().find(|w| w.to_id == id).map(|w| w.from_id)
    }

    /// Find a preset by MIDI binding match.
    pub fn find_by_midi(&self, event: &crate::midi_input::MidiEvent) -> Option<u64> {
        for node in &self.nodes {
            if let Some(ref binding) = node.midi_binding {
                if binding.matches_event(event) {
                    return Some(node.id);
                }
            }
        }
        None
    }
}

// ─── Actions returned from the UI ───────────────────────────────────────────

pub enum PresetGraphAction {
    None,
    ActivatePreset(u64),
    EditPreset(u64),
    Save,
    Load,
}

// ─── Drawing ────────────────────────────────────────────────────────────────

pub fn draw_preset_graph(
    ui: &mut egui::Ui,
    graph: &mut PresetGraph,
    learn_target: &mut MidiLearnTarget,
) -> PresetGraphAction {
    let mut action = PresetGraphAction::None;

    // Toolbar
    ui.horizontal(|ui| {
        if ui.small_button("Save Presets").clicked() { action = PresetGraphAction::Save; }
        if ui.small_button("Load Presets").clicked() { action = PresetGraphAction::Load; }
        ui.separator();

        // Next/Prev MIDI learn buttons
        let next_label = graph.midi_next.as_ref().map(|b| b.label()).unwrap_or_else(|| "---".into());
        let prev_label = graph.midi_prev.as_ref().map(|b| b.label()).unwrap_or_else(|| "---".into());

        let prev_learning = *learn_target == MidiLearnTarget::GlobalPrev;
        let prev_text = if prev_learning { "Prev: [press key...]" } else { &format!("Prev: {}", prev_label) };
        if ui.add(egui::Button::new(egui::RichText::new(prev_text).size(10.0)
            .color(if prev_learning { egui::Color32::YELLOW } else { egui::Color32::from_rgb(170, 175, 185) })
        )).clicked() {
            *learn_target = if prev_learning { MidiLearnTarget::None } else { MidiLearnTarget::GlobalPrev };
        }

        let next_learning = *learn_target == MidiLearnTarget::GlobalNext;
        let next_text = if next_learning { "Next: [press key...]" } else { &format!("Next: {}", next_label) };
        if ui.add(egui::Button::new(egui::RichText::new(next_text).size(10.0)
            .color(if next_learning { egui::Color32::YELLOW } else { egui::Color32::from_rgb(170, 175, 185) })
        )).clicked() {
            *learn_target = if next_learning { MidiLearnTarget::None } else { MidiLearnTarget::GlobalNext };
        }

        // Active preset indicator
        if let Some(active_id) = graph.active_id {
            if let Some(node) = graph.find_node(active_id) {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(80, 220, 120),
                    egui::RichText::new(format!("Active: {}", node.name)).strong().size(11.0));
            }
        }
    });

    // Canvas
    let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = response.rect;
    let pan = egui::vec2(graph.pan[0], graph.pan[1]);

    // Background
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(22, 24, 30));

    // Grid
    let grid = 30.0;
    let dot_color = egui::Color32::from_rgb(32, 35, 42);
    let mut gx = rect.left() + (pan.x % grid);
    while gx < rect.right() { let mut gy = rect.top() + (pan.y % grid);
        while gy < rect.bottom() { painter.circle_filled(egui::pos2(gx, gy), 1.0, dot_color); gy += grid; }
        gx += grid;
    }

    // Draw wires
    for wire in &graph.wires {
        let from = graph.nodes.iter().find(|n| n.id == wire.from_id);
        let to = graph.nodes.iter().find(|n| n.id == wire.to_id);
        if let (Some(f), Some(t)) = (from, to) {
            let p0 = egui::pos2(rect.left() + f.pos[0] + pan.x + PRESET_NODE_WIDTH, rect.top() + f.pos[1] + pan.y + PRESET_NODE_HEIGHT / 2.0);
            let p1 = egui::pos2(rect.left() + t.pos[0] + pan.x, rect.top() + t.pos[1] + pan.y + PRESET_NODE_HEIGHT / 2.0);
            let dx = (p1.x - p0.x).abs() * 0.4;
            let cp0 = egui::pos2(p0.x + dx, p0.y);
            let cp1 = egui::pos2(p1.x - dx, p1.y);
            painter.add(egui::Shape::CubicBezier(egui::epaint::CubicBezierShape::from_points_stroke(
                [p0, cp0, cp1, p1], false, egui::Color32::TRANSPARENT,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 140, 100)),
            )));
        }
    }

    // Draw nodes
    let mut drag_start: Option<(u64, egui::Vec2)> = None;
    let mut wire_drag_start: Option<u64> = None;
    let mut remove_node_id: Option<u64> = None;
    let mut load_pipeline_for: Option<u64> = None;
    #[allow(static_mut_refs)]
    static mut DRAGGING_NODE: Option<(u64, egui::Vec2)> = None;
    #[allow(static_mut_refs)]
    static mut DRAGGING_WIRE: Option<u64> = None;

    for node in &graph.nodes {
        let nx = rect.left() + node.pos[0] + pan.x;
        let ny = rect.top() + node.pos[1] + pan.y;
        let node_rect = egui::Rect::from_min_size(egui::pos2(nx, ny), egui::vec2(PRESET_NODE_WIDTH, PRESET_NODE_HEIGHT));

        if !rect.intersects(node_rect) { continue; }

        let is_active = graph.active_id == Some(node.id);
        let is_learning = *learn_target == MidiLearnTarget::PresetNode(node.id);

        let bg = if is_active { egui::Color32::from_rgb(35, 55, 40) }
                 else { egui::Color32::from_rgb(30, 33, 40) };
        let border = if is_learning { egui::Color32::YELLOW }
                     else if is_active { egui::Color32::from_rgb(80, 220, 120) }
                     else { egui::Color32::from_rgb(60, 65, 75) };

        painter.rect_filled(node_rect, 6.0, bg);
        painter.rect_stroke(node_rect, 6.0, egui::Stroke::new(if is_active { 2.0 } else { 1.0 }, border), egui::StrokeKind::Outside);

        // Name (painted, used as drag handle)
        painter.text(egui::pos2(nx + PRESET_NODE_WIDTH / 2.0, ny + 12.0), egui::Align2::CENTER_CENTER,
            &node.name, egui::FontId::proportional(11.0),
            if is_active { egui::Color32::WHITE } else { egui::Color32::from_rgb(200, 205, 215) });

        // Output port (right side)
        let out_pos = egui::pos2(nx + PRESET_NODE_WIDTH, ny + PRESET_NODE_HEIGHT / 2.0);
        painter.circle_filled(out_pos, PORT_RADIUS, egui::Color32::from_rgb(100, 180, 140));
        // Input port (left side)
        let in_pos = egui::pos2(nx, ny + PRESET_NODE_HEIGHT / 2.0);
        painter.circle_filled(in_pos, PORT_RADIUS, egui::Color32::from_rgb(80, 150, 220));

        // Buttons inside the node (real egui widgets)
        let buttons_rect = egui::Rect::from_min_size(
            egui::pos2(nx + 4.0, ny + 24.0),
            egui::vec2(PRESET_NODE_WIDTH - 8.0, PRESET_NODE_HEIGHT - 28.0),
        );
        if rect.intersects(buttons_rect) {
            let node_id = node.id;
            let node_name = node.name.clone();
            let binding_label = if is_learning { "[press key...]".to_string() }
                else { node.midi_binding.as_ref().map(|b| b.label()).unwrap_or_else(|| "Learn MIDI".into()) };

            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(buttons_rect), |ui| {
                ui.set_clip_rect(rect);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    // MIDI learn button
                    let learn_color = if is_learning { egui::Color32::YELLOW } else { egui::Color32::from_rgb(130, 140, 155) };
                    if ui.add(egui::Button::new(
                        egui::RichText::new(&binding_label).size(9.0).color(learn_color)
                    ).min_size(egui::vec2(60.0, 16.0))).clicked() {
                        if is_learning {
                            *learn_target = MidiLearnTarget::None;
                        } else {
                            *learn_target = MidiLearnTarget::PresetNode(node_id);
                            log::info!("MIDI learn: waiting for key on '{}'", node_name);
                        }
                    }
                    // Delete button
                    if ui.small_button(
                        egui::RichText::new("\u{2716}").size(9.0).color(egui::Color32::from_rgb(150, 60, 60))
                    ).clicked() {
                        remove_node_id = Some(node_id);
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    // Load pipeline from file
                    if ui.add(egui::Button::new(
                        egui::RichText::new("Load").size(9.0)
                    ).min_size(egui::vec2(40.0, 16.0))).clicked() {
                        load_pipeline_for = Some(node_id);
                    }
                    // Edit pipeline
                    if ui.add(egui::Button::new(
                        egui::RichText::new("Edit").size(9.0)
                    ).min_size(egui::vec2(40.0, 16.0))).clicked() {
                        action = PresetGraphAction::EditPreset(node_id);
                    }
                });
            });
        }

        // Drag and wire interaction (only from the title bar area, y < ny + 24)
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let header_rect = egui::Rect::from_min_size(egui::pos2(nx, ny), egui::vec2(PRESET_NODE_WIDTH, 24.0));
            if header_rect.contains(pointer) && ui.input(|i| i.pointer.primary_pressed()) {
                drag_start = Some((node.id, pointer - egui::pos2(nx, ny)));
            }
            if header_rect.contains(pointer) && ui.input(|i| i.pointer.primary_released()) {
                action = PresetGraphAction::ActivatePreset(node.id);
            }
            // Output port drag (wire creation)
            if out_pos.distance(pointer) < 8.0 && ui.input(|i| i.pointer.primary_pressed()) {
                wire_drag_start = Some(node.id);
            }
        }
    }

    // Node dragging (using statics for simplicity — single-threaded UI)
    #[allow(static_mut_refs)]
    unsafe {
        if let Some((id, offset)) = drag_start {
            DRAGGING_NODE = Some((id, offset));
        }
        if let Some((id, offset)) = DRAGGING_NODE {
            if ui.input(|i| i.pointer.primary_down()) {
                if let Some(pointer) = ui.ctx().pointer_latest_pos() {
                    if let Some(node) = graph.find_node_mut(id) {
                        node.pos[0] = pointer.x - rect.left() - pan.x - offset.x;
                        node.pos[1] = pointer.y - rect.top() - pan.y - offset.y;
                    }
                }
            } else {
                DRAGGING_NODE = None;
            }
        }

        // Wire dragging
        if let Some(id) = wire_drag_start { DRAGGING_WIRE = Some(id); }
        if let Some(from_id) = DRAGGING_WIRE {
            if let Some(pointer) = ui.ctx().pointer_latest_pos() {
                if let Some(from) = graph.find_node(from_id) {
                    let p0 = egui::pos2(rect.left() + from.pos[0] + pan.x + PRESET_NODE_WIDTH, rect.top() + from.pos[1] + pan.y + PRESET_NODE_HEIGHT / 2.0);
                    let dx = (pointer.x - p0.x).abs() * 0.4;
                    painter.add(egui::Shape::CubicBezier(egui::epaint::CubicBezierShape::from_points_stroke(
                        [p0, egui::pos2(p0.x + dx, p0.y), egui::pos2(pointer.x - dx, pointer.y), pointer],
                        false, egui::Color32::TRANSPARENT,
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(150, 220, 100)),
                    )));
                }
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                // Check drop on input port
                if let Some(pointer) = ui.ctx().pointer_latest_pos() {
                    for node in &graph.nodes {
                        let in_pos = egui::pos2(rect.left() + node.pos[0] + pan.x, rect.top() + node.pos[1] + pan.y + PRESET_NODE_HEIGHT / 2.0);
                        if in_pos.distance(pointer) < 10.0 && node.id != from_id {
                            graph.add_wire(from_id, node.id);
                            break;
                        }
                    }
                }
                DRAGGING_WIRE = None;
            }
        }

        // Canvas panning
        if response.dragged() && DRAGGING_NODE.is_none() && DRAGGING_WIRE.is_none() {
            let delta = response.drag_delta();
            graph.pan[0] += delta.x;
            graph.pan[1] += delta.y;
        }
    }

    // Handle deferred actions
    if let Some(id) = remove_node_id {
        graph.remove_node(id);
    }
    if let Some(id) = load_pipeline_for {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("RiffLab Graph", &["json"])
            .pick_file()
        {
            match crate::node_editor::load_graph(&path) {
                Ok(g) => {
                    if let Some(node) = graph.find_node_mut(id) {
                        node.pipeline = g;
                        node.name = path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Loaded")
                            .to_string();
                    }
                    action = PresetGraphAction::ActivatePreset(id);
                }
                Err(e) => log::error!("Load pipeline failed: {e}"),
            }
        }
    }

    // Right-click context menu (simplified — delete and edit moved to node buttons)
    response.context_menu(|ui| {
        let click_pos = ui.ctx().pointer_latest_pos()
            .map(|p| [p.x - rect.left() - pan.x, p.y - rect.top() - pan.y])
            .unwrap_or([100.0, 100.0]);

        if ui.button("Add Preset Node").clicked() {
            let pipeline = FxGraph::new_default();
            graph.add_node(format!("Preset {}", graph.next_id), pipeline, click_pos);
            ui.close_menu();
        }

        // Delete if hovering a node
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            for node in &graph.nodes {
                let nx = rect.left() + node.pos[0] + pan.x;
                let ny = rect.top() + node.pos[1] + pan.y;
                let nr = egui::Rect::from_min_size(egui::pos2(nx, ny), egui::vec2(PRESET_NODE_WIDTH, PRESET_NODE_HEIGHT));
                if nr.contains(pointer) {
                    ui.separator();
                    if ui.button(format!("Delete '{}'", node.name)).clicked() {
                        let _id = node.id;
                        // Deferred — can't modify while iterating
                        // Will handle outside
                        ui.close_menu();
                    }
                    if ui.button(format!("Edit Pipeline '{}'", node.name)).clicked() {
                        action = PresetGraphAction::EditPreset(node.id);
                        ui.close_menu();
                    }
                    break;
                }
            }
        }
    });

    action
}

// ─── Persistence ────────────────────────────────────────────────────────────

pub fn save_preset_graph(path: &std::path::Path, graph: &PresetGraph) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(graph)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_preset_graph(path: &std::path::Path) -> anyhow::Result<PresetGraph> {
    let json = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&json)?)
}
