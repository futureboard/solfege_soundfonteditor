//! Small SF2 preview synthesizer: zone resolution + a cpal output stream.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::sf2::generators::{self as sfgen, GEN_COUNT};
use crate::sf2::{SoundFont, Zone};

const MAX_VOICES: usize = 96;
/// Release floor: a voice is finished once its envelope is ~-100 dB.
const SILENCE: f32 = 1e-5;
/// Fade time when a voice is cut by another note in its exclusive class (hi-hat choke).
const CHOKE_SECS: f32 = 0.008;

#[derive(Clone)]
pub struct VoiceParams {
    pub data: Arc<Vec<i16>>,
    pub start: usize,
    pub end: usize,
    pub loop_start: usize,
    pub loop_end: usize,
    /// 0/2 = no loop, 1 = continuous loop, 3 = loop until release.
    pub mode: u8,
    /// Sample rate of the source data and pitch ratio relative to it.
    pub sample_rate: u32,
    pub pitch_ratio: f64,
    pub gain: f32,
    /// -1 = left, 1 = right.
    pub pan: f32,
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub filter_fc: f32,
    pub filter_q: f32,
    pub exclusive_class: u8,
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Done,
}

#[derive(Default, Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    enabled: bool,
}

impl Biquad {
    fn lowpass(fc: f32, q: f32, sr: f32) -> Self {
        if fc >= sr * 0.45 || fc >= 19_000.0 {
            return Self::default();
        }
        let w0 = 2.0 * std::f32::consts::PI * fc / sr;
        let alpha = w0.sin() / (2.0 * q);
        let cos = w0.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - cos) / 2.0 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            enabled: true,
            ..Default::default()
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        if !self.enabled {
            return x;
        }
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

struct Voice {
    p: VoiceParams,
    key: u8,
    id: u64,
    pos: f64,
    step: f64,
    stage: Stage,
    stage_time: f32,
    level: f32,
    decay_mul: f32,
    release_mul: f32,
    choke_mul: f32,
    released: bool,
    filter: Biquad,
    gain_l: f32,
    gain_r: f32,
}

impl Voice {
    fn new(p: VoiceParams, key: u8, id: u64, out_rate: f32) -> Self {
        let step = p.pitch_ratio * p.sample_rate as f64 / out_rate as f64;
        // Exponential segments that fall 100 dB over the given time.
        let mul = |secs: f32| (SILENCE.ln() / (secs.max(0.001) * out_rate)).exp();
        let angle = (p.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
        let filter = Biquad::lowpass(p.filter_fc, p.filter_q, out_rate);
        Self {
            key,
            id,
            pos: p.start as f64,
            step,
            stage: Stage::Delay,
            stage_time: 0.0,
            level: 0.0,
            decay_mul: mul(p.decay),
            release_mul: mul(p.release),
            choke_mul: mul(CHOKE_SECS),
            released: false,
            filter,
            gain_l: angle.cos() * p.gain,
            gain_r: angle.sin() * p.gain,
            p,
        }
    }

    /// Fast fade-out, used when another note of the same exclusive class starts.
    fn choke(&mut self) {
        self.released = true;
        self.release_mul = self.release_mul.min(self.choke_mul);
        if self.stage != Stage::Done {
            self.stage = Stage::Release;
        }
    }

    fn release(&mut self) {
        if !self.released {
            self.released = true;
            if self.stage != Stage::Done {
                self.stage = Stage::Release;
            }
        }
    }

    fn envelope(&mut self, dt: f32) -> f32 {
        self.stage_time += dt;
        loop {
            match self.stage {
                Stage::Delay if self.stage_time >= self.p.delay => self.next(Stage::Attack),
                Stage::Attack => {
                    if self.stage_time >= self.p.attack {
                        self.level = 1.0;
                        self.next(Stage::Hold);
                    } else {
                        self.level = self.stage_time / self.p.attack.max(1e-4);
                        break;
                    }
                }
                Stage::Hold if self.stage_time >= self.p.hold => self.next(Stage::Decay),
                Stage::Decay => {
                    self.level *= self.decay_mul;
                    if self.level <= self.p.sustain {
                        self.level = self.p.sustain;
                        self.stage = Stage::Sustain;
                    }
                    break;
                }
                Stage::Release => {
                    self.level *= self.release_mul;
                    if self.level <= SILENCE {
                        self.stage = Stage::Done;
                    }
                    break;
                }
                _ => break,
            }
        }
        if self.stage == Stage::Sustain && self.p.sustain <= SILENCE {
            self.stage = Stage::Done;
        }
        self.level
    }

    fn next(&mut self, s: Stage) {
        self.stage = s;
        self.stage_time = 0.0;
    }

    /// Render one sample; returns false when the voice has finished.
    fn render(&mut self, dt: f32) -> Option<(f32, f32)> {
        if self.stage == Stage::Done {
            return None;
        }
        let env = self.envelope(dt);
        let data = &self.p.data;
        let looping = match self.p.mode {
            1 => true,
            3 => !self.released,
            _ => false,
        } && self.p.loop_end > self.p.loop_start + 1;
        if looping && self.pos >= self.p.loop_end as f64 {
            let len = (self.p.loop_end - self.p.loop_start) as f64;
            self.pos = self.p.loop_start as f64 + (self.pos - self.p.loop_end as f64) % len;
        }
        let i = self.pos as usize;
        if i + 1 >= self.p.end.min(data.len()) {
            self.stage = Stage::Done;
            return None;
        }
        let frac = (self.pos - i as f64) as f32;
        let next_i = if looping && i + 1 >= self.p.loop_end { self.p.loop_start } else { i + 1 };
        let s0 = data[i] as f32;
        let s1 = data[next_i.min(data.len() - 1)] as f32;
        let s = (s0 + (s1 - s0) * frac) / 32768.0;
        self.pos += self.step;
        let v = self.filter.process(s) * env;
        Some((v * self.gain_l, v * self.gain_r))
    }
}

pub struct Mixer {
    voices: Vec<Voice>,
    sample_rate: f32,
    next_id: u64,
    pub master: f32,
}

impl Mixer {
    fn new(sample_rate: f32) -> Self {
        Self { voices: Vec::new(), sample_rate, next_id: 0, master: 0.8 }
    }

