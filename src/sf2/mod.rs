//! In-memory SoundFont 2 model plus RIFF reader/writer.

pub mod generators;
mod read;
mod write;

use std::sync::Arc;

pub use read::read_sf2;
pub use write::write_sf2;

use generators as sfgen;

#[derive(Clone, Debug, Default)]
pub struct Info {
    pub version: (u16, u16),
    pub sound_engine: String,
    pub name: String,
    pub rom_name: String,
    pub rom_version: Option<(u16, u16)>,
    pub creation_date: String,
    pub engineers: String,
    pub product: String,
    pub copyright: String,
    pub comment: String,
    pub software: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modulator {
    pub src: u16,
    pub dest: u16,
    pub amount: i16,
    pub amt_src: u16,
    pub transform: u16,
}

/// A preset or instrument zone. `link` is the instrument index (preset zones)
/// or sample index (instrument zones); `None` for a global zone.
#[derive(Clone, Debug, Default)]
pub struct Zone {
    /// Generators in file order, excluding the terminal instrument/sampleID.
    pub gens: Vec<(u16, u16)>,
    pub mods: Vec<Modulator>,
    pub link: Option<usize>,
}

impl Zone {
    pub fn get(&self, id: u16) -> Option<u16> {
        self.gens.iter().find(|g| g.0 == id).map(|g| g.1)
    }

    pub fn get_i(&self, id: u16) -> Option<i32> {
        self.get(id).map(|v| match sfgen::info(id).map(|g| g.kind) {
            Some(sfgen::GenKind::Unsigned) => v as i32,
            _ => v as i16 as i32,
        })
    }

    pub fn set(&mut self, id: u16, raw: u16) {
        match self.gens.iter_mut().find(|g| g.0 == id) {
            Some(g) => g.1 = raw,
            None => self.gens.push((id, raw)),
        }
    }

    pub fn set_i(&mut self, id: u16, v: i32) {
        self.set(id, v as i16 as u16);
    }

    pub fn remove(&mut self, id: u16) {
        self.gens.retain(|g| g.0 != id);
    }

    pub fn range(&self, id: u16) -> Option<(u8, u8)> {
        self.get(id).map(|v| ((v & 0xFF) as u8, (v >> 8) as u8))
    }

    pub fn key_range(&self) -> (u8, u8) {
        self.range(sfgen::KEY_RANGE).unwrap_or((0, 127))
    }

    pub fn vel_range(&self) -> (u8, u8) {
        self.range(sfgen::VEL_RANGE).unwrap_or((0, 127))
    }

    pub fn set_range(&mut self, id: u16, lo: u8, hi: u8) {
        self.set(id, lo as u16 | (hi as u16) << 8);
    }

    /// Generators sorted the way the spec requires: keyRange, velRange first.
    pub fn ordered_gens(&self) -> Vec<(u16, u16)> {
        let mut out = Vec::with_capacity(self.gens.len());
        for first in [sfgen::KEY_RANGE, sfgen::VEL_RANGE] {
            if let Some(v) = self.get(first) {
                out.push((first, v));
            }
        }
        out.extend(
            self.gens
                .iter()
                .filter(|g| {
                    !matches!(g.0, sfgen::KEY_RANGE | sfgen::VEL_RANGE | sfgen::INSTRUMENT | sfgen::SAMPLE_ID)
                })
                .copied(),
        );
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct Preset {
    pub name: String,
    pub program: u16,
    pub bank: u16,
    pub library: u32,
    pub genre: u32,
    pub morphology: u32,
    pub global: Option<Zone>,
    pub zones: Vec<Zone>,
}

#[derive(Clone, Debug, Default)]
pub struct Instrument {
    pub name: String,
    pub global: Option<Zone>,
    pub zones: Vec<Zone>,
}

pub mod sample_type {
    pub const MONO: u16 = 1;
    pub const RIGHT: u16 = 2;
    pub const LEFT: u16 = 4;
    pub const LINKED: u16 = 8;
    pub const ROM: u16 = 0x8000;
}

#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub name: String,
    pub data: Arc<Vec<i16>>,
    /// Loop points relative to the start of `data`.
    pub loop_start: u32,
    pub loop_end: u32,
    pub sample_rate: u32,
    pub original_pitch: u8,
    pub pitch_correction: i8,
    pub link: Option<usize>,
    pub sample_type: u16,
}

impl Sample {
    pub fn type_name(&self) -> &'static str {
        match self.sample_type & 0x7FFF {
            sample_type::MONO => "Mono",
            sample_type::RIGHT => "Right",
            sample_type::LEFT => "Left",
            sample_type::LINKED => "Linked",
            _ => "Unknown",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SoundFont {
    pub info: Info,
    pub presets: Vec<Preset>,
    pub instruments: Vec<Instrument>,
    pub samples: Vec<Sample>,
}

impl SoundFont {
    pub fn new_empty() -> Self {
        Self {
            info: Info {
                version: (2, 4),
                sound_engine: "EMU8000".into(),
                name: "Untitled".into(),
                software: concat!("Solfege SoundFont Editor ", env!("CARGO_PKG_VERSION")).into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn sort_presets(&mut self) {
        self.presets.sort_by_key(|p| (p.bank, p.program));
    }

    /// Remove a sample, dropping instrument zones that use it and fixing indices.
    pub fn remove_sample(&mut self, idx: usize) {
        self.samples.remove(idx);
        let fix = |l: &mut Option<usize>| match *l {
            Some(i) if i == idx => *l = None,
            Some(i) if i > idx => *l = Some(i - 1),
            _ => {}
        };
        for s in &mut self.samples {
            fix(&mut s.link);
        }
        for inst in &mut self.instruments {
            inst.zones.retain(|z| z.link != Some(idx));
            for z in &mut inst.zones {
                fix(&mut z.link);
            }
        }
    }

    /// Remove an instrument, dropping preset zones that use it and fixing indices.
    pub fn remove_instrument(&mut self, idx: usize) {
        self.instruments.remove(idx);
        for p in &mut self.presets {
            p.zones.retain(|z| z.link != Some(idx));
            for z in &mut p.zones {
                if let Some(i) = z.link
                    && i > idx
                {
                    z.link = Some(i - 1);
                }
            }
        }
    }

    pub fn instrument_users(&self, idx: usize) -> usize {
        self.presets
            .iter()
            .filter(|p| p.zones.iter().any(|z| z.link == Some(idx)))
            .count()
    }

    pub fn sample_users(&self, idx: usize) -> usize {
        self.instruments
            .iter()
            .filter(|i| i.zones.iter().any(|z| z.link == Some(idx)))
            .count()
    }

    /// Problems that would make the file invalid or confusing for players.
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashMap::new();
        for p in &self.presets {
            if let Some(other) = seen.insert((p.bank, p.program), &p.name) {
                out.push(format!(
                    "Presets \"{}\" and \"{}\" share bank {} / program {}",
                    other, p.name, p.bank, p.program
                ));
            }
            if p.zones.is_empty() {
                out.push(format!("Preset \"{}\" has no zones", p.name));
            }
        }
        for i in &self.instruments {
            if i.zones.is_empty() {
                out.push(format!("Instrument \"{}\" has no zones", i.name));
            }
        }
        for s in &self.samples {
            let len = s.data.len() as u32;
            if s.loop_end > len || s.loop_start > s.loop_end {
                out.push(format!("Sample \"{}\" has invalid loop points", s.name));
            }
            if s.sample_rate == 0 {
                out.push(format!("Sample \"{}\" has a sample rate of 0", s.name));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
