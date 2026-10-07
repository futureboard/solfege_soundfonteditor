use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{
    Align, Color32, ComboBox, DragValue, Key, KeyboardShortcut, Layout, Modifiers, RichText, ScrollArea, TextEdit, Ui,
};

use crate::audio::{self, Audio, AudioConfig, OutputDevice};
use crate::sf2::generators::{self as sfgen, note_name};
use crate::sf2::{self, Instrument, Preset, SoundFont, Zone, sample_type};
use crate::ui::gens::{generator_table, modulator_table};
use crate::ui::keyboard::piano;
use crate::ui::waveform::{WaveView, nearest_zero_crossing};
use crate::ui::zones::{ZoneSel, key_drag, zone_map, zone_table};
use crate::ui::{Edit, card};
use crate::wav;

pub const APP_NAME: &str = "Solfege SoundFont Editor";
const MAX_UNDO: usize = 100;
const KEY_DEVICE: &str = "audio_device";
const KEY_BUFFER: &str = "audio_buffer";
const BUFFER_SIZES: &[u32] = &[64, 128, 256, 512, 1024, 2048];

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
    /// Amount for the sample editor's "Apply gain" tool.
    gain_db: f32,

    wave: WaveView,
    wave_for: Option<usize>,
    status: String,
    pending: Option<Pending>,
    error: Option<String>,
    show_problems: bool,
    show_audio_settings: bool,
    /// Settings being edited in the audio window (applied on "Apply").
    audio_draft: AudioConfig,
    /// Cached device list; refreshed when the settings window opens.
    devices: Vec<OutputDevice>,
    title: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        cc.egui_ctx.global_style_mut(|s| {
            use egui::{FontId, TextStyle};
            s.spacing.item_spacing = egui::vec2(10.0, 8.0);
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
            s.spacing.interact_size.y = 24.0;
            s.spacing.indent = 18.0;
            s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
            s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
            s.text_styles.insert(TextStyle::Heading, FontId::proportional(21.0));
            s.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
            s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
        });
        install_fallback_fonts(&cc.egui_ctx);
        // Restore the output device / buffer size chosen in a previous session.
        let audio_config = AudioConfig {
            device_id: cc.storage.and_then(|s| s.get_string(KEY_DEVICE)).filter(|s| !s.is_empty()),
            buffer_size: cc.storage.and_then(|s| s.get_string(KEY_BUFFER)).and_then(|s| s.parse().ok()),
        };
        let audio = Audio::new(&audio_config);
        let status = audio.status();
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
            gain_db: -6.0,
            wave: WaveView::default(),
            wave_for: None,
            status,
            pending: None,
            error: None,
            show_problems: false,
            show_audio_settings: false,
            audio_draft: audio_config,
            devices: Vec::new(),
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
                if ui.button("Audio settings…").clicked() {
                    self.open_audio_settings();
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
        ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
            let title = RichText::new(format!("ℹ  {}", self.sf.info.name)).strong();
            if ui.selectable_label(self.sel == Sel::Info, title).on_hover_text("SoundFont info").clicked() {
                self.select(Sel::Info);
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (tab, label, n) in [
                (Tab::Presets, "Presets", self.sf.presets.len()),
                (Tab::Instruments, "Instruments", self.sf.instruments.len()),
                (Tab::Samples, "Samples", self.sf.samples.len()),
            ] {
                ui.selectable_value(&mut self.tab, tab, format!("{label} {n}"));
            }
        });
        ui.add_space(2.0);
        ui.add(TextEdit::singleline(&mut self.filter).hint_text("🔍 Filter").desired_width(f32::INFINITY).margin(egui::vec2(6.0, 4.0)));
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
        ui.add_space(2.0);
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

        let mut clicked = None;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let row_h = ui.spacing().interact_size.y + 2.0;
            ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h, items.len(), |ui, range| {
                ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                    for (sel, label, used) in &items[range] {
                        let text = if *used { RichText::new(label) } else { RichText::new(label).weak().italics() };
                        let r = ui.add(egui::Button::selectable(self.sel == *sel, text).min_size(egui::vec2(0.0, row_h)));
                        let r = if *used { r } else { r.on_hover_text("Not used by any preset/instrument") };
                        if r.clicked() {
                            clicked = Some(*sel);
                        }
                    }
                });
            });
        });
        if let Some(s) = clicked {
            self.select(s);
        }
    }

    fn keyboard_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let target = match self.sel {
                Sel::Preset(i) => format!("Preset · {}", self.sf.presets[i].name),
                Sel::Instrument(i) => format!("Instrument · {}", self.sf.instruments[i].name),
                Sel::Sample(i) => format!("Sample · {}", self.sf.samples[i].name),
                Sel::Info => "Select a preset, instrument or sample to play".into(),
            };
            ui.label(RichText::new("🎹").size(17.0));
            ui.strong(target);
            ui.add_space(16.0);
            ui.label("Velocity");
            ui.add(DragValue::new(&mut self.velocity).range(1..=127));
            ui.add_space(8.0);
            ui.label("Octave");
            ui.add(DragValue::new(&mut self.octave).range(-1..=8)).on_hover_text("PC keys Z–M / Q–P play from this octave (←/→ to change)");
            if matches!(self.sel, Sel::Sample(_)) {
                ui.add_space(8.0);
                ui.checkbox(&mut self.loop_preview, "Loop");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("⏹ Stop").on_hover_text("All notes off (Esc)").clicked() {
                    self.audio.all_off();
                    self.sounding = [0; 128];
                }
                if ui.button("⚙ Audio").on_hover_text("Audio output settings").clicked() {
                    self.open_audio_settings();
                }
                ui.add_space(12.0);
                if let Ok(mut m) = self.audio.mixer.lock() {
                    ui.label(RichText::new(format!("{:>2} voices", m.active_voices())).weak().monospace());
                    ui.add(egui::Slider::new(&mut m.master, 0.0..=1.5).show_value(false));
                    ui.label("Volume");
                }
            });
        });
        ui.add_space(6.0);
        let sounding = self.sounding_mask();
        let mapped = self.mapped_keys();
        let out = piano(ui, 76.0, &sounding, &mapped, &mut self.mouse_note);
        if let Some(k) = out.note_off {
            self.note_off(k);
        }
        if let Some(k) = out.note_on {
            self.note_on(k);
        }
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
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(760.0);
            ui.label(RichText::new("SOUNDFONT").small().weak());
            ui.heading(&self.sf.info.name);
            ui.label(
                RichText::new(format!(
                    "{} presets · {} instruments · {} samples",
                    self.sf.presets.len(),
                    self.sf.instruments.len(),
                    self.sf.samples.len()
                ))
                .weak(),
            );
            ui.add_space(14.0);
            let edit = &mut self.edit;
            let info = &mut self.sf.info;
            card(ui, "Details", |ui| {
                egui::Grid::new("info").num_columns(2).spacing([20.0, 10.0]).show(ui, |ui| {
                    let mut field = |ui: &mut Ui, label: &str, value: &mut String, multiline: bool| {
                        ui.label(label);
                        let r = if multiline {
                            ui.add(TextEdit::multiline(value).desired_width(f32::INFINITY).desired_rows(5).margin(egui::vec2(6.0, 4.0)))
                        } else {
                            ui.add(TextEdit::singleline(value).desired_width(f32::INFINITY).char_limit(255).margin(egui::vec2(6.0, 4.0)))
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
                });
            });
            ui.add_space(12.0);
            card(ui, "File", |ui| {
                egui::Grid::new("info_file").num_columns(2).spacing([20.0, 8.0]).show(ui, |ui| {
                    ui.label("Path");
                    ui.label(RichText::new(self.path.as_ref().map_or("Not saved yet".into(), |p| p.display().to_string())).weak());
                    ui.end_row();
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
            });
            ui.add_space(12.0);
            ui.label(
                RichText::new(
                    "Tip: drop a .sf2 file to open it, or .wav files to import samples. \
                     Play notes with the on-screen piano or the PC keyboard (Z–M, Q–P).",
                )
                .weak(),
            );
        });
    }

    fn preset_editor(&mut self, ui: &mut Ui, idx: usize) {
        let names: Vec<String> = self.sf.instruments.iter().map(|x| x.name.clone()).collect();
        let sounding = self.sounding_mask();
        let mut navigate = None;
        let mut make_unique = false;
        {
            let Self { sf, edit, zone_sel, .. } = self;
            let taken: Vec<(u16, u16)> =
                sf.presets.iter().enumerate().filter(|(i, _)| *i != idx).map(|(_, p)| (p.bank, p.program)).collect();
            let p = &mut sf.presets[idx];

            let meta = format!("{} zone{}", p.zones.len(), if p.zones.len() == 1 { "" } else { "s" });
            editor_header(ui, "Preset", &mut p.name, edit, &meta, |ui, edit| {
                // Added right to left; reads "⚠ … Fix   Bank [n]   Program [n]".
                edit.track(&ui.add(DragValue::new(&mut p.program).range(0..=127)));
                ui.label("Program");
                ui.add_space(8.0);
                edit.track(&ui.add(DragValue::new(&mut p.bank).range(0..=128)));
                ui.label("Bank");
                if taken.contains(&(p.bank, p.program)) {
                    ui.add_space(8.0);
                    make_unique = ui.button("Fix").on_hover_text("Pick a free program number").clicked();
                    ui.label(RichText::new("⚠ bank/program already used").color(ui.visuals().warn_fg_color));
                }
            });

            let labels: Vec<String> = p.zones.iter().map(|z| z.link.and_then(|l| names.get(l)).cloned().unwrap_or_default()).collect();
            card(ui, "Key map", |ui| {
                if let Some(i) = zone_map(ui, &p.zones, &labels, *zone_sel, &sounding) {
                    *zone_sel = ZoneSel::Zone(i);
                }
            });
            ui.add_space(12.0);
            zone_split(ui, "preset", zone_sel, &mut p.global, &mut p.zones, &names, "Instrument", true, edit, None, &mut navigate);
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
        let mut navigate = None;
        let mut make_preset = false;
        {
            let Self { sf, edit, zone_sel, .. } = self;
            let inst = &mut sf.instruments[idx];
            let meta = format!(
                "{} zone{} · used by {users} preset{}",
                inst.zones.len(),
                if inst.zones.len() == 1 { "" } else { "s" },
                if users == 1 { "" } else { "s" }
            );
            editor_header(ui, "Instrument", &mut inst.name, edit, &meta, |ui, _| {
                if ui.button("Create preset").on_hover_text("Create a new preset that plays this instrument").clicked() {
                    make_preset = true;
                }
            });

            let labels: Vec<String> = inst.zones.iter().map(|z| z.link.and_then(|l| names.get(l)).cloned().unwrap_or_default()).collect();
            card(ui, "Key map", |ui| {
                if let Some(i) = zone_map(ui, &inst.zones, &labels, *zone_sel, &sounding) {
                    *zone_sel = ZoneSel::Zone(i);
                }
            });
            ui.add_space(12.0);
            let global_root = inst.global.as_ref().and_then(|g| g.get_i(sfgen::OVERRIDING_ROOT_KEY));
            let root = |z: &Zone| {
                let r = z
                    .get_i(sfgen::OVERRIDING_ROOT_KEY)
                    .or(global_root)
                    .filter(|r| *r >= 0)
                    .or_else(|| z.link.and_then(|l| roots.get(l)).map(|r| *r as i32));
                r.map(|r| note_name(r as u8)).unwrap_or_default()
            };
            zone_split(ui, "instrument", zone_sel, &mut inst.global, &mut inst.zones, &names, "Sample", false, edit, Some(("Root", &root)), &mut navigate);
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
            Gain(f32),
            Reverse,
            Navigate(usize),
        }
        let mut action = None;
        {
            let Self { sf, edit, wave, gain_db, .. } = self;
            let s = &mut sf.samples[idx];
            let len = s.data.len() as u32;

            let secs = len as f64 / s.sample_rate.max(1) as f64;
            let meta = format!(
                "{len} points · {secs:.3} s · {} · used by {users} instrument{}",
                s.type_name(),
                if users == 1 { "" } else { "s" }
            );
            editor_header(ui, "Sample", &mut s.name, edit, &meta, |ui, _| {
                if ui.button("➕ Create instrument").on_hover_text("Create an instrument that plays this sample").clicked() {
                    action = Some(Action::MakeInstrument);
                }
            });

            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                card(ui, "Waveform", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Zoom all").clicked() {
                            wave.zoom_all(s.data.len());
                        }
                        let has_loop = s.loop_end > s.loop_start;
                        if ui.add_enabled(has_loop, egui::Button::new("Zoom loop")).clicked() {
                            wave.zoom_to(s.loop_start, s.loop_end);
                        }
                        if ui.add_enabled(has_loop, egui::Button::new("Zoom loop end")).clicked() {
                            wave.zoom_to(s.loop_end.saturating_sub(200), s.loop_end + 200);
                        }
                        ui.add_space(12.0);
                        if ui.button("Snap to zero crossings").on_hover_text("Move both loop points to the nearest rising zero crossing").clicked() {
                            s.loop_start = nearest_zero_crossing(&s.data, s.loop_start);
                            s.loop_end = nearest_zero_crossing(&s.data, s.loop_end).max(s.loop_start);
                            edit.structural();
                        }
                        if ui.button("Loop whole sample").clicked() {
                            s.loop_start = 0;
                            s.loop_end = len;
                            edit.structural();
                        }
                        if ui.button("Clear loop").clicked() {
                            s.loop_start = 0;
                            s.loop_end = 0;
                            edit.structural();
                        }
                    });
                    ui.add_space(4.0);
                    let (mut ls, mut le) = (s.loop_start, s.loop_end);
                    if wave.show(ui, 240.0, &s.data, &mut ls, &mut le, &playheads) {
                        s.loop_start = ls;
                        s.loop_end = le;
                        edit.changed = true;
                    }
                });
                ui.add_space(12.0);

                ui.columns(2, |cols| {
                    card(&mut cols[0], "Loop & pitch", |ui| {
                        egui::Grid::new("sample_loop").num_columns(2).spacing([20.0, 10.0]).show(ui, |ui| {
                            ui.label("Loop start");
                            let le = s.loop_end;
                            edit.track(&ui.add(DragValue::new(&mut s.loop_start).range(0..=le).speed(1.0)));
                            ui.end_row();
                            ui.label("Loop end");
                            let ls = s.loop_start;
                            edit.track(&ui.add(DragValue::new(&mut s.loop_end).range(ls..=len).speed(1.0)));
                            ui.end_row();
                            ui.label("Loop length");
                            ui.label(RichText::new(format!("{} points", s.loop_end.saturating_sub(s.loop_start))).weak());
                            ui.end_row();
                            ui.label("Root key");
                            edit.track(&ui.add(key_drag(&mut s.original_pitch)));
                            ui.end_row();
                            ui.label("Pitch correction");
                            edit.track(&ui.add(DragValue::new(&mut s.pitch_correction).range(-99..=99).suffix(" cents")));
                            ui.end_row();
                        });
                    });
                    card(&mut cols[1], "Format", |ui| {
                        egui::Grid::new("sample_fmt").num_columns(2).spacing([20.0, 10.0]).show(ui, |ui| {
                            ui.label("Sample rate");
                            edit.track(&ui.add(DragValue::new(&mut s.sample_rate).range(400..=192_000).suffix(" Hz")));
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
                            ui.end_row();
                            ui.label("Linked sample");
                            ui.add_enabled_ui(t != sample_type::MONO, |ui| {
                                ui.horizontal(|ui| {
                                    let text = s.link.and_then(|l| names.get(l)).map(String::as_str).unwrap_or("—");
                                    ComboBox::from_id_salt("slink").selected_text(text).width(170.0).height(400.0).show_ui(ui, |ui| {
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
                            });
                            ui.end_row();
                        });
                    });
                });
                ui.add_space(12.0);

                card(ui, "Level & processing", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let peak = s.data.iter().map(|v| (*v as i32).abs()).max().unwrap_or(0);
                        let peak_db = if peak > 0 { 20.0 * (peak as f64 / 32768.0).log10() } else { f64::NEG_INFINITY };
                        let text = RichText::new(format!("Peak {peak_db:.1} dBFS")).monospace();
                        ui.label(if peak >= 32767 { text.color(ui.visuals().warn_fg_color) } else { text })
                            .on_hover_text("At 0 dBFS the sample is at full scale and may already be clipped in the source.");
                        ui.add_space(16.0);
                        ui.add(DragValue::new(gain_db).range(-48.0..=24.0).speed(0.1).fixed_decimals(1).suffix(" dB"));
                        if ui.button("Apply gain").on_hover_text("Scale the sample data (negative = quieter). Values that exceed full scale are clipped.").clicked() {
                            action = Some(Action::Gain(*gain_db));
                        }
                        if ui.button("Normalize").on_hover_text("Raise the peak to -0.2 dBFS").clicked() {
                            action = Some(Action::Normalize);
                        }
                        if ui.button("Reverse").clicked() {
                            action = Some(Action::Reverse);
                        }
                    });
                });
                ui.add_space(12.0);

                card(ui, "File", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("💾 Export WAV…").clicked() {
                            action = Some(Action::Export);
                        }
                        if ui.button("📂 Replace from WAV…").on_hover_text("Replace the audio data, keeping name, root key and loop").clicked() {
                            action = Some(Action::Replace);
                        }
                    });
                });
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
            Some(Action::Gain(db)) => {
                self.checkpoint();
                self.audio.all_off();
                let gain = 10f64.powf(db as f64 / 20.0);
                let s = &mut self.sf.samples[idx];
                let mut clipped = 0usize;
                s.data = Arc::new(
                    s.data
                        .iter()
                        .map(|v| {
                            let x = (*v as f64 * gain).round();
                            if !(-32768.0..=32767.0).contains(&x) {
                                clipped += 1;
                            }
                            x.clamp(-32768.0, 32767.0) as i16
                        })
                        .collect(),
                );
                self.status = if clipped > 0 {
                    format!("Applied {db:+.1} dB — {clipped} points clipped (undo with Ctrl+Z)")
                } else {
                    format!("Applied {db:+.1} dB")
                };
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

    fn open_audio_settings(&mut self) {
        self.devices = audio::output_devices();
        self.audio_draft = self.audio.config.clone();
        self.show_audio_settings = true;
    }

    fn audio_settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_audio_settings;
        let mut apply = false;
        egui::Window::new("Audio settings").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("audio_settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Output device");
                ui.horizontal(|ui| {
                    let current = match &self.audio_draft.device_id {
                        None => "System default".to_string(),
                        Some(id) => self.devices.iter().find(|d| &d.id == id).map_or_else(|| format!("{id} (not found)"), |d| d.name.clone()),
                    };
                    ComboBox::from_id_salt("out_dev").selected_text(current).width(320.0).height(400.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.audio_draft.device_id, None, "System default");
                        for d in &self.devices {
                            ui.selectable_value(&mut self.audio_draft.device_id, Some(d.id.clone()), &d.name).on_hover_text(&d.id);
                        }
                    });
                    if ui.button("⟳").on_hover_text("Refresh device list").clicked() {
                        self.devices = audio::output_devices();
                    }
                });
                ui.end_row();

                ui.label("Buffer size");
                let rate = self.audio.sample_rate.max(1) as f32;
                let label = |b: Option<u32>| match b {
                    None => "Device default".to_string(),
                    Some(n) => format!("{n} frames  ({:.1} ms)", n as f32 / rate * 1000.0),
                };
                ComboBox::from_id_salt("out_buf").selected_text(label(self.audio_draft.buffer_size)).width(320.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.audio_draft.buffer_size, None, label(None));
                    for &n in BUFFER_SIZES {
                        ui.selectable_value(&mut self.audio_draft.buffer_size, Some(n), label(Some(n)));
                    }
                });
                ui.end_row();

                ui.label("Status");
                let status = self.audio.status();
                if self.audio.error.is_some() {
                    ui.colored_label(ui.visuals().error_fg_color, status);
                } else {
                    ui.label(RichText::new(status).weak());
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Smaller buffers lower the latency when playing notes, but can crackle on a busy system.").weak().small());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(self.audio_draft != self.audio.config, egui::Button::new("Apply")).clicked() {
                    apply = true;
                }
                if ui.button("Test sound").clicked() {
                    self.test_tone();
                }
            });
        });
        self.show_audio_settings = open;
        if apply {
            self.mouse_note = None;
            self.pc_held.clear();
            self.sounding = [0; 128];
            self.audio.reopen(&self.audio_draft.clone());
            self.status = self.audio.status();
        }
    }

    /// A short sine beep so the output can be checked without loading a SoundFont.
    fn test_tone(&mut self) {
        let rate = 44_100u32;
        let data: Vec<i16> = (0..rate / 2)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let fade = (1.0 - t * 2.0).max(0.0);
                ((t * 440.0 * std::f32::consts::TAU).sin() * 12_000.0 * fade) as i16
            })
            .collect();
        let len = data.len();
        let voice = audio::VoiceParams {
            data: Arc::new(data),
            start: 0,
            end: len,
            loop_start: 0,
            loop_end: 0,
            mode: 0,
            sample_rate: rate,
            pitch_ratio: 1.0,
            gain: 0.6,
            pan: 0.0,
            delay: 0.0,
            attack: 0.005,
            hold: 0.0,
            decay: 100.0,
            sustain: 1.0,
            release: 0.05,
            filter_fc: f32::MAX,
            filter_q: 0.707,
            exclusive_class: 0,
        };
        // Note 128 is outside the MIDI range, so it never collides with a held key.
        self.audio.note_on(128, vec![voice]);
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

