use anyhow::{Result, bail};

use super::generators as sfgen;
use super::*;

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn name20(&mut self, s: &str) {
        let mut b = [0u8; 20];
        let ascii: Vec<u8> = s.bytes().map(|c| if c.is_ascii() { c } else { b'?' }).take(20).collect();
        b[..ascii.len()].copy_from_slice(&ascii);
        self.0.extend_from_slice(&b);
    }
    fn chunk(&mut self, id: &[u8; 4], data: &[u8]) {
        self.0.extend_from_slice(id);
        self.u32(data.len() as u32);
        self.0.extend_from_slice(data);
        if data.len() & 1 == 1 {
            self.0.push(0);
        }
    }
    fn list(&mut self, kind: &[u8; 4], body: &[u8]) {
        let mut d = Vec::with_capacity(body.len() + 4);
        d.extend_from_slice(kind);
        d.extend_from_slice(body);
        self.chunk(b"LIST", &d);
    }
}

fn zstr(s: &str) -> Vec<u8> {
    // INFO strings: zero terminated, padded to an even length.
    let mut v: Vec<u8> = s.bytes().take(255).collect();
    v.push(0);
    if v.len() & 1 == 1 {
        v.push(0);
    }
    v
}

fn idx16(v: usize, what: &str) -> Result<u16> {
    if v > u16::MAX as usize {
        bail!("too many {what} for the SF2 format ({v} > 65535)");
    }
    Ok(v as u16)
}

/// Serialized bag/gen/mod tables for one hierarchy level.
#[derive(Default)]
struct Tables {
    bag: Out,
    gens: Out,
    module: Out,
    gen_count: usize,
    mod_count: usize,
    bag_count: usize,
}

impl Tables {
    fn zone(&mut self, z: &Zone, terminal: u16, link: Option<usize>) -> Result<()> {
        self.bag.u16(idx16(self.gen_count, "generators")?);
        self.bag.u16(idx16(self.mod_count, "modulators")?);
        self.bag_count += 1;
        for (id, amt) in z.ordered_gens() {
            self.gens.u16(id);
            self.gens.u16(amt);
            self.gen_count += 1;
        }
        if let Some(l) = link {
            self.gens.u16(terminal);
            self.gens.u16(idx16(l, "links")?);
            self.gen_count += 1;
        }
        for m in &z.mods {
            self.module.u16(m.src);
            self.module.u16(m.dest);
            self.module.u16(m.amount as u16);
            self.module.u16(m.amt_src);
            self.module.u16(m.transform);
            self.mod_count += 1;
        }
        Ok(())
    }

    fn zones(&mut self, global: &Option<Zone>, zones: &[Zone], terminal: u16) -> Result<()> {
        if let Some(g) = global {
            self.zone(g, terminal, None)?;
        }
        for z in zones {
            self.zone(z, terminal, z.link)?;
        }
        Ok(())
    }

    fn terminate(&mut self) -> Result<()> {
        self.bag.u16(idx16(self.gen_count, "generators")?);
        self.bag.u16(idx16(self.mod_count, "modulators")?);
        self.gens.u32(0);
        self.module.0.extend_from_slice(&[0; 10]);
        Ok(())
    }
}

