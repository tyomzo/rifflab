//! Scrolling tablature renderer for egui.
//!
//! Renders a TabDocument as a scrolling 4-line bass tab with playhead,
//! note highlighting, and click-to-seek.

use egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use rifflab_tab::model::{TabDocument, TabNote, Technique};

/// Persistent state for the tab view.
pub struct TabViewState {
    /// Horizontal scroll offset in seconds.
    pub scroll_offset: f64,
    /// Zoom: seconds visible in the viewport.
    pub visible_seconds: f64,
    /// Selected note ID.
    pub selected_note: Option<uuid::Uuid>,
    /// Whether to show ASCII text view instead of graphical.
    pub ascii_mode: bool,
    /// Fret input buffer (for two-digit frets: first digit stored, timeout 500ms).
    pub fret_input: Option<(u8, std::time::Instant)>,
    /// Undo stack: snapshots of notes before edits.
    pub undo_stack: Vec<Vec<TabNote>>,
    /// Redo stack.
    pub redo_stack: Vec<Vec<TabNote>>,
}

impl Default for TabViewState {
    fn default() -> Self {
        Self {
            scroll_offset: 0.0,
            visible_seconds: 8.0,
            selected_note: None,
            ascii_mode: false,
            fret_input: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

// Colors (dark theme)
const BG_COLOR: Color32 = Color32::from_rgb(25, 25, 30);
const STRING_COLOR: Color32 = Color32::from_rgb(70, 70, 80);
const FRET_COLOR: Color32 = Color32::from_rgb(220, 220, 220);
const ACTIVE_COLOR: Color32 = Color32::from_rgb(100, 255, 150);
const PAST_COLOR: Color32 = Color32::from_rgba_premultiplied(160, 160, 160, 128);
const PLAYHEAD_COLOR: Color32 = Color32::from_rgb(255, 100, 80);
const MEASURE_COLOR: Color32 = Color32::from_rgb(50, 50, 60);
const SECTION_COLOR: Color32 = Color32::from_rgb(180, 180, 100);
const SELECTED_COLOR: Color32 = Color32::from_rgb(80, 180, 255);

const STRING_LABELS: [&str; 4] = ["G", "D", "A", "E"];
const STRING_SPACING: f32 = 28.0;
const LABEL_WIDTH: f32 = 20.0;
const PLAYHEAD_X_RATIO: f32 = 0.25;

/// Render result from the tab view.
pub enum TabViewAction {
    None,
    Seek(f64), // Seek to this time in seconds
}

/// Draw the scrolling tab view.
pub fn draw_tab_view(
    ui: &mut egui::Ui,
    tab: &mut TabDocument,
    playback_secs: f64,
    is_playing: bool,
    state: &mut TabViewState,
) -> TabViewAction {
    let mut action = TabViewAction::None;

    // Toolbar
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.ascii_mode, "ASCII");
        ui.separator();
        ui.label(egui::RichText::new(format!("{:.0} BPM", tab.tempo.initial_bpm)).size(10.0));
        ui.label(egui::RichText::new(format!("{}/{}", tab.time_signature.beats_per_measure, tab.time_signature.beat_unit)).size(10.0));
        ui.label(egui::RichText::new(format!("{} notes", tab.notes.len())).size(10.0));
        if let Some(ref artist) = tab.artist {
            ui.label(egui::RichText::new(format!("{} — {}", artist, tab.title)).size(10.0).color(Color32::from_rgb(180, 180, 200)));
        } else {
            ui.label(egui::RichText::new(&tab.title).size(10.0).color(Color32::from_rgb(180, 180, 200)));
        }
    });

    if state.ascii_mode {
        return draw_ascii_view(ui, tab, playback_secs, state);
    }

    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // Background
    painter.rect_filled(rect, 0.0, BG_COLOR);

    let content_rect = Rect::from_min_size(
        rect.min + Vec2::new(LABEL_WIDTH, 20.0),
        Vec2::new(rect.width() - LABEL_WIDTH, STRING_SPACING * 5.0),
    );

