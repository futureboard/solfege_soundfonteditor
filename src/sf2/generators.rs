//! SoundFont 2.04 generator table (section 8.1.2 / 8.1.3 of the spec).

/// How a generator's 16-bit amount is interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenKind {
    /// Signed 16-bit value.
    Signed,
    /// Unsigned 16-bit value (sampleModes, etc).
    Unsigned,
    /// Low byte = lo, high byte = hi.
    Range,
}

/// Unit used to display a human-readable value next to the raw amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenUnit {
    None,
    Samples,
    Cents,
    AbsCents,
    Centibels,
    Permille,
    Timecents,
    Semitones,
    Keys,
    CentsPerKey,
    Mode,
}

#[derive(Clone, Copy, Debug)]
pub struct GenInfo {
    pub id: u16,
    pub name: &'static str,
    pub kind: GenKind,
    pub unit: GenUnit,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    /// Only valid at instrument level.
    pub inst_only: bool,
    pub group: &'static str,
}

pub const START_ADDRS_OFFSET: u16 = 0;
pub const END_ADDRS_OFFSET: u16 = 1;
pub const STARTLOOP_ADDRS_OFFSET: u16 = 2;
pub const ENDLOOP_ADDRS_OFFSET: u16 = 3;
pub const START_ADDRS_COARSE_OFFSET: u16 = 4;
pub const INITIAL_FILTER_FC: u16 = 8;
pub const INITIAL_FILTER_Q: u16 = 9;
pub const END_ADDRS_COARSE_OFFSET: u16 = 12;
pub const PAN: u16 = 17;
pub const DELAY_VOL_ENV: u16 = 33;
pub const ATTACK_VOL_ENV: u16 = 34;
pub const HOLD_VOL_ENV: u16 = 35;
pub const DECAY_VOL_ENV: u16 = 36;
pub const SUSTAIN_VOL_ENV: u16 = 37;
pub const RELEASE_VOL_ENV: u16 = 38;
pub const KEYNUM_TO_VOL_ENV_HOLD: u16 = 39;
pub const KEYNUM_TO_VOL_ENV_DECAY: u16 = 40;
pub const INSTRUMENT: u16 = 41;
pub const KEY_RANGE: u16 = 43;
pub const VEL_RANGE: u16 = 44;
pub const STARTLOOP_ADDRS_COARSE_OFFSET: u16 = 45;
pub const KEYNUM: u16 = 46;
pub const VELOCITY: u16 = 47;
pub const INITIAL_ATTENUATION: u16 = 48;
pub const ENDLOOP_ADDRS_COARSE_OFFSET: u16 = 50;
pub const COARSE_TUNE: u16 = 51;
pub const FINE_TUNE: u16 = 52;
pub const SAMPLE_ID: u16 = 53;
pub const SAMPLE_MODES: u16 = 54;
pub const SCALE_TUNING: u16 = 56;
pub const EXCLUSIVE_CLASS: u16 = 57;
pub const OVERRIDING_ROOT_KEY: u16 = 58;

pub const GEN_COUNT: usize = 61;

macro_rules! g {
    ($id:expr, $name:expr, $kind:ident, $unit:ident, $min:expr, $max:expr, $def:expr, $inst:expr, $group:expr) => {
        GenInfo {
            id: $id,
            name: $name,
            kind: GenKind::$kind,
            unit: GenUnit::$unit,
            min: $min,
            max: $max,
            default: $def,
            inst_only: $inst,
            group: $group,
        }
    };
}

