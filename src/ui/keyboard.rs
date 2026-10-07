//! Clickable 128-key piano.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::is_black_key;
use crate::sf2::generators::note_name;

pub struct PianoOutput {
    pub note_on: Option<u8>,
    pub note_off: Option<u8>,
}

struct Layout {
    rect: Rect,
    white_w: f32,
}

impl Layout {
    fn white_index(key: u8) -> f32 {
        const W: [f32; 12] = [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0];
        (key / 12) as f32 * 7.0 + W[(key % 12) as usize]
    }

    fn key_rect(&self, key: u8) -> Rect {
        let x = self.rect.left() + Self::white_index(key) * self.white_w;
        if is_black_key(key) {
            let w = self.white_w * 0.62;
            Rect::from_min_size(pos2(x + self.white_w * 0.5 - w * 0.5, self.rect.top()), vec2(w, self.rect.height() * 0.6))
        } else {
            Rect::from_min_size(pos2(x, self.rect.top()), vec2(self.white_w, self.rect.height()))
        }
    }

    fn hit(&self, p: Pos2) -> Option<u8> {
        (0..128u8)
            .filter(|&k| is_black_key(k))
            .chain((0..128u8).filter(|&k| !is_black_key(k)))
            .find(|&k| self.key_rect(k).contains(p))
    }
}

/// `sounding` = keys currently held; `mapped` = keys that produce sound for the selection.
pub fn piano(ui: &mut Ui, height: f32, sounding: &[bool; 128], mapped: &[bool; 128], mouse_note: &mut Option<u8>) -> PianoOutput {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let layout = Layout { rect, white_w: width / 75.0 };
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    let accent = visuals.selection.bg_fill;
    let dark_mode = visuals.dark_mode;

    let white = |k: u8| {
        if sounding[k as usize] {
            accent
        } else if mapped[k as usize] {
            if dark_mode { Color32::from_rgb(235, 240, 250) } else { Color32::WHITE }
        } else {
            Color32::from_gray(150)
        }
    };
    let black = |k: u8| {
        if sounding[k as usize] {
            accent
        } else if mapped[k as usize] {
            Color32::from_gray(20)
        } else {
            Color32::from_gray(80)
        }
    };

    for k in (0..128u8).filter(|&k| !is_black_key(k)) {
        let r = layout.key_rect(k);
        painter.rect_filled(r, CornerRadius::same(2), white(k));
        painter.rect_stroke(r, CornerRadius::same(2), Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
        if k % 12 == 0 {
            painter.text(
                pos2(r.center().x, r.bottom() - 4.0),
                egui::Align2::CENTER_BOTTOM,
                note_name(k),
                egui::FontId::proportional((layout.white_w * 0.55).clamp(6.0, 11.0)),
                Color32::from_gray(40),
            );
        }
    }
    for k in (0..128u8).filter(|&k| is_black_key(k)) {
        painter.rect_filled(layout.key_rect(k), CornerRadius::same(2), black(k));
    }

    let mut out = PianoOutput { note_on: None, note_off: None };
    let hovered_key = response.hover_pos().and_then(|p| layout.hit(p));
    if response.is_pointer_button_down_on() {
        let key = response.interact_pointer_pos().and_then(|p| layout.hit(p));
        if key != *mouse_note {
            out.note_off = *mouse_note;
            out.note_on = key;
            *mouse_note = key;
        }
    } else if let Some(k) = mouse_note.take() {
        out.note_off = Some(k);
    }
    if let Some(k) = hovered_key {
        response.on_hover_text_at_pointer(format!("{} ({k})", note_name(k)));
    }
    out
}