/// Page header: kind label, editable name, a line of details, and right-aligned controls.
fn editor_header(ui: &mut Ui, kind: &str, name: &mut String, edit: &mut Edit, meta: &str, controls: impl FnOnce(&mut Ui, &mut Edit)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(kind.to_uppercase()).small().weak());
            edit.track(&ui.add(
                TextEdit::singleline(name).char_limit(20).desired_width(280.0).font(egui::TextStyle::Heading).margin(egui::vec2(6.0, 2.0)),
            ));
            ui.label(RichText::new(meta).weak());
        });
        // Right-to-left: `controls` adds its widgets starting from the rightmost one.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| controls(ui, edit));
    });
    ui.add_space(12.0);
}

/// Zone list on the left, generator inspector for the selected zone on the right.
#[allow(clippy::too_many_arguments)]
fn zone_split(
    ui: &mut Ui,
    id: &str,
    sel: &mut ZoneSel,
    global: &mut Option<Zone>,
    zones: &mut Vec<Zone>,
    names: &[String],
    kind: &str,
    preset_level: bool,
    edit: &mut Edit,
    extra: Option<crate::ui::zones::ExtraColumn<'_>>,
    navigate: &mut Option<usize>,
) {
    egui::Panel::right(egui::Id::new((id, "inspector")))
        .resizable(true)
        .show_separator_line(false)
        .default_size(430.0)
        .size_range(340.0..=760.0)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 12, ..Default::default() }))
        .show(ui, |ui| {
            card(ui, "", |ui| {
                ScrollArea::vertical().id_salt((id, "inspector_scroll")).auto_shrink(false).show(ui, |ui| {
                    zone_inspector(ui, id, sel, global, zones, preset_level, names, edit);
                });
            });
        });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        card(ui, "Zones", |ui| {
            ScrollArea::both().id_salt((id, "zones_scroll")).auto_shrink(false).show(ui, |ui| {
                *navigate = zone_table(ui, id, global, zones, names, kind, sel, edit, extra).navigate;
            });
        });
    });
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
            (format!("Zone #{} · {link}", i + 1), &mut zones[i], global.as_ref())
        }
        _ => {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No zone selected").strong());
                ui.label(RichText::new("Click a zone in the list or the key map\nto edit its generators.").weak());
            });
            return;
        }
    };
    ui.label(RichText::new(title).size(17.0).strong());
    if preset_level {
        ui.label(RichText::new("Values are offsets added to the instrument.").weak().small());
    }
    ui.add_space(6.0);
    generator_table(ui, id, zone, parent, preset_level, edit);
    ui.add_space(4.0);
    modulator_table(ui, id, &mut zone.mods, edit);
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(KEY_DEVICE, self.audio.config.device_id.clone().unwrap_or_default());
        storage.set_string(KEY_BUFFER, self.audio.config.buffer_size.map(|b| b.to_string()).unwrap_or_default());
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Intercept window close when there are unsaved changes.
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }

        let before = self.sf.clone();
        self.handle_input(&ctx);

        let style = ctx.global_style();
        let panel = |x: i8, y: i8| egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::symmetric(x, y));
        egui::Panel::top("menu").frame(panel(10, 4)).show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").frame(panel(14, 4)).show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("keyboard").frame(panel(14, 10)).show(ui, |ui| self.keyboard_panel(ui));
        egui::Panel::left("list").frame(panel(12, 12)).resizable(true).default_size(290.0).min_size(220.0).show(ui, |ui| self.list_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&style).inner_margin(egui::Margin::same(18)))
            .show(ui, |ui| self.editor(ui));
        self.dialogs(&ctx);
        self.audio_settings_window(&ctx);

        self.finish_frame(&ctx, before);
        self.update_title(&ctx);
        if self.audio.mixer.lock().is_ok_and(|m| m.active_voices() > 0) {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }
}

