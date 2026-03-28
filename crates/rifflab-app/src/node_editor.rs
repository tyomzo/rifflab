//! Node-graph effects editor: draggable effect nodes connected by cables.

use eframe::egui;
use rifflab_core::audio::{ParamDescriptor, ParamId, ParamKind};
use serde::{Deserialize, Serialize};

// ─── Data Model ──────────────────────────────────────────────────────────────

const NODE_WIDTH: f32 = 190.0;
const NODE_HEADER_HEIGHT: f32 = 22.0;
const PORT_RADIUS: f32 = 5.0;
const PORT_HIT_RADIUS: f32 = 10.0;

/// Unique port identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortId {
    pub node_id: u64,
    pub index: u8,
    pub is_output: bool,
}

/// A cable connecting an output port to an input port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cable {
    pub from: PortId, // output
    pub to: PortId,   // input
}

/// What kind of node this is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeKind {
    /// Audio input (live input / stems).
    Input,
    /// Audio output (to master mix).
    Output,
    /// A DSP effect from the registry.
    Effect { type_id: String },
    /// Crossover splitter: 1 input → 3 outputs (Low/Mid/High).
    CrossoverSplit {
        low_mid_hz: f32,
        mid_high_hz: f32,
    },
    /// Crossover merge: 3 inputs → 1 output, with per-band gain.
    CrossoverMerge {
        gains: [f32; 3],
    },
}

/// A node in the effects graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxNode {
    pub id: u64,
    pub pos: [f32; 2], // canvas position
    pub kind: NodeKind,
    pub label: String,
    /// Saved parameter values: Vec of (param_id, value). Loaded into param_cache on graph load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<(u32, f32)>,
    /// MIDI binding for selecting this node (for knob control).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi_binding: Option<crate::preset_graph::MidiBinding>,
}

impl FxNode {
    pub fn num_inputs(&self) -> u8 {
        match &self.kind {
            NodeKind::Input => 0,
            NodeKind::Output => 1,
            NodeKind::Effect { .. } => 1,
            NodeKind::CrossoverSplit { .. } => 1,
            NodeKind::CrossoverMerge { .. } => 3,
        }
    }

    pub fn num_outputs(&self) -> u8 {
        match &self.kind {
            NodeKind::Input => 1,
            NodeKind::Output => 0,
            NodeKind::Effect { .. } => 1,
            NodeKind::CrossoverSplit { .. } => 3,
            NodeKind::CrossoverMerge { .. } => 1,
        }
    }

    pub fn output_labels(&self) -> Vec<&str> {
        match &self.kind {
            NodeKind::CrossoverSplit { .. } => vec!["Low", "Mid", "High"],
            _ => vec!["Out"],
        }
    }

    pub fn input_labels(&self) -> Vec<&str> {
        match &self.kind {
            NodeKind::CrossoverMerge { .. } => vec!["Low", "Mid", "High"],
            _ => vec!["In"],
        }
    }
}

/// The full graph state (serializable for save/load).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxGraph {
    pub nodes: Vec<FxNode>,
    pub cables: Vec<Cable>,
    pub next_id: u64,
    /// Canvas pan offset.
    #[serde(default)]
    pub pan: [f32; 2],
}

impl Default for FxGraph {
    fn default() -> Self {
        Self::new_default()
    }
}

impl FxGraph {
    /// Create a default graph with Input → Output.
    pub fn new_default() -> Self {
        Self {
            nodes: vec![
                FxNode { id: 1, pos: [50.0, 100.0], kind: NodeKind::Input, label: "Input".into(), params: Vec::new(), midi_binding: None },
                FxNode { id: 2, pos: [400.0, 100.0], kind: NodeKind::Output, label: "Output".into(), params: Vec::new(), midi_binding: None },
            ],
            cables: vec![
                Cable {
                    from: PortId { node_id: 1, index: 0, is_output: true },
                    to: PortId { node_id: 2, index: 0, is_output: false },
                },
            ],
            next_id: 3,
            pan: [0.0, 0.0],
        }
    }

