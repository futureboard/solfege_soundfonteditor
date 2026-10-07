//! Zone list + key-range map shared by the preset and instrument editors.

use egui::{Color32, ComboBox, DragValue, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::{Edit, is_black_key};
use crate::sf2::Zone;
use crate::sf2::generators::{self as sfgen, note_name};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ZoneSel {
    #[default]
    None,
    Global,
    Zone(usize),
}

pub fn key_drag(value: &mut u8) -> DragValue<'_> {
    DragValue::new(value)
        .range(0..=127)
        .speed(0.25)
        .custom_formatter(|v, _| format!("{} ({})", note_name(v as u8), v as u8))
        .custom_parser(parse_key)
}

/// Narrow variant for table cells: shows only the note name (number on hover).
pub fn key_drag_compact(value: &mut u8) -> DragValue<'_> {
    DragValue::new(value)
        .range(0..=127)
        .speed(0.25)
        .custom_formatter(|v, _| note_name(v as u8))
        .custom_parser(parse_key)
}

/// Accepts either a MIDI number ("60") or a note name ("C4", "f#3").
fn parse_key(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }
    let s = s.split_whitespace().next()?.to_ascii_uppercase();
    let mut chars = s.chars();
    let base = match chars.next()? {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (acc, oct) = if let Some(r) = rest.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('B') {
        (-1, r)
    } else {
        (0, rest.as_str())
    };
    let oct: i32 = oct.parse().ok()?;
    Some(((oct + 1) * 12 + base + acc).clamp(0, 127) as f64)
}

fn link_combo(ui: &mut Ui, id: egui::Id, link: &mut Option<usize>, names: &[String]) -> bool {
    let text = link.and_then(|l| names.get(l)).map(String::as_str).unwrap_or("—");
    let mut changed = false;
    ComboBox::from_id_salt(id).selected_text(text).width(150.0).height(400.0).show_ui(ui, |ui| {
        for (i, n) in names.iter().enumerate() {
            if ui.selectable_label(*link == Some(i), format!("{i:>3}  {n}")).clicked() {
                *link = Some(i);
                changed = true;
            }
        }
    });
    changed
}

fn range_edit(ui: &mut Ui, zone: &mut Zone, id: u16, keys: bool, edit: &mut Edit) {
    let (mut lo, mut hi) = zone.range(id).unwrap_or((0, 127));
    let (a, b) = if keys {
        {
            let a = ui.add(key_drag_compact(&mut lo)).on_hover_text(format!("Key {lo}"));
            ui.label(RichText::new("–").weak());
            let b = ui.add(key_drag_compact(&mut hi)).on_hover_text(format!("Key {hi}"));
            (a, b)
        }
    } else {
        {
            let a = ui.add(DragValue::new(&mut lo).range(0..=127));
            ui.label(RichText::new("–").weak());
            (a, ui.add(DragValue::new(&mut hi).range(0..=127)))
        }
    };
    if a.changed() || b.changed() {
        if a.changed() {
            hi = hi.max(lo);
        } else {
            lo = lo.min(hi);
        }
        if (lo, hi) == (0, 127) {
            zone.remove(id);
        } else {
            zone.set_range(id, lo, hi);
        }
        edit.changed = true;
    }
}

/// Header and cell text for an optional extra column.
pub type ExtraColumn<'a> = (&'a str, &'a dyn Fn(&Zone) -> String);

pub struct ZoneTableResult {
    pub navigate: Option<usize>,
}

