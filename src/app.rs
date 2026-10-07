use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{
    Align, Color32, ComboBox, DragValue, Key, KeyboardShortcut, Layout, Modifiers, RichText, ScrollArea, TextEdit, Ui,
};

use crate::audio::{self, Audio};
use crate::sf2::generators::{self as sfgen, note_name};
use crate::sf2::{self, Instrument, Preset, SoundFont, Zone, sample_type};
use crate::ui::gens::{generator_table, modulator_table};
use crate::ui::keyboard::piano;
use crate::ui::waveform::{WaveView, nearest_zero_crossing};
use crate::ui::zones::{ZoneSel, key_drag, zone_map, zone_table};
use crate::ui::Edit;
use crate::wav;

pub const APP_NAME: &str = "Solfege SoundFont Editor";
const MAX_UNDO: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sel {
    Info,
    Preset(usize),
    Instrument(usize),
    Sample(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Presets,
    Instruments,
    Samples,
}

/// Actions that must wait for the user to confirm discarding unsaved changes.
#[derive(Clone)]
enum Pending {
    New,
    Open(Option<PathBuf>),
    Quit,
}

/// PC keyboard → semitone offset (two rows, like most trackers/DAWs).
const PC_KEYS: &[(Key, u8)] = &[
    (Key::Z, 0), (Key::S, 1), (Key::X, 2), (Key::D, 3), (Key::C, 4), (Key::V, 5), (Key::G, 6),
    (Key::B, 7), (Key::H, 8), (Key::N, 9), (Key::J, 10), (Key::M, 11), (Key::Comma, 12),
    (Key::Q, 12), (Key::Num2, 13), (Key::W, 14), (Key::Num3, 15), (Key::E, 16), (Key::R, 17),
    (Key::Num5, 18), (Key::T, 19), (Key::Num6, 20), (Key::Y, 21), (Key::Num7, 22), (Key::U, 23),
    (Key::I, 24), (Key::Num9, 25), (Key::O, 26), (Key::Num0, 27), (Key::P, 28),
];

pub struct App {
    sf: SoundFont,
    path: Option<PathBuf>,
    dirty: bool,
    sel: Sel,
    zone_sel: ZoneSel,
    tab: Tab,
    filter: String,

    undo: Vec<SoundFont>,
    redo: Vec<SoundFont>,
    txn_open: bool,
    edit: Edit,

    audio: Audio,
    octave: i32,
    velocity: u8,
    mouse_note: Option<u8>,
    /// PC key → MIDI note it started, so releases match even if the octave changed.
    pc_held: Vec<(Key, u8)>,
    sounding: [u8; 128],
    loop_preview: bool,

    wave: WaveView,
    wave_for: Option<usize>,
    status: String,
    pending: Option<Pending>,
    error: Option<String>,
    show_problems: bool,
    title: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        cc.egui_ctx.global_style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 5.0);
        });
        install_fallback_fonts(&cc.egui_ctx);
        let audio = Audio::new();
        let status = match &audio.error {
            Some(e) => format!("Audio unavailable: {e}"),
            None => format!("Audio: {} @ {} Hz", audio.device_name, audio.sample_rate),
        };
        let mut app = Self {
            sf: SoundFont::new_empty(),
            path: None,
            dirty: false,
            sel: Sel::Info,
            zone_sel: ZoneSel::None,
            tab: Tab::Presets,
            filter: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            txn_open: false,
            edit: Edit::default(),
            audio,
            octave: 4,
            velocity: 100,
            mouse_note: None,
            pc_held: Vec::new(),
            sounding: [0; 128],
            loop_preview: true,
            wave: WaveView::default(),
            wave_for: None,
            status,
            pending: None,
            error: None,
            show_problems: false,
            title: String::new(),
        };
        if let Some(p) = path {
            app.open_path(&p);
        }
        app
    }

    // ---------------------------------------------------------------- files

    fn open_path(&mut self, path: &Path) {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if ext.as_deref() == Some("wav") {
            self.import_wav(path);
            return;
        }
        match std::fs::read(path).map_err(anyhow::Error::from).and_then(|b| sf2::read_sf2(&b)) {
            Ok(sf) => {
                self.audio.all_off();
                self.status = format!(
                    "Opened {} — {} presets, {} instruments, {} samples",
                    path.display(),
                    sf.presets.len(),
                    sf.instruments.len(),
                    sf.samples.len()
                );
                self.sf = sf;
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.undo.clear();
                self.redo.clear();
                self.txn_open = false;
                self.sel = if self.sf.presets.is_empty() { Sel::Info } else { Sel::Preset(0) };
                self.tab = Tab::Presets;
                self.zone_sel = ZoneSel::None;
            }
            Err(e) => self.error = Some(format!("Could not open {}:\n{e:#}", path.display())),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("SoundFont 2", &["sf2", "SF2"]).add_filter("All files", &["*"]).pick_file() {
            self.open_path(&p);
        }
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(p) => self.save_to(&p),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string());
        let mut d = rfd::FileDialog::new().add_filter("SoundFont 2", &["sf2"]);
        d = d.set_file_name(name.unwrap_or_else(|| format!("{}.sf2", self.sf.info.name)));
        if let Some(mut p) = d.save_file() {
            if p.extension().is_none() {
                p.set_extension("sf2");
            }
            self.save_to(&p);
        }
    }

    fn save_to(&mut self, path: &Path) {
        self.sf.info.software = concat!("Solfege SoundFont Editor ", env!("CARGO_PKG_VERSION")).into();
        let result = sf2::write_sf2(&self.sf).and_then(|bytes| {
            // Write to a temp file first so a failed save never corrupts the original.
            let tmp = path.with_extension("sf2.tmp");
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, path)?;
            Ok(bytes.len())
        });
        match result {
            Ok(n) => {
                self.path = Some(path.to_path_buf());
                self.dirty = false;
                self.status = format!("Saved {} ({:.1} MB)", path.display(), n as f64 / 1_048_576.0);
            }
            Err(e) => self.error = Some(format!("Could not save {}:\n{e:#}", path.display())),
        }
    }

    fn import_wav(&mut self, path: &Path) {
        match wav::import(path) {
            Ok(mut samples) => {
                self.checkpoint();
                let base = self.sf.samples.len();
                if samples.len() == 2 {
                    samples[0].link = Some(base + 1);
                    samples[1].link = Some(base);
                }
                let n = samples.len();
                self.sf.samples.extend(samples);
                self.select(Sel::Sample(base));
                self.status = format!("Imported {} ({} sample{})", path.display(), n, if n == 1 { "" } else { "s" });
            }
            Err(e) => self.error = Some(format!("Could not import {}:\n{e:#}", path.display())),
        }
    }

    fn import_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new().add_filter("WAV audio", &["wav", "WAV"]).pick_files() {
            for f in files {
                self.import_wav(&f);
            }
        }
    }

    fn request(&mut self, action: Pending) {
        if self.dirty {
            self.pending = Some(action);
        } else {
            self.run_pending(action);
        }
    }

    fn run_pending(&mut self, action: Pending) {
        match action {
            Pending::New => {
                self.audio.all_off();
                self.sf = SoundFont::new_empty();
                self.path = None;
                self.dirty = false;
                self.undo.clear();
                self.redo.clear();
                self.sel = Sel::Info;
                self.status = "New SoundFont".into();
            }
            Pending::Open(Some(p)) => self.open_path(&p),
            Pending::Open(None) => self.open_dialog(),
            Pending::Quit => {
                self.dirty = false;
            }
        }
    }

    // ---------------------------------------------------------------- undo

    /// Record an undo step before a structural edit made outside the editors.
    fn checkpoint(&mut self) {
        self.undo.push(self.sf.clone());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.txn_open = false;
        self.dirty = true;
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.sf, prev));
            self.after_history();
            self.status = "Undo".into();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.sf, next));
            self.after_history();
            self.status = "Redo".into();
        }
    }

    fn after_history(&mut self) {
        self.txn_open = false;
        self.dirty = true;
        self.audio.all_off();
        self.fix_selection();
    }

    fn fix_selection(&mut self) {
        self.sel = match self.sel {
            Sel::Preset(i) if i >= self.sf.presets.len() => self.sf.presets.len().checked_sub(1).map_or(Sel::Info, Sel::Preset),
            Sel::Instrument(i) if i >= self.sf.instruments.len() => {
                self.sf.instruments.len().checked_sub(1).map_or(Sel::Info, Sel::Instrument)
            }
            Sel::Sample(i) if i >= self.sf.samples.len() => self.sf.samples.len().checked_sub(1).map_or(Sel::Info, Sel::Sample),
            s => s,
        };
        let zone_count = match self.sel {
            Sel::Preset(i) => self.sf.presets[i].zones.len(),
            Sel::Instrument(i) => self.sf.instruments[i].zones.len(),
            _ => 0,
        };
        let has_global = match self.sel {
            Sel::Preset(i) => self.sf.presets[i].global.is_some(),
            Sel::Instrument(i) => self.sf.instruments[i].global.is_some(),
            _ => false,
        };
        self.zone_sel = match self.zone_sel {
            ZoneSel::Zone(z) if z >= zone_count => ZoneSel::None,
            ZoneSel::Global if !has_global => ZoneSel::None,
            z => z,
        };
    }

    /// Called at the end of each frame to fold editor changes into undo history.
    fn finish_frame(&mut self, ctx: &egui::Context, before: SoundFont) {
        let edit = std::mem::take(&mut self.edit);
        if edit.changed {
            if !self.txn_open || edit.structural {
                self.undo.push(before);
                if self.undo.len() > MAX_UNDO {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            // Group continuous edits (dragging, typing) into a single undo step.
            self.txn_open = !edit.structural;
            self.dirty = true;
            self.fix_selection();
        } else if self.txn_open {
            let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
            if !busy {
                self.txn_open = false;
            }
        }
    }

    // ---------------------------------------------------------------- selection & structure

    fn select(&mut self, sel: Sel) {
        if sel != self.sel {
            self.zone_sel = ZoneSel::None;
        }
        self.sel = sel;
        self.tab = match sel {
            Sel::Preset(_) => Tab::Presets,
            Sel::Instrument(_) => Tab::Instruments,
            Sel::Sample(_) => Tab::Samples,
            Sel::Info => self.tab,
        };
    }

    fn free_program(&self, bank: u16) -> u16 {
        (0..128).find(|p| !self.sf.presets.iter().any(|x| x.bank == bank && x.program == *p)).unwrap_or(0)
    }

    fn add_item(&mut self) {
        match self.tab {
            Tab::Presets => {
                self.checkpoint();
                let program = self.free_program(0);
                self.sf.presets.push(Preset { name: "New Preset".into(), program, ..Default::default() });
                self.select(Sel::Preset(self.sf.presets.len() - 1));
            }
            Tab::Instruments => {
                self.checkpoint();
                self.sf.instruments.push(Instrument { name: "New Instrument".into(), ..Default::default() });
                self.select(Sel::Instrument(self.sf.instruments.len() - 1));
            }
            Tab::Samples => self.import_dialog(),
        }
    }

    fn duplicate_selected(&mut self) {
        let suffix = |n: &str| format!("{} copy", n.chars().take(15).collect::<String>());
        match self.sel {
            Sel::Preset(i) => {
                self.checkpoint();
                let mut p = self.sf.presets[i].clone();
                p.name = suffix(&p.name);
                p.program = self.free_program(p.bank);
                self.sf.presets.push(p);
                self.select(Sel::Preset(self.sf.presets.len() - 1));
            }
            Sel::Instrument(i) => {
                self.checkpoint();
                let mut x = self.sf.instruments[i].clone();
                x.name = suffix(&x.name);
                self.sf.instruments.push(x);
                self.select(Sel::Instrument(self.sf.instruments.len() - 1));
            }
            Sel::Sample(i) => {
                self.checkpoint();
                let mut s = self.sf.samples[i].clone();
                s.name = suffix(&s.name);
                s.link = None;
                s.sample_type = sample_type::MONO;
                self.sf.samples.push(s);
                self.select(Sel::Sample(self.sf.samples.len() - 1));
            }
            Sel::Info => {}
        }
    }

    fn delete_selected(&mut self) {
        self.audio.all_off();
        match self.sel {
            Sel::Preset(i) => {
                self.checkpoint();
                self.sf.presets.remove(i);
            }
            Sel::Instrument(i) => {
                self.checkpoint();
                self.sf.remove_instrument(i);
            }
            Sel::Sample(i) => {
                self.checkpoint();
                self.sf.remove_sample(i);
            }
            Sel::Info => return,
        }
        self.zone_sel = ZoneSel::None;
        self.fix_selection();
    }

    fn delete_unused(&mut self) {
        self.checkpoint();
        let mut removed = (0, 0);
        let mut i = self.sf.instruments.len();
        while i > 0 {
            i -= 1;
            if self.sf.instrument_users(i) == 0 {
                self.sf.remove_instrument(i);
                removed.0 += 1;
            }
        }
        let mut i = self.sf.samples.len();
        while i > 0 {
            i -= 1;
            if self.sf.sample_users(i) == 0 {
                self.sf.remove_sample(i);
                removed.1 += 1;
            }
        }
        self.audio.all_off();
        self.fix_selection();
        self.status = format!("Removed {} unused instruments and {} unused samples", removed.0, removed.1);
    }

    fn instrument_from_sample(&mut self, idx: usize) {
        self.checkpoint();
        let s = &self.sf.samples[idx];
        let mut inst = Instrument { name: s.name.clone(), ..Default::default() };
        let mode = if s.loop_end > s.loop_start + 1 { 1 } else { 0 };
        let mut push = |link: usize, pan: i32| {
            let mut z = Zone { link: Some(link), ..Default::default() };
            if mode != 0 {
                z.set_i(sfgen::SAMPLE_MODES, mode);
            }
            if pan != 0 {
                z.set_i(sfgen::PAN, pan);
            }
            inst.zones.push(z);
        };
        match (s.sample_type & 0x7FFF, s.link) {
            (sample_type::LEFT, Some(r)) => {
                push(idx, -500);
                push(r, 500);
            }
            (sample_type::RIGHT, Some(l)) => {
                push(l, -500);
                push(idx, 500);
            }
            _ => push(idx, 0),
        }
        self.sf.instruments.push(inst);
        self.select(Sel::Instrument(self.sf.instruments.len() - 1));
    }

    fn preset_from_instrument(&mut self, idx: usize) {
        self.checkpoint();
        let program = self.free_program(0);
        let name = self.sf.instruments[idx].name.clone();
        self.sf.presets.push(Preset {
            name,
            program,
            zones: vec![Zone { link: Some(idx), ..Default::default() }],
            ..Default::default()
        });
        self.select(Sel::Preset(self.sf.presets.len() - 1));
    }

    // ---------------------------------------------------------------- audio

    fn voices_for(&self, key: u8, vel: u8) -> Vec<audio::VoiceParams> {
        match self.sel {
            Sel::Preset(i) => audio::preset_voices(&self.sf, i, key, vel),
            Sel::Instrument(i) => audio::instrument_voices(&self.sf, i, key, vel, None),
            Sel::Sample(i) => audio::sample_voice(&self.sf, i, key, vel, self.loop_preview),
            Sel::Info => Vec::new(),
        }
    }

    fn note_on(&mut self, key: u8) {
        let voices = self.voices_for(key, self.velocity);
        self.audio.note_on(key, voices);
        self.sounding[key as usize] = self.sounding[key as usize].saturating_add(1);
    }

    fn note_off(&mut self, key: u8) {
        let n = &mut self.sounding[key as usize];
        *n = n.saturating_sub(1);
        if *n == 0 {
            self.audio.note_off(key);
        }
    }

    fn mapped_keys(&self) -> [bool; 128] {
        let mut m = [false; 128];
        let mut mark = |zones: &[Zone], global: Option<&Zone>| {
            for z in zones {
                let (lo, hi) = z.range(sfgen::KEY_RANGE).or_else(|| global.and_then(|g| g.range(sfgen::KEY_RANGE))).unwrap_or((0, 127));
                for k in lo..=hi.min(127) {
                    m[k as usize] = true;
                }
            }
        };
        match self.sel {
            Sel::Preset(i) => {
                let p = &self.sf.presets[i];
                mark(&p.zones, p.global.as_ref());
            }
            Sel::Instrument(i) => {
                let x = &self.sf.instruments[i];
                mark(&x.zones, x.global.as_ref());
            }
            Sel::Sample(_) => m = [true; 128],
            Sel::Info => {}
        }
        m
    }

    fn sounding_mask(&self) -> [bool; 128] {
        std::array::from_fn(|k| self.sounding[k] > 0)
    }

    // ---------------------------------------------------------------- input

    fn handle_input(&mut self, ctx: &egui::Context) {
        let shortcut = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
        if shortcut(Modifiers::COMMAND, Key::O) {
            self.request(Pending::Open(None));
        }
        if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            self.save_as();
        }
        if shortcut(Modifiers::COMMAND, Key::S) {
            self.save();
        }
        if shortcut(Modifiers::COMMAND, Key::N) {
            self.request(Pending::New);
        }
        if shortcut(Modifiers::COMMAND, Key::I) {
            self.import_dialog();
        }

        let typing = ctx.egui_wants_keyboard_input();
        if !typing {
            if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || shortcut(Modifiers::COMMAND, Key::Y) {
                self.redo();
            }
            if shortcut(Modifiers::COMMAND, Key::Z) {
                self.undo();
            }
            if shortcut(Modifiers::COMMAND, Key::D) {
                self.duplicate_selected();
            }
        }

        // Dropped files: .sf2 opens, .wav imports.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            let is_wav = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav"));
            if is_wav {
                self.import_wav(&p);
            } else {
                self.request(Pending::Open(Some(p)));
            }
        }

        // Computer keyboard as a piano.
        if typing {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for e in events {
            let egui::Event::Key { key, pressed, repeat, modifiers, .. } = e else { continue };
            if modifiers.command || modifiers.alt {
                continue;
            }
            if pressed && !repeat {
                match key {
                    Key::ArrowLeft | Key::Minus => self.octave = (self.octave - 1).max(-1),
                    Key::ArrowRight | Key::Equals => self.octave = (self.octave + 1).min(8),
                    Key::Escape => {
                        self.audio.all_off();
                        self.sounding = [0; 128];
                        self.pc_held.clear();
                    }
                    _ => {}
                }
            }
            let Some(&(_, offset)) = PC_KEYS.iter().find(|(k, _)| *k == key) else { continue };
            if pressed && !repeat && !self.pc_held.iter().any(|(k, _)| *k == key) {
                let note = (self.octave + 1) * 12 + offset as i32;
                if (0..128).contains(&note) {
                    self.pc_held.push((key, note as u8));
                    self.note_on(note as u8);
                }
            } else if !pressed && let Some(pos) = self.pc_held.iter().position(|(k, _)| *k == key) {
                let (_, note) = self.pc_held.remove(pos);
                self.note_off(note);
            }
        }
    }

    // ---------------------------------------------------------------- panels

    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New            Ctrl+N").clicked() {
                    self.request(Pending::New);
                }
                if ui.button("Open…          Ctrl+O").clicked() {
                    self.request(Pending::Open(None));
                }
                if ui.button("Save           Ctrl+S").clicked() {
                    self.save();
                }
                if ui.button("Save As…  Ctrl+Shift+S").clicked() {
                    self.save_as();
                }
                ui.separator();
                if ui.button("Import WAV…    Ctrl+I").clicked() {
                    self.import_dialog();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo   Ctrl+Z")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo   Ctrl+Y")).clicked() {
                    self.redo();
                }
                ui.separator();
                if ui.add_enabled(self.sel != Sel::Info, egui::Button::new("Duplicate   Ctrl+D")).clicked() {
                    self.duplicate_selected();
                }
                if ui.add_enabled(self.sel != Sel::Info, egui::Button::new("Delete")).clicked() {
                    self.delete_selected();
                }
                ui.separator();
                if ui.button("Sort presets by bank/program").clicked() {
                    self.checkpoint();
                    self.sf.sort_presets();
                    self.sel = Sel::Info;
                }
                if ui.button("Remove unused instruments & samples").clicked() {
                    self.delete_unused();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("SoundFont info").clicked() {
                    self.select(Sel::Info);
                }
                if ui.button("Check for problems").clicked() {
                    self.show_problems = true;
                }
                ui.separator();
                egui::widgets::global_theme_preference_buttons(ui);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(APP_NAME).weak());
            });
        });
    }

    fn list_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        if ui.selectable_label(self.sel == Sel::Info, RichText::new(format!("ℹ  {}", self.sf.info.name)).strong()).clicked() {
            self.select(Sel::Info);
        }
        ui.separator();
        ui.horizontal(|ui| {
            for (tab, label, n) in [
                (Tab::Presets, "Presets", self.sf.presets.len()),
                (Tab::Instruments, "Instruments", self.sf.instruments.len()),
                (Tab::Samples, "Samples", self.sf.samples.len()),
            ] {
                ui.selectable_value(&mut self.tab, tab, format!("{label} {n}"));
            }
        });
        ui.add(TextEdit::singleline(&mut self.filter).hint_text("🔍 Filter").desired_width(f32::INFINITY));
        ui.horizontal(|ui| {
            let add_label = if self.tab == Tab::Samples { "➕ Import WAV" } else { "➕ New" };
            if ui.button(add_label).clicked() {
                self.add_item();
            }
            let has_sel = matches!(
                (self.tab, self.sel),
                (Tab::Presets, Sel::Preset(_)) | (Tab::Instruments, Sel::Instrument(_)) | (Tab::Samples, Sel::Sample(_))
            );
            if ui.add_enabled(has_sel, egui::Button::new("Duplicate")).clicked() {
                self.duplicate_selected();
            }
            if ui.add_enabled(has_sel, egui::Button::new("🗑 Delete")).clicked() {
                self.delete_selected();
            }
        });
        ui.separator();

        let filter = self.filter.to_lowercase();
        let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
        let items: Vec<(Sel, String, bool)> = match self.tab {
            Tab::Presets => self
                .sf
                .presets
                .iter()
                .enumerate()
                .filter(|(_, p)| matches(&p.name))
                .map(|(i, p)| (Sel::Preset(i), format!("{:03}:{:03}  {}", p.bank, p.program, p.name), true))
                .collect(),
            Tab::Instruments => self
                .sf
                .instruments
                .iter()
                .enumerate()
                .filter(|(_, x)| matches(&x.name))
                .map(|(i, x)| (Sel::Instrument(i), x.name.clone(), self.sf.instrument_users(i) > 0))
                .collect(),
            Tab::Samples => self
                .sf
                .samples
                .iter()
                .enumerate()
                .filter(|(_, s)| matches(&s.name))
                .map(|(i, s)| (Sel::Sample(i), s.name.clone(), self.sf.sample_users(i) > 0))
                .collect(),
        };

        let row_h = ui.text_style_height(&egui::TextStyle::Button) + 4.0;
        let mut clicked = None;
        ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h, items.len(), |ui, range| {
            for (sel, label, used) in &items[range] {
                let text = if *used { RichText::new(label) } else { RichText::new(label).weak().italics() };
                let r = ui.add_sized([ui.available_width(), row_h], egui::Button::selectable(self.sel == *sel, text));
                let r = if *used { r } else { r.on_hover_text("Not used by any preset/instrument") };
                if r.clicked() {
                    clicked = Some(*sel);
                }
            }
        });
        if let Some(s) = clicked {
            self.select(s);
        }
    }

    fn keyboard_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let target = match self.sel {
                Sel::Preset(i) => format!("Preset: {}", self.sf.presets[i].name),
                Sel::Instrument(i) => format!("Instrument: {}", self.sf.instruments[i].name),
                Sel::Sample(i) => format!("Sample: {}", self.sf.samples[i].name),
                Sel::Info => "Select a preset, instrument or sample to play".into(),
            };
            ui.label(RichText::new("🎹").size(16.0));
            ui.strong(target);
            ui.separator();
            ui.label("Velocity");
            ui.add(DragValue::new(&mut self.velocity).range(1..=127));
            ui.label("Octave");
            ui.add(DragValue::new(&mut self.octave).range(-1..=8)).on_hover_text("PC keys Z–M / Q–P play from this octave (←/→ to change)");
            if matches!(self.sel, Sel::Sample(_)) {
                ui.checkbox(&mut self.loop_preview, "Loop");
            }
            ui.separator();
            if let Ok(mut m) = self.audio.mixer.lock() {
                ui.label("Volume");
                ui.add(egui::Slider::new(&mut m.master, 0.0..=1.5).show_value(false));
                ui.label(RichText::new(format!("{} voices", m.active_voices())).weak().monospace());
            }
            if ui.button("⏹ Stop").on_hover_text("All notes off (Esc)").clicked() {
                self.audio.all_off();
                self.sounding = [0; 128];
            }
        });
        let sounding = self.sounding_mask();
        let mapped = self.mapped_keys();
        let out = piano(ui, 72.0, &sounding, &mapped, &mut self.mouse_note);
        if let Some(k) = out.note_off {
            self.note_off(k);
        }
        if let Some(k) = out.note_on {
            self.note_on(k);
        }
        ui.add_space(2.0);
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.status).small());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let total: usize = self.sf.samples.iter().map(|s| s.data.len() * 2).sum();
                ui.label(RichText::new(format!("{:.1} MB sample data", total as f64 / 1_048_576.0)).small().weak());
                if self.dirty {
                    ui.label(RichText::new("● modified").small().color(Color32::from_rgb(230, 170, 60)));
                }
            });
        });
    }

    // ---------------------------------------------------------------- editors

    fn editor(&mut self, ui: &mut Ui) {
        match self.sel {
            Sel::Info => self.info_editor(ui),
            Sel::Preset(i) => self.preset_editor(ui, i),
            Sel::Instrument(i) => self.instrument_editor(ui, i),
            Sel::Sample(i) => self.sample_editor(ui, i),
        }
    }

    fn info_editor(&mut self, ui: &mut Ui) {
        ui.heading("SoundFont Info");
        ui.add_space(6.0);
        let edit = &mut self.edit;
        let info = &mut self.sf.info;
        egui::Grid::new("info").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            let mut field = |ui: &mut Ui, label: &str, value: &mut String, multiline: bool| {
                ui.label(label);
                let r = if multiline {
                    ui.add(TextEdit::multiline(value).desired_width(480.0).desired_rows(4))
                } else {
                    ui.add(TextEdit::singleline(value).desired_width(480.0).char_limit(255))
                };
                edit.track(&r);
                ui.end_row();
            };
            field(ui, "Name", &mut info.name, false);
            field(ui, "Author / engineers", &mut info.engineers, false);
            field(ui, "Copyright", &mut info.copyright, false);
            field(ui, "Creation date", &mut info.creation_date, false);
            field(ui, "Product", &mut info.product, false);
            field(ui, "Sound engine", &mut info.sound_engine, false);
            field(ui, "Comment", &mut info.comment, true);
            ui.label("Format version");
            ui.label(format!("{}.{:02}", info.version.0, info.version.1));
            ui.end_row();
            ui.label("Software");
            ui.label(RichText::new(&info.software).weak());
            ui.end_row();
            if !info.rom_name.is_empty() {
                ui.label("ROM");
                ui.label(&info.rom_name);
                ui.end_row();
            }
        });
        ui.add_space(12.0);
        ui.separator();
        ui.label(format!(
            "{} presets · {} instruments · {} samples",
            self.sf.presets.len(),
            self.sf.instruments.len(),
            self.sf.samples.len()
        ));
        if let Some(p) = &self.path {
            ui.label(RichText::new(p.display().to_string()).weak());
        }
        ui.add_space(8.0);
        ui.label(RichText::new(
            "Tip: drop a .sf2 file to open it, or .wav files to import samples. \
             Play notes with the on-screen piano or the PC keyboard (Z–M, Q–P).",
        )
        .weak());
    }

    fn preset_editor(&mut self, ui: &mut Ui, idx: usize) {
        let names: Vec<String> = self.sf.instruments.iter().map(|x| x.name.clone()).collect();
        let sounding = self.sounding_mask();
        let navigate;
        let mut make_unique = false;
        {
            let Self { sf, edit, zone_sel, .. } = self;
            let taken: Vec<(u16, u16)> =
                sf.presets.iter().enumerate().filter(|(i, _)| *i != idx).map(|(_, p)| (p.bank, p.program)).collect();
            let p = &mut sf.presets[idx];

            ui.horizontal(|ui| {
                ui.heading("Preset");
                edit.track(&ui.add(TextEdit::singleline(&mut p.name).char_limit(20).desired_width(200.0).font(egui::TextStyle::Heading)));
            });
            ui.horizontal(|ui| {
                ui.label("Bank");
                edit.track(&ui.add(DragValue::new(&mut p.bank).range(0..=128)));
                ui.label("Program");
                edit.track(&ui.add(DragValue::new(&mut p.program).range(0..=127)));
                if taken.contains(&(p.bank, p.program)) {
                    ui.label(RichText::new("⚠ bank/program already used").color(ui.visuals().warn_fg_color));
                    make_unique = ui.small_button("Fix").clicked();
                }
            });
            ui.add_space(6.0);

            let labels: Vec<String> = p.zones.iter().map(|z| z.link.and_then(|l| names.get(l)).cloned().unwrap_or_default()).collect();
            if let Some(i) = zone_map(ui, &p.zones, &labels, *zone_sel, &sounding) {
                *zone_sel = ZoneSel::Zone(i);
            }
            ui.add_space(6.0);
            let res = zone_table(ui, "preset_zones", &mut p.global, &mut p.zones, &names, "Instrument", zone_sel, edit, None);
            navigate = res.navigate;

            ui.add_space(8.0);
            zone_inspector(ui, "pgen", zone_sel, &mut p.global, &mut p.zones, true, &names, edit);
        }
        if make_unique {
            let bank = self.sf.presets[idx].bank;
            let program = self.free_program(bank);
            self.sf.presets[idx].program = program;
            self.edit.structural();
        }
        if let Some(i) = navigate {
            self.select(Sel::Instrument(i));
        }
    }

    fn instrument_editor(&mut self, ui: &mut Ui, idx: usize) {
        let names: Vec<String> = self.sf.samples.iter().map(|s| s.name.clone()).collect();
        let roots: Vec<u8> = self.sf.samples.iter().map(|s| s.original_pitch).collect();
        let sounding = self.sounding_mask();
        let users = self.sf.instrument_users(idx);
        let navigate;
        let mut make_preset = false;
        {
            let Self { sf, edit, zone_sel, .. } = self;
            let inst = &mut sf.instruments[idx];
            ui.horizontal(|ui| {
                ui.heading("Instrument");
                edit.track(&ui.add(TextEdit::singleline(&mut inst.name).char_limit(20).desired_width(200.0).font(egui::TextStyle::Heading)));
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Used by {users} preset{}", if users == 1 { "" } else { "s" })).weak());
                if ui.button("Create preset from this instrument").clicked() {
                    make_preset = true;
                }
            });
            ui.add_space(6.0);

            let labels: Vec<String> = inst.zones.iter().map(|z| z.link.and_then(|l| names.get(l)).cloned().unwrap_or_default()).collect();
            if let Some(i) = zone_map(ui, &inst.zones, &labels, *zone_sel, &sounding) {
                *zone_sel = ZoneSel::Zone(i);
            }
            ui.add_space(6.0);
            let global_root = inst.global.as_ref().and_then(|g| g.get_i(sfgen::OVERRIDING_ROOT_KEY));
            let root = |z: &Zone| {
                let r = z
                    .get_i(sfgen::OVERRIDING_ROOT_KEY)
                    .or(global_root)
                    .filter(|r| *r >= 0)
                    .or_else(|| z.link.and_then(|l| roots.get(l)).map(|r| *r as i32));
                r.map(|r| note_name(r as u8)).unwrap_or_default()
            };
            let res = zone_table(ui, "inst_zones", &mut inst.global, &mut inst.zones, &names, "Sample", zone_sel, edit, Some(("Root", &root)));
            navigate = res.navigate;

            ui.add_space(8.0);
            zone_inspector(ui, "igen", zone_sel, &mut inst.global, &mut inst.zones, false, &names, edit);
        }
        if make_preset {
            self.preset_from_instrument(idx);
        }
        if let Some(i) = navigate {
            self.select(Sel::Sample(i));
        }
    }

    fn sample_editor(&mut self, ui: &mut Ui, idx: usize) {
        if self.wave_for != Some(idx) {
            self.wave_for = Some(idx);
            self.wave.zoom_all(self.sf.samples[idx].data.len());
        }
        let names: Vec<String> = self.sf.samples.iter().map(|s| s.name.clone()).collect();
        let users = self.sf.sample_users(idx);
        let playheads = self.audio.mixer.lock().map(|m| m.playheads(&self.sf.samples[idx].data)).unwrap_or_default();
        if !playheads.is_empty() {
            ui.ctx().request_repaint();
        }

        enum Action {
            Export,
            Replace,
            MakeInstrument,
            Normalize,
            Reverse,
            Navigate(usize),
        }
        let mut action = None;
        {
            let Self { sf, edit, wave, .. } = self;
            let s = &mut sf.samples[idx];
            let len = s.data.len() as u32;

            ui.horizontal(|ui| {
                ui.heading("Sample");
                edit.track(&ui.add(TextEdit::singleline(&mut s.name).char_limit(20).desired_width(200.0).font(egui::TextStyle::Heading)));
            });
            let secs = len as f64 / s.sample_rate.max(1) as f64;
            ui.label(RichText::new(format!(
                "{len} points · {secs:.3} s · {} · used by {users} instrument{}",
                s.type_name(),
                if users == 1 { "" } else { "s" }
            ))
            .weak());
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button("Zoom all").clicked() {
                    wave.zoom_all(s.data.len());
                }
                if ui.add_enabled(s.loop_end > s.loop_start, egui::Button::new("Zoom loop")).clicked() {
                    wave.zoom_to(s.loop_start, s.loop_end);
                }
                if ui.add_enabled(s.loop_end > s.loop_start, egui::Button::new("Zoom loop end")).clicked() {
                    wave.zoom_to(s.loop_end.saturating_sub(200), s.loop_end + 200);
                }
                ui.separator();
                if ui.button("Snap loop to zero crossings").clicked() {
                    s.loop_start = nearest_zero_crossing(&s.data, s.loop_start);
                    s.loop_end = nearest_zero_crossing(&s.data, s.loop_end).max(s.loop_start);
                    edit.structural();
                }
                if ui.button("Clear loop").clicked() {
                    s.loop_start = 0;
                    s.loop_end = 0;
                    edit.structural();
                }
                if ui.button("Loop whole sample").clicked() {
                    s.loop_start = 0;
                    s.loop_end = len;
                    edit.structural();
                }
            });
            let (mut ls, mut le) = (s.loop_start, s.loop_end);
            if wave.show(ui, 220.0, &s.data, &mut ls, &mut le, &playheads) {
                s.loop_start = ls;
                s.loop_end = le;
                edit.changed = true;
            }
            ui.add_space(6.0);

            egui::Grid::new("sample_props").num_columns(4).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.label("Loop start");
                let le = s.loop_end;
                edit.track(&ui.add(DragValue::new(&mut s.loop_start).range(0..=le).speed(1.0)));
                ui.label("Loop end");
                let ls = s.loop_start;
                edit.track(&ui.add(DragValue::new(&mut s.loop_end).range(ls..=len).speed(1.0)));
                ui.end_row();

                ui.label("Root key");
                edit.track(&ui.add(key_drag(&mut s.original_pitch)));
                ui.label("Pitch correction");
                edit.track(&ui.add(DragValue::new(&mut s.pitch_correction).range(-99..=99).suffix(" cents")));
                ui.end_row();

                ui.label("Sample rate");
                edit.track(&ui.add(DragValue::new(&mut s.sample_rate).range(400..=192_000).suffix(" Hz")));
                ui.label("Loop length");
                ui.label(format!("{} points", s.loop_end.saturating_sub(s.loop_start)));
                ui.end_row();

                ui.label("Type");
                let rom = s.sample_type & sample_type::ROM;
                let mut t = s.sample_type & 0x7FFF;
                ComboBox::from_id_salt("stype").selected_text(s.type_name()).show_ui(ui, |ui| {
                    for (v, n) in [
                        (sample_type::MONO, "Mono"),
                        (sample_type::LEFT, "Left"),
                        (sample_type::RIGHT, "Right"),
                        (sample_type::LINKED, "Linked"),
                    ] {
                        edit.track(&ui.selectable_value(&mut t, v, n));
                    }
                });
                s.sample_type = t | rom;
                ui.label("Linked sample");
                ui.add_enabled_ui(t != sample_type::MONO, |ui| {
                    let text = s.link.and_then(|l| names.get(l)).map(String::as_str).unwrap_or("—");
                    ComboBox::from_id_salt("slink").selected_text(text).height(400.0).show_ui(ui, |ui| {
                        if ui.selectable_label(s.link.is_none(), "—").clicked() {
                            s.link = None;
                            edit.changed = true;
                        }
                        for (i, n) in names.iter().enumerate().filter(|(i, _)| *i != idx) {
                            if ui.selectable_label(s.link == Some(i), n).clicked() {
                                s.link = Some(i);
                                edit.changed = true;
                            }
                        }
                    });
                    if let Some(l) = s.link
                        && ui.small_button("➡").on_hover_text("Go to linked sample").clicked()
                    {
                        action = Some(Action::Navigate(l));
                    }
                });
                ui.end_row();
            });

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("🎛 Create instrument from sample").clicked() {
                    action = Some(Action::MakeInstrument);
                }
                if ui.button("💾 Export WAV…").clicked() {
                    action = Some(Action::Export);
                }
                if ui.button("📂 Replace from WAV…").clicked() {
                    action = Some(Action::Replace);
                }
                if ui.button("Normalize").clicked() {
                    action = Some(Action::Normalize);
                }
                if ui.button("Reverse").clicked() {
                    action = Some(Action::Reverse);
                }
            });
        }

        match action {
            Some(Action::Navigate(i)) => self.select(Sel::Sample(i)),
            Some(Action::MakeInstrument) => self.instrument_from_sample(idx),
            Some(Action::Export) => {
                let s = &self.sf.samples[idx];
                if let Some(p) = rfd::FileDialog::new().add_filter("WAV audio", &["wav"]).set_file_name(format!("{}.wav", s.name)).save_file() {
                    match wav::export(&p, s) {
                        Ok(()) => self.status = format!("Exported {}", p.display()),
                        Err(e) => self.error = Some(format!("Could not export:\n{e:#}")),
                    }
                }
            }
            Some(Action::Replace) => {
                if let Some(p) = rfd::FileDialog::new().add_filter("WAV audio", &["wav", "WAV"]).pick_file() {
                    match wav::import(&p) {
                        Ok(mut new) => {
                            self.checkpoint();
                            self.audio.all_off();
                            let n = new.remove(0);
                            let s = &mut self.sf.samples[idx];
                            let len = n.data.len() as u32;
                            s.data = n.data;
                            s.sample_rate = n.sample_rate;
                            s.loop_start = s.loop_start.min(len);
                            s.loop_end = s.loop_end.min(len);
                            self.wave_for = None;
                            self.status = format!("Replaced sample data from {}", p.display());
                        }
                        Err(e) => self.error = Some(format!("Could not import:\n{e:#}")),
                    }
                }
            }
            Some(Action::Normalize) => {
                let peak = self.sf.samples[idx].data.iter().map(|v| (*v as i32).abs()).max().unwrap_or(0);
                if peak > 0 {
                    self.checkpoint();
                    self.audio.all_off();
                    let gain = 32767.0 * 0.98 / peak as f64;
                    let s = &mut self.sf.samples[idx];
                    s.data = Arc::new(s.data.iter().map(|v| (*v as f64 * gain).round().clamp(-32768.0, 32767.0) as i16).collect());
                    self.status = format!("Normalized ({:+.1} dB)", 20.0 * gain.log10());
                }
            }
            Some(Action::Reverse) => {
                self.checkpoint();
                self.audio.all_off();
                let s = &mut self.sf.samples[idx];
                let len = s.data.len() as u32;
                s.data = Arc::new(s.data.iter().rev().copied().collect());
                let (a, b) = (len - s.loop_end, len - s.loop_start);
                s.loop_start = a;
                s.loop_end = b;
            }
            None => {}
        }
    }

    // ---------------------------------------------------------------- dialogs

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.pending.clone() {
            let mut choice = None;
            egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading("Unsaved changes");
                ui.label("Save changes to the current SoundFont before continuing?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.pending = None;
                    self.save();
                    if !self.dirty {
                        self.finish_pending(ctx, action);
                    }
                }
                Some(1) => {
                    self.pending = None;
                    self.dirty = false;
                    self.finish_pending(ctx, action);
                }
                Some(2) => self.pending = None,
                _ => {}
            }
        }

        if let Some(msg) = self.error.clone() {
            let r = egui::Modal::new(egui::Id::new("error")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.heading("Error");
                ui.label(msg);
                ui.add_space(8.0);
                ui.button("OK").clicked()
            });
            if r.inner || r.should_close() {
                self.error = None;
            }
        }

        if self.show_problems {
            let problems = self.sf.validate();
            egui::Window::new("Problems").open(&mut self.show_problems).default_width(420.0).show(ctx, |ui| {
                if problems.is_empty() {
                    ui.label("✔ No problems found.");
                }
                ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for p in problems {
                        ui.label(format!("⚠ {p}"));
                    }
                });
            });
        }
    }

    fn finish_pending(&mut self, ctx: &egui::Context, action: Pending) {
        if matches!(action, Pending::Quit) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.run_pending(action);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let file = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        let title = format!("{}{file} — {APP_NAME}", if self.dirty { "● " } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Adds a system font with Thai coverage as a fallback, so non-ASCII file names render.
fn install_fallback_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/noto/NotoSansThai-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansThaiLooped-Regular.ttf",
        "/usr/share/fonts/TTF/Sarabun-Regular.ttf",
        "/usr/share/fonts/gnu-free/FreeSerif.otf",
        "/usr/share/fonts/truetype/freefont/FreeSerif.ttf",
        "C:\\Windows\\Fonts\\tahoma.ttf",
        "/System/Library/Fonts/Supplemental/Thonburi.ttc",
    ];
    let Some(bytes) = CANDIDATES.iter().find_map(|p| std::fs::read(p).ok()) else { return };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("fallback".into(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("fallback".into());
    }
    ctx.set_fonts(fonts);
}

/// Generator + modulator editor for whichever zone is selected.
#[allow(clippy::too_many_arguments)]
fn zone_inspector(
    ui: &mut Ui,
    id: &str,
    sel: &mut ZoneSel,
    global: &mut Option<Zone>,
    zones: &mut [Zone],
    preset_level: bool,
    names: &[String],
    edit: &mut Edit,
) {
    let (title, zone, parent) = match *sel {
        ZoneSel::Global => match global.as_mut() {
            Some(g) => ("Global zone".to_string(), g, None),
            None => return,
        },
        ZoneSel::Zone(i) if i < zones.len() => {
            let link = zones[i].link.and_then(|l| names.get(l)).cloned().unwrap_or_default();
            (format!("Zone #{} — {link}", i + 1), &mut zones[i], global.as_ref())
        }
        _ => {
            ui.label(RichText::new("Select a zone to edit its generators.").weak());
            return;
        }
    };
    ui.separator();
    ui.horizontal(|ui| {
        ui.heading(title);
        if preset_level {
            ui.label(RichText::new("(values are offsets added to the instrument)").weak());
        }
    });
    ui.add_space(4.0);
    generator_table(ui, id, zone, parent, preset_level, edit);
    modulator_table(ui, id, &mut zone.mods, edit);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Intercept window close when there are unsaved changes.
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }

        let before = self.sf.clone();
        self.handle_input(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("keyboard").show(ui, |ui| self.keyboard_panel(ui));
        egui::Panel::left("list").resizable(true).default_size(280.0).min_size(200.0).show(ui, |ui| self.list_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.editor(ui));
        });
        self.dialogs(&ctx);

        self.finish_frame(&ctx, before);
        self.update_title(&ctx);
        if self.audio.mixer.lock().is_ok_and(|m| m.active_voices() > 0) {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }
}