    pub fn add_node(&mut self, kind: NodeKind, label: String, pos: [f32; 2]) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(FxNode { id, pos, kind, label, params: Vec::new(), midi_binding: None });
        id
    }

    pub fn remove_node(&mut self, id: u64) {
        self.cables.retain(|c| c.from.node_id != id && c.to.node_id != id);
        self.nodes.retain(|n| n.id != id);
    }

    pub fn add_cable(&mut self, from: PortId, to: PortId) {
        // Remove any existing cable to this input port
        self.cables.retain(|c| c.to != to);
        self.cables.push(Cable { from, to });
    }

        #[allow(dead_code)]
    pub fn remove_cables_to(&mut self, port: PortId) {
        self.cables.retain(|c| c.to != port);
    }

    /// Save parameter cache values into node params for serialization.
    pub fn save_params_from_cache(&mut self, cache: &std::collections::HashMap<(u64, u32), f32>) {
        for node in &mut self.nodes {
            node.params.clear();
            for (&(nid, pid), &val) in cache {
                if nid == node.id {
                    node.params.push((pid, val));
                }
            }
            node.params.sort_by_key(|(pid, _)| *pid);
        }
    }

    /// Load node params into a parameter cache.
    pub fn load_params_to_cache(&self, cache: &mut std::collections::HashMap<(u64, u32), f32>) {
        cache.clear();
        for node in &self.nodes {
            for &(pid, val) in &node.params {
                cache.insert((node.id, pid), val);
            }
        }
    }

    pub fn find_node(&self, id: u64) -> Option<&FxNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn find_node_mut(&mut self, id: u64) -> Option<&mut FxNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
}

// ─── Interaction State ───────────────────────────────────────────────────────

/// Transient interaction state (not serialized).
pub struct NodeEditorState {
    /// Node being dragged, with offset from node origin.
    pub dragging_node: Option<(u64, egui::Vec2)>,
    /// Cable being dragged from a port (can be output OR input).
    pub dragging_cable: Option<(PortId, egui::Pos2)>,
    /// Currently selected port (for delete key).
    pub selected_port: Option<PortId>,
    /// Canvas is being panned.
        #[allow(dead_code)]
    pub panning: bool,
    /// Local parameter value cache — holds slider values between frames
    /// so they don't jump back when snapshots are stale or missing.
    pub param_cache: std::collections::HashMap<(u64, u32), f32>, // (node_id, param_id) → value
    /// Currently selected effect node (for MIDI knob binding).
    pub selected_node: Option<u64>,
}