/// `kind` is "Instrument" or "Sample". `extra` renders an optional extra column.
#[allow(clippy::too_many_arguments)]
pub fn zone_table(
    ui: &mut Ui,
    id_salt: &str,
    global: &mut Option<Zone>,
    zones: &mut Vec<Zone>,
    names: &[String],
    kind: &str,
    sel: &mut ZoneSel,
    edit: &mut Edit,
    extra: Option<ExtraColumn<'_>>,
) -> ZoneTableResult {
    let mut result = ZoneTableResult { navigate: None };
    let mut delete = None;

    ui.horizontal(|ui| {
        if ui.button("➕ Add zone").clicked() {
            let link = if names.is_empty() { None } else { Some(0) };
            zones.push(Zone { link, ..Default::default() });
            *sel = ZoneSel::Zone(zones.len() - 1);
            edit.structural();
        }
        let has_global = global.is_some();
        if ui.selectable_label(has_global, "Global zone").on_hover_text("Default generators shared by every zone").clicked() {
            if has_global {
                *global = None;
                if *sel == ZoneSel::Global {
                    *sel = ZoneSel::None;
                }
            } else {
                *global = Some(Zone::default());
                *sel = ZoneSel::Global;
            }
            edit.structural();
        }
        if ui.button("Sort by key").clicked() {
            zones.sort_by_key(|z| (z.key_range(), z.vel_range()));
            *sel = ZoneSel::None;
            edit.structural();
        }
    });
    ui.add_space(6.0);

    egui::Grid::new(id_salt).striped(true).num_columns(7).spacing([12.0, 6.0]).min_row_height(26.0).show(ui, |ui| {
        ui.strong("Zone");
        ui.strong(kind);
        ui.strong("Key range");
        ui.strong("Velocity");
        ui.strong(extra.map(|e| e.0).unwrap_or(""));
        ui.label("");
        ui.end_row();

        if global.is_some() {
            if ui.selectable_label(*sel == ZoneSel::Global, RichText::new("Global").italics()).clicked() {
                *sel = ZoneSel::Global;
            }
            ui.label(RichText::new(format!("{} generators", global.as_ref().map_or(0, |g| g.gens.len()))).weak());
            ui.end_row();
        }

        for (i, z) in zones.iter_mut().enumerate() {
            if ui.selectable_label(*sel == ZoneSel::Zone(i), format!("#{}", i + 1)).clicked() {
                *sel = ZoneSel::Zone(i);
            }
            if link_combo(ui, egui::Id::new((id_salt, i)), &mut z.link, names) {
                edit.changed = true;
                *sel = ZoneSel::Zone(i);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                range_edit(ui, z, sfgen::KEY_RANGE, true, edit)
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                range_edit(ui, z, sfgen::VEL_RANGE, false, edit)
            });
            match extra {
                Some((_, f)) => ui.label(f(z)),
                None => ui.label(""),
            };
            ui.horizontal(|ui| {
                if ui.small_button("➡").on_hover_text(format!("Go to {}", kind.to_lowercase())).clicked() {
                    result.navigate = z.link;
                }
                if ui.small_button("🗑").on_hover_text("Delete zone").clicked() {
                    delete = Some(i);
                }
            });
            ui.end_row();
        }
    });

    if let Some(i) = delete {
        zones.remove(i);
        *sel = match *sel {
            ZoneSel::Zone(s) if s == i => ZoneSel::None,
            ZoneSel::Zone(s) if s > i => ZoneSel::Zone(s - 1),
            s => s,
        };
        edit.structural();
    }
    if zones.is_empty() && global.is_none() {
        ui.label(RichText::new(format!("No zones yet — add one and choose a {}.", kind.to_lowercase())).weak());
    }
    result
}

/// Assigns each zone to a lane so that zones whose key ranges don't overlap share a row
/// (a drum kit becomes a single row instead of a staircase).
fn pack_lanes(zones: &[Zone]) -> (Vec<usize>, usize) {
    let mut lanes: Vec<Vec<(u8, u8)>> = Vec::new();
    let mut order: Vec<usize> = (0..zones.len()).collect();
    order.sort_by_key(|&i| zones[i].key_range());
    let mut lane_of = vec![0; zones.len()];
    for i in order {
        let (lo, hi) = zones[i].key_range();
        let free = lanes.iter().position(|l| l.iter().all(|&(a, b)| hi < a || lo > b));
        let lane = free.unwrap_or_else(|| {
            lanes.push(Vec::new());
            lanes.len() - 1
        });
        lanes[lane].push((lo, hi));
        lane_of[i] = lane;
    }
    (lane_of, lanes.len().max(1))
}