/// Every generator that is meaningful to edit (unused/reserved ids omitted).
pub const GENERATORS: &[GenInfo] = &[
    g!(43, "Key Range", Range, Keys, 0, 127, 0x7F00, false, "Range"),
    g!(44, "Velocity Range", Range, None, 0, 127, 0x7F00, false, "Range"),
    g!(46, "Fixed Key", Signed, Keys, -1, 127, -1, true, "Range"),
    g!(47, "Fixed Velocity", Signed, None, -1, 127, -1, true, "Range"),
    g!(58, "Root Key Override", Signed, Keys, -1, 127, -1, true, "Sample"),
    g!(54, "Sample Mode", Unsigned, Mode, 0, 3, 0, true, "Sample"),
    g!(57, "Exclusive Class", Signed, None, 0, 127, 0, true, "Sample"),
    g!(0, "Start Offset", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(4, "Start Offset (×32768)", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(1, "End Offset", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(12, "End Offset (×32768)", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(2, "Loop Start Offset", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(45, "Loop Start Offset (×32768)", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(3, "Loop End Offset", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(50, "Loop End Offset (×32768)", Signed, Samples, -32768, 32767, 0, true, "Sample"),
    g!(48, "Attenuation", Signed, Centibels, 0, 1440, 0, false, "Pitch & Level"),
    g!(17, "Pan", Signed, Permille, -500, 500, 0, false, "Pitch & Level"),
    g!(51, "Coarse Tune", Signed, Semitones, -120, 120, 0, false, "Pitch & Level"),
    g!(52, "Fine Tune", Signed, Cents, -99, 99, 0, false, "Pitch & Level"),
    g!(56, "Scale Tuning", Signed, CentsPerKey, 0, 1200, 100, false, "Pitch & Level"),
    g!(33, "Vol Env Delay", Signed, Timecents, -12000, 5000, -12000, false, "Volume Envelope"),
    g!(34, "Vol Env Attack", Signed, Timecents, -12000, 8000, -12000, false, "Volume Envelope"),
    g!(35, "Vol Env Hold", Signed, Timecents, -12000, 5000, -12000, false, "Volume Envelope"),
    g!(36, "Vol Env Decay", Signed, Timecents, -12000, 8000, -12000, false, "Volume Envelope"),
    g!(37, "Vol Env Sustain", Signed, Centibels, 0, 1440, 0, false, "Volume Envelope"),
    g!(38, "Vol Env Release", Signed, Timecents, -12000, 8000, -12000, false, "Volume Envelope"),
    g!(39, "Key → Vol Env Hold", Signed, CentsPerKey, -1200, 1200, 0, false, "Volume Envelope"),
    g!(40, "Key → Vol Env Decay", Signed, CentsPerKey, -1200, 1200, 0, false, "Volume Envelope"),
    g!(25, "Mod Env Delay", Signed, Timecents, -12000, 5000, -12000, false, "Modulation Envelope"),
    g!(26, "Mod Env Attack", Signed, Timecents, -12000, 8000, -12000, false, "Modulation Envelope"),
    g!(27, "Mod Env Hold", Signed, Timecents, -12000, 5000, -12000, false, "Modulation Envelope"),
    g!(28, "Mod Env Decay", Signed, Timecents, -12000, 8000, -12000, false, "Modulation Envelope"),
    g!(29, "Mod Env Sustain", Signed, Permille, 0, 1000, 0, false, "Modulation Envelope"),
    g!(30, "Mod Env Release", Signed, Timecents, -12000, 8000, -12000, false, "Modulation Envelope"),
    g!(31, "Key → Mod Env Hold", Signed, CentsPerKey, -1200, 1200, 0, false, "Modulation Envelope"),
    g!(32, "Key → Mod Env Decay", Signed, CentsPerKey, -1200, 1200, 0, false, "Modulation Envelope"),
    g!(7, "Mod Env → Pitch", Signed, Cents, -12000, 12000, 0, false, "Modulation Envelope"),
    g!(11, "Mod Env → Filter Fc", Signed, Cents, -12000, 12000, 0, false, "Modulation Envelope"),
    g!(8, "Filter Cutoff", Signed, AbsCents, 1500, 13500, 13500, false, "Filter"),
    g!(9, "Filter Q", Signed, Centibels, 0, 960, 0, false, "Filter"),
    g!(21, "Mod LFO Delay", Signed, Timecents, -12000, 5000, -12000, false, "LFO"),
    g!(22, "Mod LFO Freq", Signed, AbsCents, -16000, 4500, 0, false, "LFO"),
    g!(5, "Mod LFO → Pitch", Signed, Cents, -12000, 12000, 0, false, "LFO"),
    g!(10, "Mod LFO → Filter Fc", Signed, Cents, -12000, 12000, 0, false, "LFO"),
    g!(13, "Mod LFO → Volume", Signed, Centibels, -960, 960, 0, false, "LFO"),
    g!(23, "Vib LFO Delay", Signed, Timecents, -12000, 5000, -12000, false, "LFO"),
    g!(24, "Vib LFO Freq", Signed, AbsCents, -16000, 4500, 0, false, "LFO"),
    g!(6, "Vib LFO → Pitch", Signed, Cents, -12000, 12000, 0, false, "LFO"),
    g!(15, "Chorus Send", Signed, Permille, 0, 1000, 0, false, "Effects"),
    g!(16, "Reverb Send", Signed, Permille, 0, 1000, 0, false, "Effects"),
];

pub fn info(id: u16) -> Option<&'static GenInfo> {
    GENERATORS.iter().find(|g| g.id == id)
}

/// Name for any generator id, including ones not in the editable table.
pub fn name(id: u16) -> String {
    match info(id) {
        Some(g) => g.name.to_string(),
        None => match id {
            INSTRUMENT => "Instrument".into(),
            SAMPLE_ID => "Sample".into(),
            _ => format!("Generator #{id}"),
        },
    }
}

pub fn default_value(id: u16) -> i32 {
    info(id).map(|g| g.default).unwrap_or(0)
}

pub fn timecents_to_secs(tc: f64) -> f64 {
    2f64.powf(tc / 1200.0)
}

pub fn abscents_to_hz(c: f64) -> f64 {
    8.176 * 2f64.powf(c / 1200.0)
}

/// Format a value with its physical unit for display.
pub fn describe(unit: GenUnit, v: i32) -> String {
    let v = v as f64;
    match unit {
        GenUnit::None | GenUnit::Samples => String::new(),
        GenUnit::Cents => format!("{v:+} cents"),
        GenUnit::AbsCents => format!("{:.2} Hz", abscents_to_hz(v)),
        GenUnit::Centibels => format!("{:.1} dB", v / 10.0),
        GenUnit::Permille => format!("{:.1} %", v / 10.0),
        GenUnit::Timecents => {
            if v <= -12000.0 {
                "0 s".into()
            } else {
                let s = timecents_to_secs(v);
                if s < 1.0 {
                    format!("{:.1} ms", s * 1000.0)
                } else {
                    format!("{s:.3} s")
                }
            }
        }
        GenUnit::Semitones => format!("{v:+} semi"),
        GenUnit::Keys => {
            if v < 0.0 {
                "off".into()
            } else {
                note_name(v as u8)
            }
        }
        GenUnit::CentsPerKey => format!("{v} c/key"),
        GenUnit::Mode => match v as i32 {
            0 => "No loop".into(),
            1 => "Loop".into(),
            3 => "Loop + release".into(),
            _ => "Unused (no loop)".into(),
        },
    }
}

pub fn note_name(key: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[key as usize % 12], key as i32 / 12 - 1)
}
