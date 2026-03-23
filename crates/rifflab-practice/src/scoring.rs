use rifflab_core::practice::ComparisonFrame;

/// Per-session scoring accumulator.
pub struct SessionScorer {
    frames: Vec<ComparisonFrame>,
    notes_total: u32,
    notes_correct: u32,
    total_cents_deviation: f64,
    total_timing_offset: f64,
}

impl SessionScorer {
    pub fn new() -> Self {
        Self {
            frames: Vec::new(),
            notes_total: 0,
            notes_correct: 0,
            total_cents_deviation: 0.0,
            total_timing_offset: 0.0,
        }
    }

    pub fn feed(&mut self, frame: ComparisonFrame) {
        if frame.reference_note.is_some() {
            self.notes_total += 1;
            if frame.note_correct {
                self.notes_correct += 1;
            }
            self.total_cents_deviation += frame.cents_deviation.abs() as f64;
            self.total_timing_offset += frame.timing_offset_ms.abs() as f64;
        }
        self.frames.push(frame);
    }

    /// Compute aggregate session score (0.0–100.0).
    pub fn score(&self) -> f32 {
        if self.notes_total == 0 {
            return 0.0;
        }
        let correctness = self.notes_correct as f32 / self.notes_total as f32;
        let avg_cents = self.total_cents_deviation as f32 / self.notes_total as f32;
        let pitch_score = (1.0 - (avg_cents / 50.0).min(1.0)).max(0.0);

        // Weighted: 50% correctness, 50% pitch accuracy
        (correctness * 0.5 + pitch_score * 0.5) * 100.0
    }

    pub fn notes_correct(&self) -> u32 {
        self.notes_correct
    }

    pub fn notes_total(&self) -> u32 {
        self.notes_total
    }
}

impl Default for SessionScorer {
    fn default() -> Self {
        Self::new()
    }
}