    pub fn note_on(&mut self, key: u8, params: Vec<VoiceParams>) {
        for p in &params {
            if p.exclusive_class != 0 {
                for v in &mut self.voices {
                    if v.p.exclusive_class == p.exclusive_class {
                        v.choke();
                    }
                }
            }
        }
        for p in params {
            if self.voices.len() >= MAX_VOICES {
                // Steal the oldest voice.
                if let Some((i, _)) = self.voices.iter().enumerate().min_by_key(|(_, v)| v.id) {
                    self.voices.swap_remove(i);
                }
            }
            self.next_id += 1;
            self.voices.push(Voice::new(p, key, self.next_id, self.sample_rate));
        }
    }

    pub fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.key == key) {
            v.release();
        }
    }

    pub fn all_off(&mut self) {
        self.voices.clear();
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    /// Current playback positions of voices that read from `data`.
    pub fn playheads(&self, data: &Arc<Vec<i16>>) -> Vec<f64> {
        self.voices.iter().filter(|v| Arc::ptr_eq(&v.p.data, data)).map(|v| v.pos).collect()
    }

    fn render(&mut self, out: &mut [f32], channels: usize) {
        let dt = 1.0 / self.sample_rate;
        out.fill(0.0);
        for frame in out.chunks_mut(channels) {
            let (mut l, mut r) = (0.0, 0.0);
            for v in &mut self.voices {
                if let Some((a, b)) = v.render(dt) {
                    l += a;
                    r += b;
                }
            }
            l = (l * self.master).clamp(-1.0, 1.0);
            r = (r * self.master).clamp(-1.0, 1.0);
            match channels {
                1 => frame[0] = (l + r) * 0.5,
                _ => {
                    frame[0] = l;
                    frame[1] = r;
                }
            }
        }
        self.voices.retain(|v| v.stage != Stage::Done);
    }
}

pub struct Audio {
    _stream: Option<cpal::Stream>,
    pub mixer: Arc<Mutex<Mixer>>,
    pub sample_rate: u32,
    pub error: Option<String>,
    pub device_name: String,
}

impl Audio {
    pub fn new() -> Self {
        match Self::open() {
            Ok(a) => a,
            Err(e) => Self {
                _stream: None,
                mixer: Arc::new(Mutex::new(Mixer::new(44100.0))),
                sample_rate: 44100,
                error: Some(e.to_string()),
                device_name: "none".into(),
            },
        }
    }