pub fn write_sf2(sf: &SoundFont) -> Result<Vec<u8>> {
    // INFO
    let mut info = Out(Vec::new());
    let mut ifil = Out(Vec::new());
    ifil.u16(sf.info.version.0.max(2));
    ifil.u16(sf.info.version.1);
    info.chunk(b"ifil", &ifil.0);
    let engine = if sf.info.sound_engine.is_empty() { "EMU8000" } else { &sf.info.sound_engine };
    info.chunk(b"isng", &zstr(engine));
    let name = if sf.info.name.is_empty() { "Untitled" } else { &sf.info.name };
    info.chunk(b"INAM", &zstr(name));
    if !sf.info.rom_name.is_empty() {
        info.chunk(b"irom", &zstr(&sf.info.rom_name));
    }
    if let Some((a, b)) = sf.info.rom_version {
        let mut v = Out(Vec::new());
        v.u16(a);
        v.u16(b);
        info.chunk(b"iver", &v.0);
    }
    for (id, s) in [
        (b"ICRD", &sf.info.creation_date),
        (b"IENG", &sf.info.engineers),
        (b"IPRD", &sf.info.product),
        (b"ICOP", &sf.info.copyright),
        (b"ICMT", &sf.info.comment),
        (b"ISFT", &sf.info.software),
    ] {
        if !s.is_empty() {
            info.chunk(id, &zstr(s));
        }
    }

    // sdta: every sample is followed by 46 zero points, as required by the spec.
    let mut smpl = Out(Vec::new());
    let mut shdr = Out(Vec::new());
    let mut pos: u32 = 0;
    for s in &sf.samples {
        for &v in s.data.iter() {
            smpl.0.extend_from_slice(&v.to_le_bytes());
        }
        smpl.0.extend_from_slice(&[0u8; 46 * 2]);
        let len = s.data.len() as u32;
        shdr.name20(&s.name);
        shdr.u32(pos);
        shdr.u32(pos + len);
        shdr.u32(pos + s.loop_start.min(len));
        shdr.u32(pos + s.loop_end.min(len));
        shdr.u32(s.sample_rate);
        shdr.u8(s.original_pitch.min(127));
        shdr.u8(s.pitch_correction as u8);
        shdr.u16(s.link.map(|l| l as u16).unwrap_or(0));
        shdr.u16(s.sample_type);
        pos = pos
            .checked_add(len + 46)
            .ok_or_else(|| anyhow::anyhow!("sample data exceeds 4 GiB"))?;
    }
    shdr.name20("EOS");
    shdr.0.extend_from_slice(&[0u8; 26]);

    // Instruments
    let mut inst = Out(Vec::new());
    let mut it = Tables::default();
    for i in &sf.instruments {
        inst.name20(&i.name);
        inst.u16(idx16(it.bag_count, "instrument zones")?);
        it.zones(&i.global, &i.zones, sfgen::SAMPLE_ID)?;
    }
    inst.name20("EOI");
    inst.u16(idx16(it.bag_count, "instrument zones")?);
    it.terminate()?;

    // Presets
    let mut phdr = Out(Vec::new());
    let mut pt = Tables::default();
    for p in &sf.presets {
        phdr.name20(&p.name);
        phdr.u16(p.program);
        phdr.u16(p.bank);
        phdr.u16(idx16(pt.bag_count, "preset zones")?);
        phdr.u32(p.library);
        phdr.u32(p.genre);
        phdr.u32(p.morphology);
        pt.zones(&p.global, &p.zones, sfgen::INSTRUMENT)?;
    }
    phdr.name20("EOP");
    phdr.u16(0);
    phdr.u16(0);
    phdr.u16(idx16(pt.bag_count, "preset zones")?);
    phdr.0.extend_from_slice(&[0u8; 12]);
    pt.terminate()?;

    let mut sdta = Out(Vec::new());
    sdta.chunk(b"smpl", &smpl.0);

    let mut pdta = Out(Vec::new());
    pdta.chunk(b"phdr", &phdr.0);
    pdta.chunk(b"pbag", &pt.bag.0);
    pdta.chunk(b"pmod", &pt.module.0);
    pdta.chunk(b"pgen", &pt.gens.0);
    pdta.chunk(b"inst", &inst.0);
    pdta.chunk(b"ibag", &it.bag.0);
    pdta.chunk(b"imod", &it.module.0);
    pdta.chunk(b"igen", &it.gens.0);
    pdta.chunk(b"shdr", &shdr.0);

    let mut body = Out(Vec::new());
    body.0.extend_from_slice(b"sfbk");
    body.list(b"INFO", &info.0);
    body.list(b"sdta", &sdta.0);
    body.list(b"pdta", &pdta.0);

    let mut out = Out(Vec::with_capacity(body.0.len() + 8));
    out.chunk(b"RIFF", &body.0);
    Ok(out.0)
}