impl Default for NodeEditorState {
    fn default() -> Self {
        Self {
            dragging_node: None,
            dragging_cable: None,
            selected_port: None,
            panning: false,
            param_cache: std::collections::HashMap::new(),
            selected_node: None,
        }
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

/// Colors for different node types.
fn node_color(kind: &NodeKind) -> egui::Color32 {
    match kind {
        NodeKind::Input => egui::Color32::from_rgb(60, 140, 200),
        NodeKind::Output => egui::Color32::from_rgb(200, 80, 80),
        NodeKind::Effect { .. } => egui::Color32::from_rgb(60, 180, 120),
        NodeKind::CrossoverSplit { .. } => egui::Color32::from_rgb(200, 160, 60),
        NodeKind::CrossoverMerge { .. } => egui::Color32::from_rgb(200, 160, 60),
    }
}

fn port_color(is_output: bool) -> egui::Color32 {
    if is_output {
        egui::Color32::from_rgb(120, 220, 160)
    } else {
        egui::Color32::from_rgb(100, 180, 240)
    }
}

/// Get the canvas position of a port on a node.
fn port_position(node: &FxNode, port_idx: u8, is_output: bool, pan: egui::Vec2) -> egui::Pos2 {
    let x = node.pos[0] + pan.x;
    let y = node.pos[1] + pan.y;

    if is_output {
        let count = node.num_outputs();
        let spacing = if count > 1 { 20.0 } else { 0.0 };
        let total_h = (count as f32 - 1.0) * spacing;
        let start_y = y + NODE_HEADER_HEIGHT + 10.0 - total_h / 2.0;
        egui::pos2(x + NODE_WIDTH, start_y + port_idx as f32 * spacing)
    } else {
        let count = node.num_inputs();
        let spacing = if count > 1 { 20.0 } else { 0.0 };
        let total_h = (count as f32 - 1.0) * spacing;
        let start_y = y + NODE_HEADER_HEIGHT + 10.0 - total_h / 2.0;
        egui::pos2(x, start_y + port_idx as f32 * spacing)
    }
}

/// Estimate node height based on kind and number of parameters.
fn node_height(node: &FxNode, param_count: usize) -> f32 {
    let ports = node.num_inputs().max(node.num_outputs()) as f32;
    let port_height = (ports - 1.0).max(0.0) * 20.0 + 20.0;
    // Each param: ~12px label + ~18px control = ~30px, plus padding
    let param_height = if param_count > 0 { param_count as f32 * 30.0 + 8.0 } else { 0.0 };
    NODE_HEADER_HEIGHT + port_height.max(param_height) + 12.0
}

/// Action requested by the node editor toolbar.
pub enum GraphAction {
    None,
    Save,
    Load,
    Changed,
}

/// Parameter snapshot for an effect node (passed from the app).
pub struct NodeParamSnapshot {
    pub node_id: u64,
    pub params: Vec<(ParamDescriptor, f32)>,
    pub bypassed: bool,
}

/// A parameter change from the node editor UI.
pub struct NodeParamChange {
    pub node_id: u64,
    pub param_id: ParamId,
    pub value: f32,
}

/// Draw the entire node editor. Returns parameter changes to apply.
pub fn draw_node_editor(
    ui: &mut egui::Ui,
    graph: &mut FxGraph,
    state: &mut NodeEditorState,
    registry_effects: &[(String, String, String)],
    param_snapshots: &[NodeParamSnapshot],
    registry: &rifflab_fx::registry::EffectRegistry,
) -> (Vec<NodeParamChange>, GraphAction) {
    let mut param_changes: Vec<NodeParamChange> = Vec::new();
    let mut action = GraphAction::None;

    // Toolbar
    ui.horizontal(|ui| {
        if ui.small_button("Save Graph").clicked() {
            action = GraphAction::Save;
        }
        if ui.small_button("Load Graph").clicked() {
            action = GraphAction::Load;
        }
        ui.separator();
        if ui.small_button("Reset").clicked() {
            *graph = FxGraph::new_default();
            state.param_cache.clear();
            action = GraphAction::Changed;
        }
    });

    let (response, painter) = ui.allocate_painter(
        ui.available_size(),
        egui::Sense::click_and_drag(),
    );
    let canvas_rect = response.rect;
    let pan = egui::vec2(graph.pan[0], graph.pan[1]);

    // Dark background
    painter.rect_filled(canvas_rect, 0.0, egui::Color32::from_rgb(20, 22, 26));

    // Grid dots
    let grid_size = 30.0;
    let dot_color = egui::Color32::from_rgb(35, 38, 44);
    let start_x = ((-pan.x / grid_size).floor() * grid_size) + pan.x;
    let start_y = ((-pan.y / grid_size).floor() * grid_size) + pan.y;
    let mut gx = canvas_rect.left() + start_x % grid_size;
    while gx < canvas_rect.right() {
        let mut gy = canvas_rect.top() + start_y % grid_size;
        while gy < canvas_rect.bottom() {
            painter.circle_filled(egui::pos2(gx, gy), 1.0, dot_color);
            gy += grid_size;
        }
        gx += grid_size;
    }

    // Draw cables (highlight if connected to selected port)
    for cable in &graph.cables {
        let from_node = graph.nodes.iter().find(|n| n.id == cable.from.node_id);
        let to_node = graph.nodes.iter().find(|n| n.id == cable.to.node_id);
        if let (Some(from_n), Some(to_n)) = (from_node, to_node) {
            let p0 = port_position(from_n, cable.from.index, true, pan) + canvas_rect.left_top().to_vec2();
            let p1 = port_position(to_n, cable.to.index, false, pan) + canvas_rect.left_top().to_vec2();
            let is_selected = state.selected_port.map_or(false, |sp| sp == cable.from || sp == cable.to);
            let color = if is_selected {
                egui::Color32::from_rgb(255, 220, 80)
            } else {
                egui::Color32::from_rgb(100, 180, 140)
            };
            let width = if is_selected { 3.0 } else { 2.0 };
            draw_cable_width(&painter, p0, p1, color, width);
        }
    }

    // Draw cable being dragged
    if let Some((port, mouse_pos)) = &state.dragging_cable {
        if let Some(from_node) = graph.nodes.iter().find(|n| n.id == port.node_id) {
            let port_pos = port_position(from_node, port.index, port.is_output, pan) + canvas_rect.left_top().to_vec2();
            let (p0, p1) = if port.is_output {
                (port_pos, *mouse_pos)
            } else {
                (*mouse_pos, port_pos)
            };
            draw_cable(&painter, p0, p1, egui::Color32::from_rgb(180, 220, 100));
        }
    }

    // Draw nodes (collect interactions to avoid borrow issues)
    let mut node_drag_start: Option<(u64, egui::Vec2)> = None;
    let mut cable_drag_start: Option<PortId> = None;
    let mut cable_drop_target: Option<PortId> = None;
    let mut port_clicked: Option<PortId> = None;
    let mut remove_node_id: Option<u64> = None;

    for node in &graph.nodes {
        let nx = canvas_rect.left() + node.pos[0] + pan.x;
        let ny = canvas_rect.top() + node.pos[1] + pan.y;
        let snap = param_snapshots.iter().find(|s| s.node_id == node.id);
        let param_count = if let Some(s) = snap {
            s.params.iter().filter(|(d, _)| d.name != "Bypass").count()
        } else if let NodeKind::Effect { type_id } = &node.kind {
            registry.create_effect(type_id)
                .map(|e| e.param_descriptors().iter().filter(|d| d.name != "Bypass").count())
                .unwrap_or(0)
        } else {
            0
        };
        let height = node_height(node, param_count);
        let node_rect = egui::Rect::from_min_size(egui::pos2(nx, ny), egui::vec2(NODE_WIDTH, height));

        // Skip if not visible
        if !canvas_rect.intersects(node_rect) {
            continue;
        }

        let color = node_color(&node.kind);
        let bg = egui::Color32::from_rgb(30, 33, 40);

        // Node body
        painter.rect_filled(node_rect, 4.0, bg);
        // Selection highlight
        if state.selected_node == Some(node.id) {
            painter.rect_stroke(node_rect.expand(2.0), 4.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 200, 255)), egui::StrokeKind::Outside);
        }
        // Header bar
        let header_rect = egui::Rect::from_min_size(
            node_rect.min,
            egui::vec2(NODE_WIDTH, NODE_HEADER_HEIGHT),
        );
        painter.rect_filled(header_rect, 4, color);
        // Label
        painter.text(
            header_rect.center(),
            egui::Align2::CENTER_CENTER,
            &node.label,
            egui::FontId::proportional(11.0),
            egui::Color32::WHITE,
        );
        // MIDI binding indicator (top-right of header)
        if let Some(ref binding) = node.midi_binding {
            painter.text(
                header_rect.right_top() + egui::vec2(-4.0, 3.0),
                egui::Align2::RIGHT_TOP,
                binding.label(),
                egui::FontId::proportional(8.0),
                egui::Color32::from_rgb(200, 200, 100),
            );
        }

        // Border
        painter.rect_stroke(node_rect, 4.0, egui::Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Outside);

        // Draw output ports
        for i in 0..node.num_outputs() {
            let port_id = PortId { node_id: node.id, index: i, is_output: true };
            let pos = port_position(node, i, true, pan) + canvas_rect.left_top().to_vec2();
            let is_selected = state.selected_port == Some(port_id);
            let color = if is_selected { egui::Color32::from_rgb(255, 255, 100) } else { port_color(true) };
            let radius = if is_selected { PORT_RADIUS + 2.0 } else { PORT_RADIUS };
            painter.circle_filled(pos, radius, color);
            if is_selected {
                painter.circle_stroke(pos, radius + 2.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 255, 100)));
            }
            let labels = node.output_labels();
            if labels.len() > 1 {
                painter.text(
                    pos + egui::vec2(-12.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    labels[i as usize],
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgb(160, 170, 180),
                );
            }
        }