    // Auto-scroll during playback
    if is_playing {
        state.scroll_offset = playback_secs - state.visible_seconds * PLAYHEAD_X_RATIO as f64;
    }

    // Mouse scroll for zoom/pan
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta);
        if ui.input(|i| i.modifiers.ctrl) {
            // Ctrl+scroll = zoom
            let zoom_factor = 1.0 + scroll.y as f64 * 0.01;
            state.visible_seconds = (state.visible_seconds / zoom_factor).clamp(2.0, 30.0);
        } else {
            // Scroll = pan
            state.scroll_offset -= scroll.x as f64 * 0.01 * state.visible_seconds;
        }
    }

    let time_to_x = |t: f64| -> f32 {
        let frac = (t - state.scroll_offset) / state.visible_seconds;
        content_rect.left() + frac as f32 * content_rect.width()
    };
    let x_to_time = |x: f32| -> f64 {
        let frac = (x - content_rect.left()) / content_rect.width();
        state.scroll_offset + frac as f64 * state.visible_seconds
    };
    let string_y = |string: u8| -> f32 {
        // G=3 at top, E=0 at bottom
        let idx = 3 - string;
        content_rect.top() + idx as f32 * STRING_SPACING + STRING_SPACING * 0.5
    };

    // Draw string lines
    for s in 0..4u8 {
        let y = string_y(s);
        painter.line_segment(
            [Pos2::new(content_rect.left(), y), Pos2::new(content_rect.right(), y)],
            Stroke::new(1.0, STRING_COLOR),
        );
    }

    // String labels
    for (i, label) in STRING_LABELS.iter().enumerate() {
        let s = 3 - i as u8;
        let y = string_y(s);
        painter.text(
            Pos2::new(rect.left() + 4.0, y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::monospace(12.0),
            Color32::from_rgb(120, 120, 140),
        );
    }

    // Draw measure lines
    let beat_duration = 60.0 / tab.tempo.initial_bpm;
    let measure_duration = beat_duration * tab.time_signature.beats_per_measure as f64;
    {
        let start_measure = (state.scroll_offset / measure_duration).floor() as i64;
        let end_measure = ((state.scroll_offset + state.visible_seconds) / measure_duration).ceil() as i64;
        for m in start_measure..=end_measure {
            let t = m as f64 * measure_duration;
            let x = time_to_x(t);
            if x >= content_rect.left() && x <= content_rect.right() {
                painter.line_segment(
                    [Pos2::new(x, content_rect.top()), Pos2::new(x, content_rect.bottom())],
                    Stroke::new(1.0, MEASURE_COLOR),
                );
                // Measure number
                if m > 0 {
                    painter.text(
                        Pos2::new(x + 2.0, content_rect.top() - 2.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{}", m),
                        FontId::proportional(8.0),
                        Color32::from_rgb(80, 80, 90),
                    );
                }
            }
            // Beat ticks
            for beat in 1..tab.time_signature.beats_per_measure {
                let bt = t + beat as f64 * beat_duration;
                let bx = time_to_x(bt);
                if bx >= content_rect.left() && bx <= content_rect.right() {
                    painter.line_segment(
                        [Pos2::new(bx, content_rect.top()), Pos2::new(bx, content_rect.top() + 4.0)],
                        Stroke::new(0.5, Color32::from_rgb(45, 45, 55)),
                    );
                }
            }
        }
    }

    // Draw notes
    for note in &tab.notes {
        let x = time_to_x(note.time_secs);
        if x < content_rect.left() - 20.0 || x > content_rect.right() + 20.0 {
            continue;
        }
        let y = string_y(note.string);

        let is_active = (note.time_secs - playback_secs).abs() < beat_duration * 0.5;
        let is_selected = state.selected_note == Some(note.id);
        let is_past = note.time_secs < playback_secs;

        let color = if is_selected {
            SELECTED_COLOR
        } else if is_active {
            ACTIVE_COLOR
        } else if is_past {
            PAST_COLOR
        } else {
            FRET_COLOR
        };

        let font_size = if is_active { 14.0 } else { 12.0 };

        // Fret number
        let text = format!("{}", note.fret);
        // Background pill for readability
        let text_rect = painter.text(
            Pos2::new(x, y),
            egui::Align2::CENTER_CENTER,
            &text,
            FontId::monospace(font_size),
            color,
        );
        painter.rect_filled(text_rect.expand(1.0), 2.0, Color32::from_rgba_premultiplied(25, 25, 30, 200));
        painter.text(
            Pos2::new(x, y),
            egui::Align2::CENTER_CENTER,
            &text,
            FontId::monospace(font_size),
            color,
        );

        // Technique annotation
        if let Some(tech) = note.technique {
            let label = match tech {
                Technique::HammerOn => "H",
                Technique::PullOff => "P",
                Technique::SlideUp => "/",
                Technique::SlideDown => "\\",
                Technique::Mute => "X",
                Technique::Vibrato => "~",
                Technique::Bend => "b",
                Technique::Ghost => "()",
                Technique::Harmonic => "*",
                Technique::TapOn => "T",
                _ => "",
            };
            if !label.is_empty() {
                painter.text(
                    Pos2::new(x + 8.0, y - 8.0),
                    egui::Align2::LEFT_BOTTOM,
                    label,
                    FontId::proportional(8.0),
                    Color32::from_rgb(180, 140, 100),
                );
            }
        }

        // Section label
        if let Some(ref section) = note.section {
            painter.text(
                Pos2::new(x, content_rect.top() - 12.0),
                egui::Align2::LEFT_BOTTOM,
                section,
                FontId::proportional(10.0),
                SECTION_COLOR,
            );
        }
    }

    // Playhead
    let playhead_x = time_to_x(playback_secs);
    if playhead_x >= content_rect.left() && playhead_x <= content_rect.right() {
        painter.line_segment(
            [Pos2::new(playhead_x, content_rect.top() - 5.0), Pos2::new(playhead_x, content_rect.bottom() + 5.0)],
            Stroke::new(2.0, PLAYHEAD_COLOR),
        );
    }

    // Click to select note or seek
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            if content_rect.contains(pos) {
                // Check if click is near a note
                let click_time = x_to_time(pos.x);
                let click_string = {
                    let mut closest = 0u8;
                    let mut min_dist = f32::MAX;
                    for s in 0..4u8 {
                        let d = (pos.y - string_y(s)).abs();
                        if d < min_dist { min_dist = d; closest = s; }
                    }
                    if min_dist < STRING_SPACING * 0.6 { Some(closest) } else { None }
                };

                let mut hit_note = None;
                if let Some(click_s) = click_string {
                    for note in tab.notes.iter() {
                        if note.string == click_s && (note.time_secs - click_time).abs() < beat_duration * 0.3 {
                            hit_note = Some(note.id);
                            break;
                        }
                    }
                }

                if let Some(id) = hit_note {
                    state.selected_note = Some(id);
                } else {
                    state.selected_note = None;
                    action = TabViewAction::Seek(click_time.max(0.0));
                }
            }
        }
    }

    // Keyboard editing (when a note is selected)
    if state.selected_note.is_some() && response.hovered() {
        let events: Vec<egui::Event> = ui.input(|i| i.events.clone());
        for event in &events {
            match event {
                egui::Event::Key { key: egui::Key::Delete, pressed: true, .. }
                | egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => {
                    if let Some(sel_id) = state.selected_note {
                        // Undo snapshot
                        state.undo_stack.push(tab.notes.clone());
                        state.redo_stack.clear();
                        tab.notes.retain(|n| n.id != sel_id);
                        state.selected_note = None;
                    }
                }
                egui::Event::Key { key: egui::Key::Tab, pressed: true, modifiers, .. } => {
                    // Tab/Shift+Tab: select next/previous note
                    if let Some(sel_id) = state.selected_note {
                        if let Some(idx) = tab.notes.iter().position(|n| n.id == sel_id) {
                            let new_idx = if modifiers.shift {
                                idx.saturating_sub(1)
                            } else {
                                (idx + 1).min(tab.notes.len().saturating_sub(1))
                            };
                            state.selected_note = Some(tab.notes[new_idx].id);
                        }
                    }
                }
                egui::Event::Text(text) => {
                    // Type digit to change fret of selected note
                    if let Some(digit) = text.chars().next().and_then(|c| c.to_digit(10)) {
                        let digit = digit as u8;
                        // Two-digit support: if previous digit was typed within 500ms, combine
                        let fret = if let Some((prev_digit, when)) = state.fret_input.take() {
                            if when.elapsed() < std::time::Duration::from_millis(500) {
                                let combined = prev_digit * 10 + digit;
                                if combined <= 24 { combined } else { digit }
                            } else {
                                state.fret_input = Some((digit, std::time::Instant::now()));
                                return action; // Wait for possible second digit
                            }
                        } else {
                            state.fret_input = Some((digit, std::time::Instant::now()));
                            return action; // Wait for possible second digit
                        };

                        if let Some(sel_id) = state.selected_note {
                            state.undo_stack.push(tab.notes.clone());
                            state.redo_stack.clear();
                            if let Some(note) = tab.notes.iter_mut().find(|n| n.id == sel_id) {
                                note.fret = fret;
                                note.source = rifflab_tab::model::NoteSource::UserEdit;
                                note.confidence = 1.0;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Apply single-digit fret after timeout
        if let Some((digit, when)) = state.fret_input {
            if when.elapsed() >= std::time::Duration::from_millis(500) {
                state.fret_input = None;
                if let Some(sel_id) = state.selected_note {
                    state.undo_stack.push(tab.notes.clone());
                    state.redo_stack.clear();
                    if let Some(note) = tab.notes.iter_mut().find(|n| n.id == sel_id) {
                        note.fret = digit;
                        note.source = rifflab_tab::model::NoteSource::UserEdit;
                        note.confidence = 1.0;
                    }
                }
            }
        }

        // Ctrl+Z undo, Ctrl+Shift+Z redo
        let (undo, redo) = ui.input(|i| {
            (i.modifiers.ctrl && i.key_pressed(egui::Key::Z) && !i.modifiers.shift,
             i.modifiers.ctrl && i.key_pressed(egui::Key::Z) && i.modifiers.shift)
        });
        if undo {
            if let Some(snapshot) = state.undo_stack.pop() {
                state.redo_stack.push(tab.notes.clone());
                tab.notes = snapshot;
            }
        }
        if redo {
            if let Some(snapshot) = state.redo_stack.pop() {
                state.undo_stack.push(tab.notes.clone());
                tab.notes = snapshot;
            }
        }
    }

    action
}

/// ASCII text view (Task 08).
fn draw_ascii_view(
    ui: &mut egui::Ui,
    tab: &TabDocument,
    playback_secs: f64,
    _state: &mut TabViewState,
) -> TabViewAction {
    let ascii = rifflab_tab::render_ascii::tab_to_ascii(tab, 4);
    let cursor_col = rifflab_tab::render_ascii::position_to_column(playback_secs, tab, 4);

    egui::ScrollArea::vertical().show(ui, |ui| {
        for line in ascii.lines() {
            // Highlight the current column character
            let annotated = if cursor_col < line.len() {
                let (before, rest) = line.split_at(cursor_col);
                if let Some((cursor_ch, after)) = rest.split_at(1.min(rest.len())).into() {
                    format!("{}{}{}", before, cursor_ch, after)
                } else {
                    line.to_string()
                }
            } else {
                line.to_string()
            };
            ui.label(egui::RichText::new(&annotated).font(FontId::monospace(11.0)).color(Color32::from_rgb(200, 200, 200)));
        }
    });

    TabViewAction::None
}
