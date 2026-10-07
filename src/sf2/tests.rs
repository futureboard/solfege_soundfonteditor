use std::sync::Arc;

use super::generators as sfgen;
use super::*;

fn sample_font() -> SoundFont {
    let mut sf = SoundFont::new_empty();
    sf.info.name = "Test Font".into();
    sf.info.copyright = "CC0".into();
    let wave: Vec<i16> = (0..1000).map(|i| ((i as f32 * 0.1).sin() * 20000.0) as i16).collect();
    sf.samples.push(Sample {
        name: "Sine L".into(),
        data: Arc::new(wave.clone()),
        loop_start: 100,
        loop_end: 900,
        sample_rate: 44100,
        original_pitch: 69,
        pitch_correction: -3,
        link: Some(1),
        sample_type: sample_type::LEFT,
    });
    sf.samples.push(Sample {
        name: "Sine R".into(),
        data: Arc::new(wave),
        loop_start: 10,
        loop_end: 20,
        sample_rate: 22050,
        original_pitch: 60,
        pitch_correction: 5,
        link: Some(0),
        sample_type: sample_type::RIGHT,
    });

    let mut global = Zone::default();
    global.set_i(sfgen::ATTACK_VOL_ENV, -1200);
    let mut z1 = Zone { link: Some(0), ..Default::default() };
    z1.set_i(sfgen::PAN, -500);
    z1.set_range(sfgen::KEY_RANGE, 0, 64);
    z1.set_i(sfgen::SAMPLE_MODES, 1);
    z1.mods.push(Modulator { src: 0x0081, dest: 6, amount: 50, amt_src: 0, transform: 0 });
    let mut z2 = Zone { link: Some(1), ..Default::default() };
    z2.set_i(sfgen::PAN, 500);
    z2.set_range(sfgen::VEL_RANGE, 10, 100);
    sf.instruments.push(Instrument { name: "Sine".into(), global: Some(global), zones: vec![z1, z2] });
    sf.instruments.push(Instrument { name: "Empty".into(), ..Default::default() });

    let mut pz = Zone { link: Some(0), ..Default::default() };
    pz.set_i(sfgen::COARSE_TUNE, 12);
    sf.presets.push(Preset { name: "Sine Lead".into(), program: 5, bank: 1, zones: vec![pz], ..Default::default() });
    sf.presets.push(Preset { name: "Other".into(), program: 0, bank: 0, ..Default::default() });
    sf
}

#[test]
fn roundtrip() {
    let sf = sample_font();
    let bytes = write_sf2(&sf).unwrap();
    let back = read_sf2(&bytes).unwrap();

    assert_eq!(back.info.name, "Test Font");
    assert_eq!(back.info.copyright, "CC0");
    assert_eq!(back.samples.len(), 2);
    for (a, b) in sf.samples.iter().zip(&back.samples) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.data, b.data);
        assert_eq!((a.loop_start, a.loop_end), (b.loop_start, b.loop_end));
        assert_eq!(a.sample_rate, b.sample_rate);
        assert_eq!(a.original_pitch, b.original_pitch);
        assert_eq!(a.pitch_correction, b.pitch_correction);
        assert_eq!(a.link, b.link);
        assert_eq!(a.sample_type, b.sample_type);
    }

    assert_eq!(back.instruments.len(), 2);
    let i = &back.instruments[0];
    assert_eq!(i.global.as_ref().unwrap().get_i(sfgen::ATTACK_VOL_ENV), Some(-1200));
    assert_eq!(i.zones.len(), 2);
    assert_eq!(i.zones[0].link, Some(0));
    assert_eq!(i.zones[0].key_range(), (0, 64));
    assert_eq!(i.zones[0].get_i(sfgen::PAN), Some(-500));
    assert_eq!(i.zones[0].mods.len(), 1);
    assert_eq!(i.zones[1].vel_range(), (10, 100));
    // keyRange must be the first generator in a zone.
    assert_eq!(i.zones[0].gens[0].0, sfgen::KEY_RANGE);
    assert!(back.instruments[1].zones.is_empty());

    assert_eq!(back.presets.len(), 2);
    assert_eq!((back.presets[0].bank, back.presets[0].program), (1, 5));
    assert_eq!(back.presets[0].zones[0].get_i(sfgen::COARSE_TUNE), Some(12));

    // Writing the re-read font gives identical bytes.
    assert_eq!(write_sf2(&back).unwrap(), bytes);
}

#[test]
fn remove_sample_fixes_references() {
    let mut sf = sample_font();
    sf.remove_sample(0);
    assert_eq!(sf.samples.len(), 1);
    assert_eq!(sf.samples[0].link, None);
    assert_eq!(sf.instruments[0].zones.len(), 1);
    assert_eq!(sf.instruments[0].zones[0].link, Some(0));
}

#[test]
fn remove_instrument_fixes_references() {
    let mut sf = sample_font();
    sf.remove_instrument(0);
    assert!(sf.presets[0].zones.is_empty());
}

#[test]
fn rejects_garbage() {
    assert!(read_sf2(b"not a soundfont").is_err());
}

#[test]
fn validate_flags_problems() {
    let mut sf = sample_font();
    sf.presets[1].bank = 1;
    sf.presets[1].program = 5;
    let problems = sf.validate();
    assert!(problems.iter().any(|p| p.contains("share bank")));
    assert!(problems.iter().any(|p| p.contains("\"Empty\" has no zones")));
}

#[test]
fn preset_resolution_produces_voices() {
    let sf = sample_font();
    // Key 40 is in zone 1 (0..64) for all velocities, zone 2 only for vel 10..100.
    assert_eq!(crate::audio::preset_voices(&sf, 0, 40, 50).len(), 2);
    assert_eq!(crate::audio::preset_voices(&sf, 0, 40, 120).len(), 1);
    assert_eq!(crate::audio::preset_voices(&sf, 0, 100, 120).len(), 0);
}

/// Writes the test font to `$SF2_OUT` so it can be checked with an external player.
#[test]
fn export_for_external_check() {
    if let Ok(path) = std::env::var("SF2_OUT") {
        std::fs::write(path, write_sf2(&sample_font()).unwrap()).unwrap();
    }
}