        // Draw input ports
        for i in 0..node.num_inputs() {
            let port_id = PortId { node_id: node.id, index: i, is_output: false };
            let pos = port_position(node, i, false, pan) + canvas_rect.left_top().to_vec2();
            let is_selected = state.selected_port == Some(port_id);
            let color = if is_selected { egui::Color32::from_rgb(255, 255, 100) } else { port_color(false) };
            let radius = if is_selected { PORT_RADIUS + 2.0 } else { PORT_RADIUS };
            painter.circle_filled(pos, radius, color);
            if is_selected {
                painter.circle_stroke(pos, radius + 2.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 255, 100)));
            }
            let labels = node.input_labels();
            if labels.len() > 1 {
                painter.text(
                    pos + egui::vec2(12.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    labels[i as usize],
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgb(160, 170, 180),
                );
            }
        }

        // Render parameter sliders for effect nodes (connected or not)
        if let NodeKind::Effect { type_id } = &node.kind {
            // Get param descriptors and values:
            // 1. From snapshot (if connected and engine has the effect)
            // 2. From local cache (persists slider values between frames)
            // 3. From registry defaults (initial values)
            let base_params: Vec<(ParamDescriptor, f32)> = if let Some(s) = snap {
                s.params.clone()
            } else {
                registry.create_effect(type_id)
                    .map(|e| {
                        e.param_descriptors().into_iter()
                            .map(|d| { let v = e.get_param(d.id); (d, v) })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            // Apply cached values on top (so slider changes stick)
            let params: Vec<(ParamDescriptor, f32)> = base_params.into_iter()
                .map(|(d, v)| {
                    let cached = state.param_cache.get(&(node.id, d.id.0)).copied();
                    (d, cached.unwrap_or(v))
                })
                .collect();
            // Update cache from snapshot (so cache stays in sync with engine)
            if snap.is_some() {
                for (d, v) in &params {
                    if !state.param_cache.contains_key(&(node.id, d.id.0)) {
                        state.param_cache.insert((node.id, d.id.0), *v);
                    }
                }
            }
            let bypassed = snap.map_or(false, |s| s.bypassed);

            if !bypassed {
                let params_rect = egui::Rect::from_min_size(
                    egui::pos2(nx + 8.0, ny + NODE_HEADER_HEIGHT + 4.0),
                    egui::vec2(NODE_WIDTH - 16.0, height - NODE_HEADER_HEIGHT - 12.0),
                );
                if canvas_rect.intersects(params_rect) {
                    let node_id = node.id;
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(params_rect), |ui| {
                        ui.set_clip_rect(canvas_rect);
                        let param_width = NODE_WIDTH - 24.0;
                        for (desc, val) in &params {
                            if desc.name == "Bypass" { continue; }
                            let mut v = *val;
                            // Param name as tiny label above the control
                            ui.label(egui::RichText::new(&desc.name).size(8.0)
                                .color(egui::Color32::from_rgb(140, 145, 155)));
                            let changed = match &desc.kind {
                                ParamKind::Float => {
                                    let unit = desc.unit.clone();
                                    ui.add_sized(
                                        egui::vec2(param_width, 16.0),
                                        egui::Slider::new(&mut v, desc.min..=desc.max)
                                            .show_value(true)
                                            .custom_formatter(move |val, _| {
                                                if unit.is_empty() { format!("{:.2}", val) }
                                                else { format!("{:.1}{}", val, unit) }
                                            })
                                    ).changed()
                                }
                                ParamKind::Enum(labels) => {
                                    let cur = v.round() as usize;
                                    let cur_label = labels.get(cur).cloned().unwrap_or_default();
                                    let mut changed = false;
                                    egui::ComboBox::from_id_salt(format!("ne_{}_{}", node_id, desc.id.0))
                                        .selected_text(&cur_label)
                                        .width(param_width)
                                        .show_ui(ui, |ui| {
                                            for (i, l) in labels.iter().enumerate() {
                                                if ui.selectable_value(&mut v, i as f32, l).changed() {
                                                    changed = true;
                                                }
                                            }
                                        });
                                    changed
                                }
                                ParamKind::Int => {
                                    let mut iv = v.round() as i32;
                                    let c = ui.add_sized(
                                        egui::vec2(param_width, 16.0),
                                        egui::Slider::new(&mut iv, desc.min as i32..=desc.max as i32)
                                    ).changed();
                                    if c { v = iv as f32; }
                                    c
                                }
                                ParamKind::Bool => {
                                    let mut b = v > 0.5;
                                    let c = ui.checkbox(&mut b, "").changed();
                                    if c { v = if b { 1.0 } else { 0.0 }; }
                                    c
                                }
                            };
                            if changed {
                                state.param_cache.insert((node_id, desc.id.0), v);
                                param_changes.push(NodeParamChange { node_id, param_id: desc.id, value: v });
                            }
                        }
                    });
                }
            }
        }

        // Check port hits (extend beyond node rect for ports on the edge)
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            if canvas_rect.contains(pointer) {
                let pressed = ui.input(|inp| inp.pointer.primary_pressed());
                let idle = state.dragging_cable.is_none() && state.dragging_node.is_none();

                // Output ports — drag start or selection
                for i in 0..node.num_outputs() {
                    let pos = port_position(node, i, true, pan) + canvas_rect.left_top().to_vec2();
                    if pos.distance(pointer) < PORT_HIT_RADIUS {
                        let port_id = PortId { node_id: node.id, index: i, is_output: true };
                        if pressed && idle {
                            cable_drag_start = Some(port_id);
                            port_clicked = Some(port_id);
                        }
                        // Drop target when dragging FROM an input port
                        if state.dragging_cable.as_ref().map_or(false, |(p, _)| !p.is_output) {
                            cable_drop_target = Some(port_id);
                        }
                    }
                }
                // Input ports — drag start or selection, or drop target
                for i in 0..node.num_inputs() {
                    let pos = port_position(node, i, false, pan) + canvas_rect.left_top().to_vec2();
                    if pos.distance(pointer) < PORT_HIT_RADIUS {
                        let port_id = PortId { node_id: node.id, index: i, is_output: false };
                        if pressed && idle {
                            cable_drag_start = Some(port_id);
                            port_clicked = Some(port_id);
                        }
                        // Drop target when dragging FROM an output port
                        if state.dragging_cable.as_ref().map_or(false, |(p, _)| p.is_output) {
                            cable_drop_target = Some(port_id);
                        }
                    }
                }
                // Node drag — only from the header bar (not the param area)
                let header_rect = egui::Rect::from_min_size(
                    node_rect.min,
                    egui::vec2(NODE_WIDTH, NODE_HEADER_HEIGHT),
                );
                if cable_drag_start.is_none() && header_rect.contains(pointer) {
                    if pressed && idle {
                        let offset = pointer - node_rect.min;
                        node_drag_start = Some((node.id, offset));
                    }
                }
            }
        }
    }

    // Handle node dragging and selection
    if let Some((id, offset)) = node_drag_start {
        state.dragging_node = Some((id, offset));
        // Select node on click (for MIDI knob binding)
        if graph.find_node(id).map_or(false, |n| matches!(n.kind, NodeKind::Effect { .. })) {
            state.selected_node = Some(id);
        }
    }
    if let Some((id, offset)) = state.dragging_node {
        if ui.input(|i| i.pointer.primary_down()) {
            if let Some(pointer) = ui.ctx().pointer_latest_pos() {
                if let Some(node) = graph.find_node_mut(id) {
                    node.pos[0] = pointer.x - canvas_rect.left() - pan.x - offset.x;
                    node.pos[1] = pointer.y - canvas_rect.top() - pan.y - offset.y;
                }
            }
        } else {
            state.dragging_node = None;
        }
    }

    // Handle port selection (click without drag)
    if let Some(port) = port_clicked {
        state.selected_port = Some(port);
    } else if ui.input(|i| i.pointer.primary_pressed()) {
        // Click on empty space deselects
        if state.dragging_cable.is_none() && state.dragging_node.is_none() {
            state.selected_port = None;
        }
    }

    // Handle Delete key on selected port — remove connected cables
    if state.selected_port.is_some() && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
        let port = state.selected_port.unwrap();
        graph.cables.retain(|c| {
            if port.is_output {
                c.from != port
            } else {
                c.to != port
            }
        });
        state.selected_port = None;
    }

    // Handle cable dragging (from either input or output port)
    if let Some(port) = cable_drag_start {
        state.dragging_cable = Some((port, egui::Pos2::ZERO));
    }
    if state.dragging_cable.is_some() {
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let port = state.dragging_cable.as_ref().unwrap().0;
            state.dragging_cable = Some((port, pointer));
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            // Mouse released — check for drop target
            if let Some(target) = cable_drop_target {
                let source = state.dragging_cable.unwrap().0;
                if source.node_id != target.node_id {
                    // Normalize: ensure cable goes from output to input
                    let (from, to) = if source.is_output {
                        (source, target)
                    } else {
                        (target, source)
                    };
                    if from.is_output && !to.is_output {
                        graph.add_cable(from, to);
                    }
                }
            }
            state.dragging_cable = None;
        }
    }

    // Canvas panning (drag on empty background only)
    if response.dragged() && state.dragging_node.is_none() && state.dragging_cable.is_none() {
        let delta = response.drag_delta();
        graph.pan[0] += delta.x;
        graph.pan[1] += delta.y;
    }

    // Right-click context menu
    response.context_menu(|ui| {
        let click_pos = response.interact_pointer_pos()
            .map(|p| [p.x - canvas_rect.left() - pan.x, p.y - canvas_rect.top() - pan.y])
            .unwrap_or([200.0, 100.0]);

        ui.label(egui::RichText::new("Add Node").strong().size(11.0));
        ui.separator();

        if ui.button("Crossover Split").clicked() {
            let _id = graph.add_node(
                NodeKind::CrossoverSplit { low_mid_hz: 250.0, mid_high_hz: 2500.0 },
                "Crossover".into(),
                click_pos,
            );
            ui.close_menu();
        }
        if ui.button("Crossover Merge").clicked() {
            graph.add_node(
                NodeKind::CrossoverMerge { gains: [0.0; 3] },
                "Merge".into(),
                click_pos,
            );
            ui.close_menu();
        }
        ui.separator();
        for (type_id, name, _) in registry_effects {
            if type_id == "builtin:tuner" || type_id == "builtin:multiband" { continue; }
            if ui.button(name).clicked() {
                graph.add_node(
                    NodeKind::Effect { type_id: type_id.clone() },
                    name.clone(),
                    click_pos,
                );
                ui.close_menu();
            }
        }

        // Delete hovered node
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            for node in &graph.nodes {
                let nx = canvas_rect.left() + node.pos[0] + pan.x;
                let ny = canvas_rect.top() + node.pos[1] + pan.y;
                let node_rect = egui::Rect::from_min_size(
                    egui::pos2(nx, ny),
                    egui::vec2(NODE_WIDTH, node_height(node, 0)),
                );
                if node_rect.contains(pointer) {
                    if !matches!(node.kind, NodeKind::Input | NodeKind::Output) {
                        ui.separator();
                        if ui.button(format!("Delete {}", node.label)).clicked() {
                            remove_node_id = Some(node.id);
                            ui.close_menu();
                        }
                    }
                    break;
                }
            }
        }
    });

    // Apply deferred removal
    if let Some(id) = remove_node_id {
        graph.remove_node(id);
    }

    (param_changes, action)
}

