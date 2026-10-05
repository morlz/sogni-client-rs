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
    pub ace_controls: bool,
    pub duration_default: f64,
    pub duration_range: (f64, f64),
    pub guidance_range: (f64, f64),
    pub prompt_strength_default: f64,
}

pub(super) fn model_spec(id: &str) -> Result<ModelSpec> {
    // Turbo variants deliberately omit CFG guidance. SFT variants retain it and
    // use their larger validated step/sampler ranges.
    let ace = ModelSpec {
        name: "ACE-Step 1.5 XL Turbo",
        steps: 8,
        step_range: (4, 16),
        guidance: None,
        sampler: "euler",
        samplers: XL_SAMPLERS,
        scheduler: "simple",
        schedulers: SIMPLE_SCHEDULER,
        ace_controls: true,
        duration_default: 30.0,
        duration_range: (10.0, 600.0),
        guidance_range: (1.0, 15.0),
        prompt_strength_default: 2.0,
    };
    let spec = match id {
        "minimax_music3" => ModelSpec {
            name: "MiniMax Music 3",
            steps: 30,
            step_range: (10, 100),
            guidance: Some(1.7),
            samplers: &["euler"],
            ace_controls: false,
            duration_default: 60.0,
            duration_range: (10.0, 300.0),
            guidance_range: (1.0, 5.0),
            prompt_strength_default: 1.7,
            ..ace
        },
        "ace_step_1.5_xl_turbo" => ModelSpec {
            name: "ACE-Step 1.5 XL Turbo",
            steps: 8,
            step_range: (4, 16),
            guidance: None,
            sampler: "euler",
            samplers: XL_SAMPLERS,
            scheduler: "simple",
            schedulers: SIMPLE_SCHEDULER,
            ..ace
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
            ..ace
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
            ..ace
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
            ..ace
        },
        _ => bail!("unsupported music model: {id}"),
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
    if !spec.ace_controls
        && (args.shift.is_some() || args.no_composer_mode || args.creativity.is_some())
    {
        bail!(
            "MiniMax Music 3 has no shift, composer-mode or creativity controls; use --model ace_step_1.5_xl_turbo"
        );
    }
    let duration = args.duration.unwrap_or(spec.duration_default);
    finite("Duration", duration)?;
    if !(spec.duration_range.0..=spec.duration_range.1).contains(&duration) {
        if spec.ace_controls {
            bail!("Duration must be between 10 and 600 seconds");
        }
        bail!("Duration must be between 10 and 300 seconds for MiniMax Music 3");
    }
    if spec.ace_controls && args.bpm.is_some_and(|bpm| !(30..=300).contains(&bpm)) {
        bail!("BPM must be between 30 and 300");
    }
    if spec.ace_controls
        && args
            .keyscale
            .as_deref()
            .is_some_and(|key| !KEYS.contains(&key))
    {
        bail!(
            "Key/scale must be one of: A major, A minor, A# major, A# minor, Ab major, Ab minor..."
        );
    }
    if spec.ace_controls
        && args
            .timesignature
            .as_deref()
            .is_some_and(|signature| !["2", "3", "4", "6"].contains(&signature))
    {
        bail!("Time signature must be one of: 2, 3, 4, 6");
    }
    if spec.ace_controls
        && !lyrics.is_empty()
        && args
            .language
            .as_deref()
            .is_some_and(|language| !LANGUAGES.contains(&language))
    {
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
        if !(spec.guidance_range.0..=spec.guidance_range.1).contains(&value) {
            if spec.ace_controls {
                bail!("Guidance must be between 1 and 15");
            }
            bail!("Guidance must be between 1 and 5 for MiniMax Music 3");
        }
    }
    let shift = args.shift.unwrap_or(3.0);
    finite("Shift", shift)?;
    if !(1.0..=5.0).contains(&shift) {
        bail!("Shift must be between 1 and 5");
    }
    let prompt_strength = args.prompt_strength.unwrap_or(spec.prompt_strength_default);
    finite("Prompt strength", prompt_strength)?;
    if !(0.0..=10.0).contains(&prompt_strength) {
        bail!("Prompt strength must be between 0 and 10");
    }
    let creativity = args.creativity.unwrap_or(0.85);
    finite("Creativity", creativity)?;
    if !(0.0..=2.0).contains(&creativity) {
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