#[cfg(test)]
mod screenshots {
    //! Renders the UI offscreen with wgpu so the layout can be reviewed as PNGs.
    //! Run with: `SHOT_DIR=/some/dir cargo test screenshots -- --nocapture`
    use super::*;
    use crate::sf2::Sample;

    fn demo_font() -> SoundFont {
        let mut sf = SoundFont::new_empty();
        sf.info.name = "Demo Kit".into();
        let names = ["Kick", "Side Stick", "Snare", "Clap", "Snare 2", "Low Tom", "Closed Hat", "Floor Tom", "Pedal Hat", "Mid Tom", "Open Hat", "Hi Tom", "Crash", "Ride"];
        let mut drums = Instrument { name: "Drum Kit".into(), ..Default::default() };
        for (i, n) in names.iter().enumerate() {
            let data: Vec<i16> = (0..20_000).map(|t| (((t as f32) * 0.05 * (i + 1) as f32).sin() * 20_000.0 * (1.0 - t as f32 / 20_000.0)) as i16).collect();
            sf.samples.push(Sample { name: n.to_string(), data: Arc::new(data), sample_rate: 44_100, original_pitch: 60, sample_type: sample_type::MONO, ..Default::default() });
            let key = 36 + i as u8;
            let mut z = Zone { link: Some(i), ..Default::default() };
            z.set_range(sfgen::KEY_RANGE, key, key);
            if matches!(*n, "Closed Hat" | "Pedal Hat" | "Open Hat") {
                z.set_i(sfgen::EXCLUSIVE_CLASS, 1);
            }
            drums.zones.push(z);
        }
        sf.instruments.push(drums);
        sf.presets.push(Preset { name: "Standard Kit".into(), bank: 128, zones: vec![Zone { link: Some(0), ..Default::default() }], ..Default::default() });
        sf
    }

    #[test]
    fn render_layout() {
        let Ok(dir) = std::env::var("SHOT_DIR") else { return };
        for (name, sel, zone) in [
            ("instrument", Sel::Instrument(0), ZoneSel::Zone(6)),
            ("preset", Sel::Preset(0), ZoneSel::Zone(0)),
            ("sample", Sel::Sample(10), ZoneSel::None),
            ("info", Sel::Info, ZoneSel::None),
        ] {
            let mut harness = egui_kittest::Harness::builder().with_size([1360.0, 860.0]).wgpu().build_eframe(|cc| {
                let mut app = App::new(cc, None);
                app.sf = demo_font();
                app.select(sel);
                app.zone_sel = zone;
                app
            });
            harness.run();
            let img = harness.render().expect("render");
            img.save(format!("{dir}/{name}.png")).unwrap();
        }
    }
}