/// Draw a Bezier cable between two points.
fn draw_cable(painter: &egui::Painter, p0: egui::Pos2, p1: egui::Pos2, color: egui::Color32) {
    draw_cable_width(painter, p0, p1, color, 2.0);
}

fn draw_cable_width(painter: &egui::Painter, p0: egui::Pos2, p1: egui::Pos2, color: egui::Color32, width: f32) {
    let dx = (p1.x - p0.x).abs() * 0.5;
    let cp0 = egui::pos2(p0.x + dx, p0.y);
    let cp1 = egui::pos2(p1.x - dx, p1.y);
    painter.add(egui::Shape::CubicBezier(egui::epaint::CubicBezierShape::from_points_stroke(
        [p0, cp0, cp1, p1],
        false,
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(width, color),
    )));
}

// ─── Graph Compilation (Visual → Audio Engine) ──────────────────────────────

/// Compiled audio routing from the visual graph.
pub enum CompiledRoute {
    /// Linear chain of effect type IDs (Input → Effect → ... → Output).
    SingleChain(Vec<String>),
    /// Multiband routing: crossover freqs + per-band effect chains.
    Multiband {
        low_mid_hz: f32,
        mid_high_hz: f32,
        gains: [f32; 3],
        low_chain: Vec<String>,
        mid_chain: Vec<String>,
        high_chain: Vec<String>,
    },
    /// Empty (Input → Output directly, or broken graph).
    Empty,
}