    fn open() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device =
            host.default_output_device().ok_or_else(|| anyhow::anyhow!("no audio output device"))?;
        let device_name =
            device.description().map(|d| d.to_string()).unwrap_or_else(|_| "default".into());
        let supported = device.default_output_config()?;
        let config: cpal::StreamConfig = supported.config();
        let sample_rate = config.sample_rate;
        let mixer = Arc::new(Mutex::new(Mixer::new(sample_rate as f32)));
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, config, mixer.clone())?,
            cpal::SampleFormat::I16 => build::<i16>(&device, config, mixer.clone())?,
            cpal::SampleFormat::I32 => build::<i32>(&device, config, mixer.clone())?,
            cpal::SampleFormat::U16 => build::<u16>(&device, config, mixer.clone())?,
            other => anyhow::bail!("unsupported sample format {other:?}"),
        };
        stream.play()?;
        Ok(Self { _stream: Some(stream), mixer, sample_rate, error: None, device_name })
    }

    pub fn note_on(&self, key: u8, params: Vec<VoiceParams>) {
        if let Ok(mut m) = self.mixer.lock() {
            m.note_on(key, params);
        }
    }

    pub fn note_off(&self, key: u8) {
        if let Ok(mut m) = self.mixer.lock() {
            m.note_off(key);
        }
    }

    pub fn all_off(&self) {
        if let Ok(mut m) = self.mixer.lock() {
            m.all_off();
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
) -> anyhow::Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut scratch: Vec<f32> = Vec::new();
    let stream = device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            scratch.resize(data.len(), 0.0);
            match mixer.lock() {
                Ok(mut m) => m.render(&mut scratch, channels),
                Err(_) => scratch.fill(0.0),
            }
            for (o, s) in data.iter_mut().zip(&scratch) {
                *o = T::from_sample(*s);
            }
        },
        |e| eprintln!("audio stream error: {e}"),
        None,
    )?;
    Ok(stream)
}

// ---------------------------------------------------------------------------
// Zone resolution (SF2 section 9.4)
// ---------------------------------------------------------------------------

type Gens = [i32; GEN_COUNT];

fn apply(gens: &mut Gens, z: &Zone) {
    for &(id, _) in &z.gens {
        if (id as usize) < GEN_COUNT {
            gens[id as usize] = z.get_i(id).unwrap_or(0);
        }
    }
}

fn in_range(z: &Zone, global: Option<&Zone>, key: u8, vel: u8) -> bool {
    let kr = z.range(sfgen::KEY_RANGE).or_else(|| global.and_then(|g| g.range(sfgen::KEY_RANGE)));
    let vr = z.range(sfgen::VEL_RANGE).or_else(|| global.and_then(|g| g.range(sfgen::VEL_RANGE)));
    let (klo, khi) = kr.unwrap_or((0, 127));
    let (vlo, vhi) = vr.unwrap_or((0, 127));
    (klo..=khi).contains(&key) && (vlo..=vhi).contains(&vel)
}

