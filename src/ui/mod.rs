pub mod gens;
pub mod keyboard;
pub mod waveform;
pub mod zones;

use egui::Response;

/// Collects whether the document was modified during a frame (for undo/dirty tracking).
#[derive(Default)]
pub struct Edit {
    pub changed: bool,
    /// Forces a separate undo step (add/delete/etc), even mid-transaction.
    pub structural: bool,
}

impl Edit {
    pub fn track(&mut self, r: &Response) {
        if r.changed() {
            self.changed = true;
        }
    }

    pub fn structural(&mut self) {
        self.changed = true;
        self.structural = true;
    }
}

pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}
