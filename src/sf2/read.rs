use std::sync::Arc;

use anyhow::{Context, Result, bail};

use super::generators as sfgen;
use super::*;

struct Chunk<'a> {
    id: [u8; 4],
    data: &'a [u8],
}

fn chunks(mut buf: &[u8]) -> Vec<Chunk<'_>> {
    let mut out = Vec::new();
    while buf.len() >= 8 {
        let id = [buf[0], buf[1], buf[2], buf[3]];
        let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        let end = (8 + len).min(buf.len());
        out.push(Chunk { id, data: &buf[8..end] });
        // Chunks are padded to an even size.
        let next = 8 + len + (len & 1);
        if next >= buf.len() {
            break;
        }
        buf = &buf[next..];
    }
    out
}

/// Returns the sub-chunks of a `LIST` chunk with the given list type.
fn list<'a>(top: &[Chunk<'a>], kind: &[u8; 4]) -> Vec<Chunk<'a>> {
    top.iter()
        .find(|c| &c.id == b"LIST" && c.data.len() >= 4 && &c.data[0..4] == kind)
        .map(|c| chunks(&c.data[4..]))
        .unwrap_or_default()
}

fn find<'a>(chunks: &[Chunk<'a>], id: &[u8; 4]) -> Option<&'a [u8]> {
    chunks.iter().find(|c| &c.id == id).map(|c| c.data)
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim_end().to_string()
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn records<'a>(data: Option<&'a [u8]>, size: usize, what: &str) -> Result<Vec<&'a [u8]>> {
    let data = data.with_context(|| format!("missing '{what}' chunk"))?;
    if data.len() % size != 0 {
        bail!("'{what}' chunk has invalid size {}", data.len());
    }
    Ok(data.chunks_exact(size).collect())
}

struct Bag {
    gen_idx: usize,
    module: usize,
}

fn read_bags(data: Option<&[u8]>, what: &str) -> Result<Vec<Bag>> {
    Ok(records(data, 4, what)?
        .into_iter()
        .map(|r| Bag { gen_idx: u16_at(r, 0) as usize, module: u16_at(r, 2) as usize })
        .collect())
}

fn read_gens(data: Option<&[u8]>, what: &str) -> Result<Vec<(u16, u16)>> {
    Ok(records(data, 4, what)?.into_iter().map(|r| (u16_at(r, 0), u16_at(r, 2))).collect())
}

fn read_mods(data: Option<&[u8]>, what: &str) -> Result<Vec<Modulator>> {
    // Some files omit the modulator chunks entirely; treat that as empty.
    let Some(data) = data else { return Ok(Vec::new()) };
    Ok(records(Some(data), 10, what)?
        .into_iter()
        .map(|r| Modulator {
            src: u16_at(r, 0),
            dest: u16_at(r, 2),
            amount: u16_at(r, 4) as i16,
            amt_src: u16_at(r, 6),
            transform: u16_at(r, 8),
        })
        .collect())
}

/// Build the zones for the bag range `[first, last)`. `terminal` is the
/// generator that links a zone to the next level (instrument or sampleID).
fn build_zones(
    bags: &[Bag],
    first: usize,
    last: usize,
    gens: &[(u16, u16)],
    mods: &[Modulator],
    terminal: u16,
    link_count: usize,
) -> (Option<Zone>, Vec<Zone>) {
    let mut global = None;
    let mut zones = Vec::new();
    let last = last.min(bags.len().saturating_sub(1));
    for b in first..last {
        let (g0, g1) = (bags[b].gen_idx, bags[b + 1].gen_idx.min(gens.len()));
        let (m0, m1) = (bags[b].module, bags[b + 1].module.min(mods.len()));
        let mut zone = Zone::default();
        if g0 < g1 {
            for &(id, amt) in &gens[g0..g1] {
                if id == terminal {
                    zone.link = Some(amt as usize);
                    // Generators after the terminal one are ignored by spec.
                    break;
                }
                if (id as usize) < sfgen::GEN_COUNT {
                    zone.set(id, amt);
                }
            }
        }
        if m0 < m1 {
            zone.mods = mods[m0..m1].to_vec();
        }
        match zone.link {
            Some(l) if l < link_count => zones.push(zone),
            Some(_) => {} // dangling reference: drop
            None if b == first => global = Some(zone),
            None => {} // a non-first zone without link is ignored per spec
        }
    }
    (global, zones)
}