impl FxGraph {
    /// Compile the visual graph into an audio routing configuration.
    /// Traces cables from Input to Output, collecting effect type_ids in order.
    pub fn compile(&self) -> CompiledRoute {
        // Find the Input node
        let input_node = match self.nodes.iter().find(|n| matches!(n.kind, NodeKind::Input)) {
            Some(n) => n,
            None => return CompiledRoute::Empty,
        };

        // Trace from Input's output port
        let start_port = PortId { node_id: input_node.id, index: 0, is_output: true };
        let chain = self.trace_chain(start_port);

        // Check if the chain contains a crossover split
        let split_pos = chain.iter().position(|id| {
            self.find_node(*id).map_or(false, |n| matches!(n.kind, NodeKind::CrossoverSplit { .. }))
        });

        if let Some(split_idx) = split_pos {
            let split_node_id = chain[split_idx];
            let split_node = self.find_node(split_node_id).unwrap();

            // Get crossover params
            let (low_mid_hz, mid_high_hz) = match &split_node.kind {
                NodeKind::CrossoverSplit { low_mid_hz, mid_high_hz } => (*low_mid_hz, *mid_high_hz),
                _ => (250.0, 2500.0),
            };

            // Collect effects before the split
            let _pre_split: Vec<String> = chain[..split_idx].iter()
                .filter_map(|id| match &self.find_node(*id)?.kind {
                    NodeKind::Effect { type_id } => Some(type_id.clone()),
                    _ => None,
                })
                .collect();

            // Trace each band output from the split node
            let _band_labels = ["Low", "Mid", "High"];
            let mut band_chains: [Vec<String>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            let mut gains = [0.0f32; 3];

            for band_idx in 0..3u8 {
                let band_port = PortId { node_id: split_node_id, index: band_idx, is_output: true };
                let band_node_ids = self.trace_chain(band_port);
                for node_id in &band_node_ids {
                    if let Some(node) = self.find_node(*node_id) {
                        match &node.kind {
                            NodeKind::Effect { type_id } => {
                                band_chains[band_idx as usize].push(type_id.clone());
                            }
                            NodeKind::CrossoverMerge { gains: g } => {
                                gains = *g;
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Prepend pre-split effects to each band (they apply to all bands)
            // Actually, pre-split effects should be in a separate single chain before the split.
            // For now, the multiband route only uses effects after the split.

            CompiledRoute::Multiband {
                low_mid_hz,
                mid_high_hz,
                gains,
                low_chain: band_chains[0].clone(),
                mid_chain: band_chains[1].clone(),
                high_chain: band_chains[2].clone(),
            }
        } else {
            // Simple linear chain
            let has_output = chain.iter().any(|id| {
                self.find_node(*id).map_or(false, |n| matches!(n.kind, NodeKind::Output))
            });
            let effects: Vec<String> = chain.iter()
                .filter_map(|id| match &self.find_node(*id)?.kind {
                    NodeKind::Effect { type_id } => Some(type_id.clone()),
                    _ => None,
                })
                .collect();

            if has_output {
                // Connected path (may have zero effects = passthrough)
                CompiledRoute::SingleChain(effects)
            } else {
                // No path to output
                CompiledRoute::Empty
            }
        }
    }

    /// Trace a chain of nodes starting from a given output port.
    /// Returns node IDs in order (excluding the starting node).
    pub fn trace_chain(&self, start_port: PortId) -> Vec<u64> {
        let mut result = Vec::new();
        let mut current_port = start_port;

        loop {
            // Find cable from current output port
            let cable = self.cables.iter().find(|c| c.from == current_port);
            let cable = match cable {
                Some(c) => c,
                None => break, // dead end
            };

            let next_node_id = cable.to.node_id;

            // Avoid loops
            if result.contains(&next_node_id) {
                break;
            }

            result.push(next_node_id);

            // Find the next node
            let next_node = match self.find_node(next_node_id) {
                Some(n) => n,
                None => break,
            };

            // If this node is Output or CrossoverSplit (which has multiple outputs), stop
            match &next_node.kind {
                NodeKind::Output => break,
                NodeKind::CrossoverSplit { .. } => break, // caller handles multi-output
                NodeKind::CrossoverMerge { .. } => break, // stop at merge
                _ => {
                    // Continue from this node's output port 0
                    current_port = PortId { node_id: next_node_id, index: 0, is_output: true };
                }
            }
        }

        result
    }
}

// ─── Persistence ─────────────────────────────────────────────────────────────

/// Save graph to JSON.
pub fn save_graph(path: &std::path::Path, graph: &FxGraph) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(graph)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Load graph from JSON.
pub fn load_graph(path: &std::path::Path) -> Result<FxGraph, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&json)?)
}