/// Horizontal key map: zones laid out along the 128 keys above a piano strip.
/// Scrolls sideways when the keys would be too narrow. Returns a clicked zone.
pub fn zone_map(ui: &mut Ui, zones: &[Zone], labels: &[String], sel: ZoneSel, sounding: &[bool; 128]) -> Option<usize> {
    const MIN_KEY_W: f32 = 14.0;
    const HEADER: f32 = 14.0;
    const PIANO_H: f32 = 40.0;
    let (lane_of, lanes) = pack_lanes(zones);
    let row_h = if lanes > 8 { 18.0 } else { 26.0 };
    let height = HEADER + row_h * lanes as f32 + 4.0 + PIANO_H;
    let kw = (ui.available_width() / 128.0).max(MIN_KEY_W);

    // Scroll the selected zone into view when the selection changes.
    let sel_id = ui.id().with("zone_map_last_sel");
    let changed_sel = ui.data(|d| d.get_temp::<ZoneSel>(sel_id)) != Some(sel);
    ui.data_mut(|d| d.insert_temp(sel_id, sel));

    egui::ScrollArea::horizontal()
        .id_salt("zone_map_scroll")
        .max_height(height + 14.0)
        .show(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(vec2(kw * 128.0, height), Sense::click());
            let painter = ui.painter_at(rect);
            let v = ui.visuals();
            let x = |k: f32| rect.left() + k * kw;
            let lanes_rect = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - PIANO_H));
            painter.rect_filled(lanes_rect, 4.0, v.extreme_bg_color);

            for k in 0..128u8 {
                let col = Rect::from_x_y_ranges(x(k as f32)..=x(k as f32 + 1.0), rect.top() + HEADER..=lanes_rect.bottom());
                if is_black_key(k) {
                    painter.rect_filled(col, 0.0, v.faint_bg_color);
                }
                if sounding[k as usize] {
                    painter.rect_filled(col, 0.0, v.selection.bg_fill.gamma_multiply(0.35));
                }
                if k % 12 == 0 {
                    painter.vline(x(k as f32), lanes_rect.y_range(), Stroke::new(1.0, v.weak_text_color().gamma_multiply(0.3)));
                    painter.text(pos2(x(k as f32) + 2.0, rect.top()), egui::Align2::LEFT_TOP, note_name(k), egui::FontId::proportional(10.0), v.weak_text_color());
                }
            }

            // Piano strip under the lanes, one column per semitone so it lines up with the bars:
            // a white bed, short black keys on top, and white-key seams between them.
            let piano = Rect::from_x_y_ranges(rect.x_range(), lanes_rect.bottom() + 4.0..=rect.bottom());
            let black_bottom = piano.top() + piano.height() * 0.6;
            let seam = Stroke::new(1.0, Color32::from_gray(150));
            painter.rect_filled(piano, 3.0, Color32::from_gray(238));
            for k in 0..128u8 {
                let col = Rect::from_x_y_ranges(x(k as f32)..=x(k as f32 + 1.0), piano.y_range());
                if is_black_key(k) {
                    let top = Rect::from_x_y_ranges(col.x_range(), piano.top()..=black_bottom);
                    let fill = if sounding[k as usize] { v.selection.bg_fill } else { Color32::from_gray(35) };
                    painter.rect_filled(top, 1.0, fill);
                    // The white-key seam continues below the black key.
                    painter.vline(col.center().x, black_bottom..=piano.bottom(), seam);
                } else {
                    if sounding[k as usize] {
                        painter.rect_filled(col, 0.0, v.selection.bg_fill.gamma_multiply(0.6));
                    }
                    // E–F and B–C have no black key between them: full-height seam.
                    if matches!(k % 12, 4 | 11) {
                        painter.vline(col.right(), piano.y_range(), seam);
                    }
                }
            }

            let mut hovered = None;
            let mut selected_rect = None;
            for (i, z) in zones.iter().enumerate() {
                let (lo, hi) = z.key_range();
                let (vlo, vhi) = z.vel_range();
                let top = rect.top() + HEADER + lane_of[i] as f32 * row_h;
                let bar = Rect::from_min_max(pos2(x(lo as f32) + 1.0, top + 1.0), pos2(x(hi as f32 + 1.0) - 1.0, top + row_h - 1.0));
                let hue = (i as f32 * 0.61803) % 1.0;
                let mut color: Color32 = egui::ecolor::Hsva::new(hue, 0.55, 0.85, 1.0).into();
                if (vlo, vhi) != (0, 127) {
                    color = color.gamma_multiply(0.4 + 0.6 * (vhi as f32 / 127.0));
                }
                painter.rect_filled(bar, 3.0, color);
                if sel == ZoneSel::Zone(i) {
                    painter.rect_stroke(bar.expand(1.0), 3.0, Stroke::new(2.0, v.strong_text_color()), egui::StrokeKind::Outside);
                    selected_rect = Some(bar);
                }
                let label = labels.get(i).map(String::as_str).unwrap_or("");
                let font = egui::FontId::proportional((row_h - 8.0).max(9.0));
                let galley = painter.layout_no_wrap(label.to_string(), font, Color32::BLACK);
                // Keep the label inside the visible part of the bar when scrolled.
                let visible = bar.intersect(ui.clip_rect());
                if galley.size().x + 6.0 <= visible.width() {
                    painter.galley(pos2(visible.left() + 3.0, bar.center().y - galley.size().y / 2.0), galley, Color32::BLACK);
                }
                if response.hover_pos().is_some_and(|p| bar.contains(p)) {
                    hovered = Some(i);
                }
            }

            if changed_sel && let Some(r) = selected_rect {
                ui.scroll_to_rect(r, Some(egui::Align::Center));
            }

            let clicked = if response.clicked() { hovered } else { None };
            if let Some(i) = hovered {
                let z = &zones[i];
                let (lo, hi) = z.key_range();
                let (vlo, vhi) = z.vel_range();
                let keys = if lo == hi { note_name(lo) } else { format!("{}–{}", note_name(lo), note_name(hi)) };
                response.on_hover_text_at_pointer(format!(
                    "#{} {}\nKeys {keys}  Vel {vlo}–{vhi}",
                    i + 1,
                    labels.get(i).map(String::as_str).unwrap_or(""),
                ));
            }
            clicked
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(lo: u8, hi: u8) -> Zone {
        let mut z = Zone::default();
        z.set_range(sfgen::KEY_RANGE, lo, hi);
        z
    }

    #[test]
    fn drum_kit_packs_into_one_lane() {
        let zones: Vec<Zone> = (35..=60).map(|k| zone(k, k)).collect();
        let (_, lanes) = pack_lanes(&zones);
        assert_eq!(lanes, 1);
    }

    #[test]
    fn overlapping_zones_get_separate_lanes() {
        // Two velocity layers over the same keys, plus one zone beside them.
        let zones = vec![zone(0, 60), zone(0, 60), zone(61, 127)];
        let (lane_of, lanes) = pack_lanes(&zones);
        assert_eq!(lanes, 2);
        assert_ne!(lane_of[0], lane_of[1]);
        assert_eq!(lane_of[2], 0);
    }
}
