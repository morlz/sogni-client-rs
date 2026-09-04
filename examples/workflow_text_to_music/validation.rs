use anyhow::{Result, bail};

use super::Args;

const KEYS: &[&str] = &[
    "A major",
    "A minor",
    "A# major",
    "A# minor",
    "Ab major",
    "Ab minor",
    "A♯ major",
    "A♯ minor",
    "A♭ major",
    "A♭ minor",
    "B major",
    "B minor",
    "B# major",
    "B# minor",
    "Bb major",
    "Bb minor",
    "B♯ major",
    "B♯ minor",
    "B♭ major",
    "B♭ minor",
    "C major",
    "C minor",
    "C# major",
    "C# minor",
    "Cb major",
    "Cb minor",
    "C♯ major",
    "C♯ minor",
    "C♭ major",
    "C♭ minor",
    "D major",
    "D minor",
    "D# major",
    "D# minor",
    "Db major",
    "Db minor",
    "D♯ major",
    "D♯ minor",
    "D♭ major",
    "D♭ minor",
    "E major",
    "E minor",
    "E# major",
    "E# minor",
    "Eb major",
    "Eb minor",
    "E♯ major",
    "E♯ minor",
    "E♭ major",
    "E♭ minor",
    "F major",
    "F minor",
    "F# major",
    "F# minor",
    "Fb major",
    "Fb minor",
    "F♯ major",
    "F♯ minor",
    "F♭ major",
    "F♭ minor",
    "G major",
    "G minor",
    "G# major",
    "G# minor",
    "Gb major",
    "Gb minor",
    "G♯ major",
    "G♯ minor",
    "G♭ major",
    "G♭ minor",
];
const LANGUAGES: &[&str] = &[
    "ar", "az", "bg", "bn", "ca", "cs", "da", "de", "el", "en", "es", "fa", "fi", "fr", "he", "hi",
    "hr", "ht", "hu", "id", "is", "it", "ja", "ko", "la", "lt", "ms", "ne", "nl", "no", "pa", "pl",
    "pt", "ro", "ru", "sa", "sk", "sr", "sv", "sw", "ta", "te", "th", "tl", "tr", "uk", "ur", "vi",
    "yue", "zh", "unknown",
];
const XL_SAMPLERS: &[&str] = &["euler", "euler_ancestral"];
const LEGACY_SFT_SAMPLERS: &[&str] = &["euler", "euler_ancestral", "er_sde"];
const SIMPLE_SCHEDULER: &[&str] = &["simple"];
const LEGACY_SFT_SCHEDULERS: &[&str] = &["simple", "linear_quadratic"];

#[derive(Clone, Copy)]
pub(super) struct ModelSpec {
    pub name: &'static str,
    pub steps: u32,
    pub step_range: (u32, u32),
    pub guidance: Option<f64>,
    pub sampler: &'static str,
    pub samplers: &'static [&'static str],
    pub scheduler: &'static str,
    pub schedulers: &'static [&'static str],
}

pub(super) fn model_spec(id: &str) -> Result<ModelSpec> {
    // Turbo variants deliberately omit CFG guidance. SFT variants retain it and
    // use their larger validated step/sampler ranges.
    let spec = match id {
        "ace_step_1.5_xl_turbo" => ModelSpec {
            name: "ACE-Step 1.5 XL Turbo",
            steps: 8,
            step_range: (4, 16),
            guidance: None,
            sampler: "euler",
            samplers: XL_SAMPLERS,
            scheduler: "simple",
            schedulers: SIMPLE_SCHEDULER,
        },
        "ace_step_1.5_xl_sft" => ModelSpec {
            name: "ACE-Step 1.5 XL SFT",
            steps: 50,
            step_range: (10, 200),
            guidance: Some(7.0),
            sampler: "euler",
            samplers: XL_SAMPLERS,
            scheduler: "simple",
            schedulers: SIMPLE_SCHEDULER,
        },
        "ace_step_1.5_turbo" => ModelSpec {
            name: "ACE-Step 1.5 Turbo (Legacy)",
            steps: 8,
            step_range: (4, 16),
            guidance: None,
            sampler: "euler",
            samplers: XL_SAMPLERS,
            scheduler: "simple",
            schedulers: SIMPLE_SCHEDULER,
        },
        "ace_step_1.5_sft" => ModelSpec {
            name: "ACE-Step 1.5 SFT (Legacy)",
            steps: 50,
            step_range: (10, 200),
            guidance: Some(5.0),
            sampler: "er_sde",
            samplers: LEGACY_SFT_SAMPLERS,
            scheduler: "linear_quadratic",
            schedulers: LEGACY_SFT_SCHEDULERS,
        },
        _ => bail!("unsupported ACE-Step model: {id}"),
    };
    Ok(spec)
}

pub(super) fn validate(
    args: &Args,
    spec: &ModelSpec,
    lyrics: &str,
    steps: u32,
    guidance: Option<f64>,
    sampler: &str,
    scheduler: &str,
) -> Result<()> {
    finite("Duration", args.duration)?;
    if !(10.0..=600.0).contains(&args.duration) {
        bail!("Duration must be between 10 and 600 seconds");
    }
    if !(30..=300).contains(&args.bpm) {
        bail!("BPM must be between 30 and 300");
    }
    if !KEYS.contains(&args.keyscale.as_str()) {
        bail!(
            "Key/scale must be one of: A major, A minor, A# major, A# minor, Ab major, Ab minor..."
        );
    }
    if !["2", "3", "4", "6"].contains(&args.timesignature.as_str()) {
        bail!("Time signature must be one of: 2, 3, 4, 6");
    }
    if !lyrics.is_empty() && !LANGUAGES.contains(&args.language.as_str()) {
        bail!("Language must be one of: {}", LANGUAGES.join(", "));
    }
    if !(spec.step_range.0..=spec.step_range.1).contains(&steps) {
        bail!(
            "Steps must be between {} and {} for {}",
            spec.step_range.0,
            spec.step_range.1,
            spec.name
        );
    }
    if let Some(raw) = args.guidance {
        finite("Guidance", raw)?;
    }
    if let Some(value) = guidance {
        if !(1.0..=15.0).contains(&value) {
            bail!("Guidance must be between 1 and 15");
        }
    }
    finite("Shift", args.shift)?;
    if !(1.0..=5.0).contains(&args.shift) {
        bail!("Shift must be between 1 and 5");
    }
    finite("Prompt strength", args.prompt_strength)?;
    if !(0.0..=10.0).contains(&args.prompt_strength) {
        bail!("Prompt strength must be between 0 and 10");
    }
    finite("Creativity", args.creativity)?;
    if !(0.0..=2.0).contains(&args.creativity) {
        bail!("Creativity must be between 0 and 2");
    }
    if !spec.samplers.contains(&sampler) {
        bail!(
            "Sampler must be one of: {} for {}",
            spec.samplers.join(", "),
            spec.name
        );
    }
    if !spec.schedulers.contains(&scheduler) {
        bail!(
            "Scheduler must be one of: {} for {}",
            spec.schedulers.join(", "),
            spec.name
        );
    }
    if !["mp3", "wav", "flac"].contains(&args.format.as_str()) {
        bail!("Format must be one of: mp3, wav, flac");
    }
    if !(1..=512).contains(&args.number) {
        bail!("Batch count must be between 1 and 512");
    }
    Ok(())
}

fn finite(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() {
        bail!("{name} must be finite");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
