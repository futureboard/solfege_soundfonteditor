//! Zoomable waveform view with draggable loop markers.

use egui::{Color32, CursorIcon, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    LoopStart,
    LoopEnd,
    Pan,
}

#[derive(Default)]
pub struct WaveView {
    /// First visible sample and number of visible samples.
    start: f64,
    len: f64,
    drag: Option<Drag>,
    data_len: usize,
}

const LOOP_START: Color32 = Color32::from_rgb(80, 200, 120);
const LOOP_END: Color32 = Color32::from_rgb(230, 90, 80);

impl WaveView {
    pub fn zoom_all(&mut self, len: usize) {
        self.start = 0.0;
        self.len = len.max(1) as f64;
        self.data_len = len;
    }

    pub fn zoom_to(&mut self, a: u32, b: u32) {
        let (a, b) = (a.min(b) as f64, a.max(b) as f64);
        let pad = ((b - a) * 0.1).max(32.0);
        self.start = (a - pad).max(0.0);
        self.len = (b - a + pad * 2.0).max(16.0);
        self.clamp();
    }

    fn clamp(&mut self) {
        let n = self.data_len.max(1) as f64;
        self.len = self.len.clamp(8.0_f64.min(n), n);
        self.start = self.start.clamp(0.0, (n - self.len).max(0.0));
    }

    /// Returns true when the loop points were changed.
    pub fn show(&mut self, ui: &mut Ui, height: f32, data: &[i16], loop_start: &mut u32, loop_end: &mut u32, playheads: &[f64]) -> bool {
        if self.data_len != data.len() || self.len <= 0.0 {
            self.zoom_all(data.len());
        }
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        let bg = if visuals.dark_mode { Color32::from_gray(18) } else { Color32::from_gray(245) };
        let wave_color = visuals.selection.bg_fill;
        painter.rect_filled(rect, 4.0, bg);
        let mid = rect.center().y;
        painter.hline(rect.x_range(), mid, Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.4)));

        if data.is_empty() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "No sample data", egui::FontId::proportional(14.0), visuals.weak_text_color());
            return false;
        }

        let spp = self.len / rect.width() as f64; // samples per pixel
        let view_start = self.start;
        let x_of = |s: f64| rect.left() + ((s - view_start) / spp) as f32;
        let s_of = |x: f32| view_start + (x - rect.left()) as f64 * spp;
        let y_of = |v: f32| mid - v / 32768.0 * rect.height() * 0.48;

        // Loop region shading.
        if *loop_end > *loop_start {
            let a = x_of(*loop_start as f64).max(rect.left());
            let b = x_of(*loop_end as f64).min(rect.right());
            if b > a {
                painter.rect_filled(Rect::from_x_y_ranges(a..=b, rect.y_range()), 0.0, LOOP_START.gamma_multiply(0.08));
            }
        }

        // Waveform: min/max columns when zoomed out, a polyline when zoomed in.
        let n = data.len();
        if spp > 1.0 {
            let mut shapes = Vec::with_capacity(rect.width() as usize);
            for px in 0..rect.width() as usize {
                let a = (self.start + px as f64 * spp) as usize;
                let b = ((self.start + (px + 1) as f64 * spp) as usize).min(n);
                if a >= b {
                    continue;
                }
                let stride = ((b - a) / 512).max(1);
                let (mut lo, mut hi) = (i16::MAX, i16::MIN);
                for &v in data[a..b].iter().step_by(stride) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                let x = rect.left() + px as f32 + 0.5;
                shapes.push(Shape::line_segment(
                    [pos2(x, y_of(hi as f32)), pos2(x, y_of(lo as f32) + 0.5)],
                    Stroke::new(1.0, wave_color),
                ));
            }
            painter.extend(shapes);
        } else {
            let a = self.start.floor() as usize;
            let b = ((self.start + self.len).ceil() as usize + 1).min(n);
            let pts: Vec<Pos2> = (a..b).map(|i| pos2(x_of(i as f64), y_of(data[i] as f32))).collect();
            if spp < 0.15 {
                for p in &pts {
                    painter.circle_filled(*p, 2.0, wave_color);
                }
            }
            painter.add(Shape::line(pts, Stroke::new(1.2, wave_color)));
        }

        // Markers.
        let marker = |pos: u32, color: Color32, label: &str| {
            let x = x_of(pos as f64);
            if rect.x_range().contains(x) {
                painter.vline(x, rect.y_range(), Stroke::new(1.5, color));
                painter.text(pos2(x + 3.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::monospace(11.0), color);
            }
        };
        marker(*loop_start, LOOP_START, "LS");
        marker(*loop_end, LOOP_END, "LE");
        for &p in playheads {
            let x = x_of(p);
            if rect.x_range().contains(x) {
                painter.vline(x, rect.y_range(), Stroke::new(1.0, visuals.strong_text_color()));
            }
        }
        painter.text(
            rect.left_bottom() + vec2(4.0, -2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{:.0} – {:.0}", self.start, self.start + self.len),
            egui::FontId::monospace(10.0),
            visuals.weak_text_color(),
        );

        // Interaction.
        let mut changed = false;
        let near = |x: f32, pos: u32| (x - x_of(pos as f64)).abs() < 6.0;
        if let Some(p) = response.hover_pos() {
            if near(p.x, *loop_start) || near(p.x, *loop_end) {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            let zoom_delta = ui.input(|i| i.zoom_delta());
            let factor = if zoom_delta != 1.0 {
                1.0 / zoom_delta as f64
            } else if scroll.y != 0.0 {
                (-scroll.y as f64 * 0.004).exp()
            } else {
                1.0
            };
            if factor != 1.0 {
                let anchor = s_of(p.x);
                let t = (anchor - self.start) / self.len;
                self.len *= factor;
                self.clamp();
                self.start = anchor - t * self.len;
                self.clamp();
            }
            if scroll.x != 0.0 {
                self.start -= scroll.x as f64 * spp;
                self.clamp();
            }
        }
        if response.drag_started()
            && let Some(p) = response.interact_pointer_pos()
        {
            self.drag = Some(if near(p.x, *loop_end) {
                Drag::LoopEnd
            } else if near(p.x, *loop_start) {
                Drag::LoopStart
            } else {
                Drag::Pan
            });
        }
        if response.dragged() {
            match self.drag {
                Some(Drag::Pan) => {
                    self.start -= response.drag_delta().x as f64 * spp;
                    self.clamp();
                }
                Some(d) => {
                    if let Some(p) = response.interact_pointer_pos() {
                        let s = s_of(p.x).round().clamp(0.0, n as f64) as u32;
                        if d == Drag::LoopStart {
                            *loop_start = s.min(*loop_end);
                        } else {
                            *loop_end = s.max(*loop_start);
                        }
                        changed = true;
                    }
                }
                None => {}
            }
        }
        if response.drag_stopped() {
            self.drag = None;
        }
        if response.double_clicked() {
            self.zoom_all(n);
        }
        response.on_hover_text_at_pointer("Scroll: zoom · Drag: pan · Drag LS/LE: move loop · Double-click: show all");
        changed
    }
}

/// Nearest rising zero crossing to `pos` within a small window.
pub fn nearest_zero_crossing(data: &[i16], pos: u32) -> u32 {
    let pos = pos as usize;
    let window = 4096;
    let lo = pos.saturating_sub(window).max(1);
    let hi = (pos + window).min(data.len());
    (lo..hi)
        .filter(|&i| data[i - 1] < 0 && data[i] >= 0)
        .min_by_key(|&i| i.abs_diff(pos))
        .unwrap_or(pos) as u32
}