pub fn read_sf2(bytes: &[u8]) -> Result<SoundFont> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"sfbk" {
        bail!("not a SoundFont 2 file (missing RIFF/sfbk header)");
    }
    let riff_len = (u32_at(bytes, 4) as usize + 8).min(bytes.len());
    let top = chunks(&bytes[12..riff_len]);

    let info_chunks = list(&top, b"INFO");
    let s = |id: &[u8; 4]| find(&info_chunks, id).map(cstr).unwrap_or_default();
    let ver = |id: &[u8; 4]| {
        find(&info_chunks, id).filter(|d| d.len() >= 4).map(|d| (u16_at(d, 0), u16_at(d, 2)))
    };
    let info = Info {
        version: ver(b"ifil").unwrap_or((2, 1)),
        sound_engine: s(b"isng"),
        name: s(b"INAM"),
        rom_name: s(b"irom"),
        rom_version: ver(b"iver"),
        creation_date: s(b"ICRD"),
        engineers: s(b"IENG"),
        product: s(b"IPRD"),
        copyright: s(b"ICOP"),
        comment: s(b"ICMT"),
        software: s(b"ISFT"),
    };

    let sdta = list(&top, b"sdta");
    let smpl: Vec<i16> = find(&sdta, b"smpl")
        .map(|d| d.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect())
        .unwrap_or_default();

    let pdta = list(&top, b"pdta");
    if pdta.is_empty() {
        bail!("missing 'pdta' list");
    }

    // Samples
    let shdr = records(find(&pdta, b"shdr"), 46, "shdr")?;
    let sample_count = shdr.len().saturating_sub(1);
    let mut samples = Vec::with_capacity(sample_count);
    for r in &shdr[..sample_count] {
        let start = u32_at(r, 20) as usize;
        let end = u32_at(r, 24) as usize;
        let (lo, hi) = (start.min(smpl.len()), end.min(smpl.len()));
        let data = if lo < hi { smpl[lo..hi].to_vec() } else { Vec::new() };
        let len = data.len() as u32;
        let loop_start = (u32_at(r, 28) as usize).saturating_sub(start) as u32;
        let loop_end = (u32_at(r, 32) as usize).saturating_sub(start) as u32;
        let sample_type = u16_at(r, 44);
        let link = u16_at(r, 42) as usize;
        let stereo = sample_type & (sample_type::LEFT | sample_type::RIGHT | sample_type::LINKED) != 0;
        samples.push(Sample {
            name: cstr(&r[0..20]),
            data: Arc::new(data),
            loop_start: loop_start.min(len),
            loop_end: loop_end.min(len),
            sample_rate: u32_at(r, 36),
            original_pitch: r[40].min(127),
            pitch_correction: r[41] as i8,
            link: (stereo && link < sample_count).then_some(link),
            sample_type,
        });
    }

    // Instruments
    let inst = records(find(&pdta, b"inst"), 22, "inst")?;
    let ibag = read_bags(find(&pdta, b"ibag"), "ibag")?;
    let igen = read_gens(find(&pdta, b"igen"), "igen")?;
    let imod = read_mods(find(&pdta, b"imod"), "imod")?;
    let inst_count = inst.len().saturating_sub(1);
    let mut instruments = Vec::with_capacity(inst_count);
    for w in inst.windows(2) {
        let (first, last) = (u16_at(w[0], 20) as usize, u16_at(w[1], 20) as usize);
        let (global, zones) =
            build_zones(&ibag, first, last, &igen, &imod, sfgen::SAMPLE_ID, sample_count);
        instruments.push(Instrument { name: cstr(&w[0][0..20]), global, zones });
    }

    // Presets
    let phdr = records(find(&pdta, b"phdr"), 38, "phdr")?;
    let pbag = read_bags(find(&pdta, b"pbag"), "pbag")?;
    let pgen = read_gens(find(&pdta, b"pgen"), "pgen")?;
    let pmod = read_mods(find(&pdta, b"pmod"), "pmod")?;
    let mut presets = Vec::with_capacity(phdr.len().saturating_sub(1));
    for w in phdr.windows(2) {
        let r = w[0];
        let (first, last) = (u16_at(r, 24) as usize, u16_at(w[1], 24) as usize);
        let (global, zones) =
            build_zones(&pbag, first, last, &pgen, &pmod, sfgen::INSTRUMENT, inst_count);
        presets.push(Preset {
            name: cstr(&r[0..20]),
            program: u16_at(r, 20),
            bank: u16_at(r, 22),
            library: u32_at(r, 26),
            genre: u32_at(r, 30),
            morphology: u32_at(r, 34),
            global,
            zones,
        });
    }

    Ok(SoundFont { info, presets, instruments, samples })
}