/// Voices for an instrument, optionally layered with preset-level offsets.
pub fn instrument_voices(sf: &SoundFont, inst: usize, key: u8, vel: u8, preset: Option<&Gens>) -> Vec<VoiceParams> {
    let Some(instrument) = sf.instruments.get(inst) else { return Vec::new() };
    let mut out = Vec::new();
    for z in &instrument.zones {
        if !in_range(z, instrument.global.as_ref(), key, vel) {
            continue;
        }
        let Some(sample_idx) = z.link else { continue };
        let mut g: Gens = std::array::from_fn(|i| sfgen::default_value(i as u16));
        if let Some(gz) = &instrument.global {
            apply(&mut g, gz);
        }
        apply(&mut g, z);
        if let Some(p) = preset {
            for (i, v) in p.iter().enumerate() {
                let id = i as u16;
                let skip = sfgen::info(id).is_none_or(|info| info.inst_only)
                    || matches!(id, sfgen::KEY_RANGE | sfgen::VEL_RANGE);
                if !skip {
                    g[i] += v;
                }
            }
        }
        if let Some(v) = voice_from_gens(sf, sample_idx, &g, key, vel) {
            out.push(v);
        }
    }
    out
}

pub fn preset_voices(sf: &SoundFont, preset: usize, key: u8, vel: u8) -> Vec<VoiceParams> {
    let Some(p) = sf.presets.get(preset) else { return Vec::new() };
    let mut out = Vec::new();
    for z in &p.zones {
        if !in_range(z, p.global.as_ref(), key, vel) {
            continue;
        }
        let Some(inst) = z.link else { continue };
        let mut g: Gens = [0; GEN_COUNT];
        if let Some(gz) = &p.global {
            apply(&mut g, gz);
        }
        apply(&mut g, z);
        out.extend(instrument_voices(sf, inst, key, vel, Some(&g)));
    }
    out
}

/// Play a raw sample: pitched around its root key, looping if it has a loop.
pub fn sample_voice(sf: &SoundFont, sample: usize, key: u8, vel: u8, looped: bool) -> Vec<VoiceParams> {
    let mut g: Gens = std::array::from_fn(|i| sfgen::default_value(i as u16));
    g[sfgen::SAMPLE_MODES as usize] = if looped { 1 } else { 0 };
    g[sfgen::RELEASE_VOL_ENV as usize] = -2400;
    voice_from_gens(sf, sample, &g, key, vel).into_iter().collect()
}

