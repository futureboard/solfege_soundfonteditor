//! Generator & modulator editors for a single zone.

use egui::{CollapsingHeader, ComboBox, DragValue, RichText, Ui};

use super::Edit;
use super::zones::key_drag;
use crate::sf2::generators::{self as sfgen, GENERATORS, GenKind, GenUnit};
use crate::sf2::{Modulator, Zone};

/// `preset_level`: generators are relative offsets and inst-only ones are hidden.
/// `global`: the parent's global zone, used to show inherited values.
pub fn generator_table(ui: &mut Ui, id: &str, zone: &mut Zone, global: Option<&Zone>, preset_level: bool, edit: &mut Edit) {
    let mut groups: Vec<&str> = Vec::new();
    for g in GENERATORS {
        if !groups.contains(&g.group) {
            groups.push(g.group);
        }
    }

    for group in groups {
        let items: Vec<_> = GENERATORS
            .iter()
            .filter(|g| g.group == group && !(preset_level && g.inst_only))
            .collect();
        if items.is_empty() {
            continue;
        }
        let set_count = items.iter().filter(|g| zone.get(g.id).is_some()).count();
        let title = if set_count > 0 { format!("{group}  ({set_count})") } else { group.to_string() };
        let open = matches!(group, "Range" | "Pitch & Level" | "Volume Envelope" | "Sample");
        CollapsingHeader::new(RichText::new(title).strong())
            .id_salt((id, group))
            .default_open(open)
            .show(ui, |ui| {
                egui::Grid::new((id, group, "grid")).num_columns(3).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                    for g in items {
                        row(ui, zone, global, g, preset_level, edit);
                        ui.end_row();
                    }
                });
            });
    }
}

fn row(ui: &mut Ui, zone: &mut Zone, global: Option<&Zone>, g: &sfgen::GenInfo, preset_level: bool, edit: &mut Edit) {
    let mut enabled = zone.get(g.id).is_some();
    let inherited = global.filter(|z| z.get(g.id).is_some());
    if ui.checkbox(&mut enabled, g.name).on_hover_text(format!("Generator #{}", g.id)).changed() {
        if enabled {
            // Start from the inherited value (if any), else the neutral value.
            let raw = match inherited.and_then(|z| z.get(g.id)) {
                Some(v) => v,
                None if g.kind == GenKind::Range => 0x7F00,
                None if preset_level => 0,
                None => g.default as i16 as u16,
            };
            zone.set(g.id, raw);
        } else {
            zone.remove(g.id);
        }
        edit.changed = true;
    }

    if !enabled {
        let text = match inherited {
            Some(z) => format!("global: {}", display(g, z.get_i(g.id).unwrap_or(0), z.get(g.id).unwrap_or(0))),
            None if preset_level => "—".to_string(),
            None => format!("default: {}", display(g, g.default, g.default as u16)),
        };
        ui.label(RichText::new(text).weak());
        ui.label("");
        return;
    }

    match g.kind {
        GenKind::Range => {
            let (mut lo, mut hi) = zone.range(g.id).unwrap_or((0, 127));
            let keys = g.unit == GenUnit::Keys;
            let changed = ui
                .horizontal(|ui| {
                    let a = if keys { ui.add(key_drag(&mut lo)) } else { ui.add(DragValue::new(&mut lo).range(0..=127)) };
                    let b = if keys { ui.add(key_drag(&mut hi)) } else { ui.add(DragValue::new(&mut hi).range(0..=127)) };
                    if a.changed() {
                        hi = hi.max(lo);
                    }
                    if b.changed() {
                        lo = lo.min(hi);
                    }
                    a.changed() || b.changed()
                })
                .inner;
            if changed {
                zone.set_range(g.id, lo, hi);
                edit.changed = true;
            }
            ui.label("");
        }
        _ if g.unit == GenUnit::Mode => {
            let mut v = zone.get_i(g.id).unwrap_or(0);
            ComboBox::from_id_salt(("mode", g.id)).selected_text(sfgen::describe(GenUnit::Mode, v)).show_ui(ui, |ui| {
                for m in [0, 1, 3] {
                    if ui.selectable_value(&mut v, m, sfgen::describe(GenUnit::Mode, m)).changed() {
                        zone.set_i(g.id, v);
                        edit.changed = true;
                    }
                }
            });
            ui.label("");
        }
        _ => {
            let mut v = zone.get_i(g.id).unwrap_or(0);
            let (min, max) = if preset_level {
                let span = g.max - g.min;
                (-span, span)
            } else {
                (g.min, g.max)
            };
            let speed = ((max - min) as f64 / 400.0).max(0.1);
            let r = ui.add(DragValue::new(&mut v).range(min..=max).speed(speed));
            if r.changed() {
                zone.set_i(g.id, v);
                edit.changed = true;
            }
            let desc = if preset_level && matches!(g.unit, GenUnit::Timecents) {
                format!("× {:.3}", sfgen::timecents_to_secs(v as f64))
            } else if preset_level && matches!(g.unit, GenUnit::AbsCents) {
                format!("{v:+} cents")
            } else {
                sfgen::describe(g.unit, v)
            };
            ui.label(RichText::new(desc).weak());
        }
    }
}

fn display(g: &sfgen::GenInfo, v: i32, raw: u16) -> String {
    match g.kind {
        GenKind::Range => format!("{}–{}", raw & 0xFF, raw >> 8),
        _ => {
            let d = sfgen::describe(g.unit, v);
            if d.is_empty() { v.to_string() } else { format!("{v} ({d})") }
        }
    }
}

pub fn modulator_table(ui: &mut Ui, id: &str, mods: &mut Vec<Modulator>, edit: &mut Edit) {
    CollapsingHeader::new(RichText::new(format!("Modulators  ({})", mods.len())).strong())
        .id_salt((id, "mods"))
        .default_open(false)
        .show(ui, |ui| {
            ui.label(RichText::new("Raw SF2 modulator records (source / destination generator / amount / amount source / transform).").weak());
            let mut delete = None;
            egui::Grid::new((id, "mod_grid")).num_columns(6).striped(true).show(ui, |ui| {
                for h in ["Source", "Destination", "Amount", "Amt source", "Transform", ""] {
                    ui.strong(h);
                }
                ui.end_row();
                for (i, m) in mods.iter_mut().enumerate() {
                    edit.track(&ui.add(DragValue::new(&mut m.src).hexadecimal(4, false, true)));
                    ui.horizontal(|ui| {
                        edit.track(&ui.add(DragValue::new(&mut m.dest).range(0..=60)));
                        ui.label(RichText::new(sfgen::name(m.dest)).weak());
                    });
                    edit.track(&ui.add(DragValue::new(&mut m.amount)));
                    edit.track(&ui.add(DragValue::new(&mut m.amt_src).hexadecimal(4, false, true)));
                    edit.track(&ui.add(DragValue::new(&mut m.transform).range(0..=2)));
                    if ui.small_button("🗑").clicked() {
                        delete = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = delete {
                mods.remove(i);
                edit.structural();
            }
            if ui.button("➕ Add modulator").clicked() {
                // Default: CC1 (mod wheel) → vibrato LFO pitch depth, a common choice.
                mods.push(Modulator { src: 0x0081, dest: 6, amount: 50, amt_src: 0, transform: 0 });
                edit.structural();
            }
        });
}
