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

/// A titled, bordered section used to group related controls.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let v = ui.visuals();
    egui::Frame::NONE
        .fill(v.faint_bg_color)
        .stroke(v.widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(egui::RichText::new(title).strong());
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner
}