fn voice_from_gens(sf: &SoundFont, sample_idx: usize, g: &Gens, key: u8, vel: u8) -> Option<VoiceParams> {
    let s = sf.samples.get(sample_idx)?;
    let len = s.data.len() as i64;
    if len < 2 || s.sample_rate == 0 {
        return None;
    }
    let off = |fine: u16, coarse: u16| g[fine as usize] as i64 + g[coarse as usize] as i64 * 32768;
    let clamp = |v: i64| v.clamp(0, len) as usize;
    let start = clamp(off(sfgen::START_ADDRS_OFFSET, sfgen::START_ADDRS_COARSE_OFFSET));
    let end = clamp(len + off(sfgen::END_ADDRS_OFFSET, sfgen::END_ADDRS_COARSE_OFFSET));
    let loop_start = clamp(s.loop_start as i64 + off(sfgen::STARTLOOP_ADDRS_OFFSET, sfgen::STARTLOOP_ADDRS_COARSE_OFFSET));
    let loop_end = clamp(s.loop_end as i64 + off(sfgen::ENDLOOP_ADDRS_OFFSET, sfgen::ENDLOOP_ADDRS_COARSE_OFFSET));
    if start + 1 >= end {
        return None;
    }

    let key = if g[sfgen::KEYNUM as usize] >= 0 { g[sfgen::KEYNUM as usize] as u8 } else { key };
    let vel = if g[sfgen::VELOCITY as usize] >= 0 { g[sfgen::VELOCITY as usize] as u8 } else { vel };
    let root = if g[sfgen::OVERRIDING_ROOT_KEY as usize] >= 0 {
        g[sfgen::OVERRIDING_ROOT_KEY as usize]
    } else {
        s.original_pitch as i32
    };
    let cents = (key as i32 - root) * g[sfgen::SCALE_TUNING as usize]
        + g[sfgen::COARSE_TUNE as usize] * 100
        + g[sfgen::FINE_TUNE as usize]
        + s.pitch_correction as i32;

    // EMU-style attenuation (0.4 factor, as most players do) plus a squared velocity curve.
    let atten_db = g[sfgen::INITIAL_ATTENUATION as usize].max(0) as f32 * 0.04;
    let vel_gain = (vel as f32 / 127.0).powi(2);
    let gain = 10f32.powf(-atten_db / 20.0) * vel_gain * 0.5;

    let tc = |id: u16| sfgen::timecents_to_secs(g[id as usize] as f64) as f32;
    let key_scale = |id: u16| 2f32.powf((60 - key as i32) as f32 * g[id as usize] as f32 / 1200.0);
    let sustain_cb = g[sfgen::SUSTAIN_VOL_ENV as usize].clamp(0, 1440) as f32;

    let fc = g[sfgen::INITIAL_FILTER_FC as usize].clamp(1500, 13500);
    Some(VoiceParams {
        data: s.data.clone(),
        start,
        end,
        loop_start,
        loop_end,
        mode: (g[sfgen::SAMPLE_MODES as usize] & 3) as u8,
        sample_rate: s.sample_rate,
        pitch_ratio: 2f64.powf(cents as f64 / 1200.0),
        gain,
        pan: g[sfgen::PAN as usize].clamp(-500, 500) as f32 / 500.0,
        delay: tc(sfgen::DELAY_VOL_ENV),
        attack: tc(sfgen::ATTACK_VOL_ENV),
        hold: tc(sfgen::HOLD_VOL_ENV) * key_scale(sfgen::KEYNUM_TO_VOL_ENV_HOLD),
        decay: tc(sfgen::DECAY_VOL_ENV) * key_scale(sfgen::KEYNUM_TO_VOL_ENV_DECAY),
        sustain: 10f32.powf(-sustain_cb / 200.0),
        release: tc(sfgen::RELEASE_VOL_ENV),
        filter_fc: if fc >= 13500 { f32::MAX } else { sfgen::abscents_to_hz(fc as f64) as f32 },
        filter_q: 10f32.powf(g[sfgen::INITIAL_FILTER_Q as usize].clamp(0, 960) as f32 / 200.0).max(0.707),
        exclusive_class: g[sfgen::EXCLUSIVE_CLASS as usize].clamp(0, 127) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hat(class: u8) -> VoiceParams {
        VoiceParams {
            data: Arc::new(vec![10_000; 48_000]),
            start: 0,
            end: 48_000,
            loop_start: 0,
            loop_end: 0,
            mode: 0,
            sample_rate: 48_000,
            pitch_ratio: 1.0,
            gain: 1.0,
            pan: 0.0,
            delay: 0.0,
            attack: 0.0,
            hold: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 1.0,
            filter_fc: f32::MAX,
            filter_q: 0.707,
            exclusive_class: class,
        }
    }

    #[test]
    fn exclusive_class_chokes_quickly() {
        let mut m = Mixer::new(48_000.0);
        let mut buf = vec![0.0; 2 * 480];
        m.note_on(46, vec![hat(1)]); // open hat
        m.render(&mut buf, 2);
        m.note_on(42, vec![hat(1)]); // closed hat chokes it
        m.note_on(36, vec![hat(0)]); // kick (no class) is untouched
        assert_eq!(m.active_voices(), 3);
        // 20 ms later only the open hat is gone.
        for _ in 0..2 {
            m.render(&mut buf, 2);
        }
        let mut keys: Vec<u8> = m.voices.iter().map(|v| v.key).collect();
        keys.sort();
        assert_eq!(keys, vec![36, 42]);
    }
}
