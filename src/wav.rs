//! WAV import/export for samples.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::sf2::{Sample, sample_type};

/// Reads a WAV file as one mono sample, or a linked left/right pair.
pub fn import(path: &Path) -> Result<Vec<Sample>> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let ch = spec.channels as usize;
    if ch == 0 {
        bail!("WAV file has no channels");
    }
    let interleaved: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16))
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let shift = spec.bits_per_sample as i32 - 16;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| if shift >= 0 { (v >> shift) as i16 } else { (v << -shift) as i16 }))
                .collect::<Result<_, _>>()?
        }
    };

    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sample".into());
    let make = |name: String, data: Vec<i16>, kind: u16| Sample {
        name: name.chars().take(20).collect(),
        data: Arc::new(data),
        loop_start: 0,
        loop_end: 0,
        sample_rate: spec.sample_rate,
        original_pitch: 60,
        pitch_correction: 0,
        link: None,
        sample_type: kind,
    };

    if ch == 2 {
        let l = interleaved.iter().step_by(2).copied().collect();
        let r = interleaved.iter().skip(1).step_by(2).copied().collect();
        let base: String = stem.chars().take(18).collect();
        Ok(vec![make(format!("{base}_L"), l, sample_type::LEFT), make(format!("{base}_R"), r, sample_type::RIGHT)])
    } else {
        // Mono, or a mixdown of more than two channels.
        let mono = interleaved
            .chunks_exact(ch)
            .map(|f| (f.iter().map(|&v| v as i32).sum::<i32>() / ch as i32) as i16)
            .collect();
        Ok(vec![make(stem, mono, sample_type::MONO)])
    }
}

pub fn export(path: &Path, sample: &Sample) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sample.sample_rate.max(1),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &v in sample.data.iter() {
        w.write_sample(v)?;
    }
    w.finalize()?;
    Ok(())
}
