use super::config::{Args, AudioPolicy, Mode, resolved_mode};
use anyhow::{Context, Result, bail};
use std::fs;
const I2V_LINE: &str = "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.";
const CORE: &str = "integrated_multimodal_description: [Shot 1] Live-action cinematic footage with stable subjects, deliberate camera motion, natural physical action, and no subtitles, logos, watermarks, or newly invented people.\n\noverall_soundscape: Coherent room tone, ambience, action sounds, and non-verbal sounds; dialogue and music are kept out of this section.\n\nnon_diegetic_music: Sparse cinematic score, or N/A when no audience-only score is requested.";
pub fn build(args: &Args, duration: f64, soundtracked_videos: &[usize]) -> Result<String> {
    let mode = resolved_mode(args)?;
    if let Some(value) = &args.prompt {
        return review(&with_alignment(value, args, mode, duration), mode);
    }
    if let Some(path) = &args.prompt_file {
        let value = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        return review(&with_alignment(value.trim(), args, mode, duration), mode);
    }
    let value = match mode {
        Mode::T2v => CORE.into(),
        Mode::I2v | Mode::Flf2v => with_alignment(CORE, args, mode, duration),
        Mode::R2v => r2v(args, soundtracked_videos),
    };
    review(&value, mode)
}

fn with_alignment(value: &str, args: &Args, mode: Mode, duration: f64) -> String {
    let Some(line) = alignment_line(args, mode, duration) else {
        return value.to_owned();
    };
    // Preserve a caller-authored body byte-for-byte and prepend only the exact
    // required line when it is absent.
    if value.starts_with(&line) {
        value.to_owned()
    } else {
        format!("{line}\n\n{value}")
    }
}

fn alignment_line(args: &Args, mode: Mode, duration: f64) -> Option<String> {
    match mode {
        Mode::I2v if args.image.is_none() => Some(format!(
            "How the reference pictures align with the target video — <Picture 1> (from [Shot 1]) aligns with the {duration:.2}-second mark of the target video."
        )),
        Mode::I2v if args.end_image.is_some() => Some(flf2v_alignment_line(duration)),
        Mode::I2v => Some(I2V_LINE.to_owned()),
        Mode::Flf2v => Some(flf2v_alignment_line(duration)),
        Mode::T2v | Mode::R2v => None,
    }
}

fn flf2v_alignment_line(duration: f64) -> String {
    format!(
        "How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot 1) aligns with the {duration:.2}-second mark of the target video."
    )
}
fn r2v(args: &Args, soundtracked: &[usize]) -> String {
    let visual = if !args.ref_images.is_empty() {
        "<Picture 1>"
    } else {
        "<Video 1>"
    };
    let policy = args
        .source_audio_policy
        .map(AudioPolicy::as_str)
        .unwrap_or("none");
    let images = (1..=args.ref_images.len())
        .map(|n| format!("<Picture {n}> defines a visual subject or environment reference."))
        .collect::<Vec<_>>()
        .join("\n");
    let videos=(1..=args.ref_videos.len()).map(|n|format!("<Video {n}> defines camera movement, blocking, and temporal rhythm; soundtrack present: {}.",soundtracked.contains(&n))).collect::<Vec<_>>().join("\n");
    // Video soundtracks are presented before standalone clips, so their count
    // shifts every following <Audio N> ordinal.
    let audios = (1..=args.ref_audios.len() + soundtracked.len())
        .map(|n| format!("<Audio {n}> has source-audio policy {policy}."))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "subject_definitions:\n<Subject 1> is the lead subject whose identity comes from {visual}.\n{images}\n{videos}\n{audios}\n\nsummary:\n[reference generation] Create a coherent cinematic clip preserving <Subject 1>.\n\nretention_analysis:\n<Subject 1>: fully_preserved. Numbered picture, video, and audio references retain only their assigned roles.\n\ndetailed_description:\n[Shot 1] A continuous cinematic sequence with stable identity, anatomy, lighting, and physical motion. Use the numbered references exactly as assigned; add no subtitles, logos, or watermark.\n\noverall_soundscape:\nNatural synchronized ambience and action sounds. Source-audio policy: {policy}.\n\nnon_diegetic_music:\n{}",
        if policy == "reuse" {
            "Reuse the source signal unchanged; generate no replacement music."
        } else {
            "Sparse score guided only by explicitly assigned references."
        }
    )
}
fn review(value: &str, mode: Mode) -> Result<String> {
    if value.chars().count() > 7000 {
        bail!("H3 prompt exceeds the 7000-character limit");
    }
    let required = if mode == Mode::R2v {
        [
            "subject_definitions:",
            "summary:",
            "retention_analysis:",
            "detailed_description:",
            "overall_soundscape:",
            "non_diegetic_music:",
        ]
        .as_slice()
    } else {
        [
            "integrated_multimodal_description:",
            "overall_soundscape:",
            "non_diegetic_music:",
        ]
        .as_slice()
    };
    // Section review is advisory: arbitrary caller prompts remain valid input,
    // but ordering mistakes are called out because they reduce H3 prompt quality.
    let mut previous = 0;
    for section in required {
        if let Some(index) = value.find(section) {
            if index < previous {
                eprintln!("Warning: H3 Context-IR sections are out of official order.");
                break;
            }
            previous = index;
        } else {
            eprintln!("Warning: prompt omits H3 Context-IR section {section}");
        }
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::{ffi::OsString, io::Write, path::Path};

    const CUSTOM_BODY: &str = "subject_definitions: Custom references.\n\nsummary: Custom summary.\n\nretention_analysis: Preserve the subject.\n\ndetailed_description: Custom detail.\n\nintegrated_multimodal_description: [Shot 1] Custom cinematic action.\n\noverall_soundscape: Custom ambience.\n\nnon_diegetic_music: N/A";
    const L2V_LINE_8S: &str = "How the reference pictures align with the target video — <Picture 1> (from [Shot 1]) aligns with the 8.00-second mark of the target video.";
    const FLF2V_LINE_8S: &str = "How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot 1) aligns with the 8.00-second mark of the target video.";

    const T2V: &[&str] = &["--mode", "t2v"];
    const I2V: &[&str] = &["--mode", "i2v", "--image", "first.png"];
    const L2V: &[&str] = &["--mode", "i2v", "--end-image", "last.png"];
    const I2V_BOTH: &[&str] = &[
        "--mode",
        "i2v",
        "--image",
        "first.png",
        "--end-image",
        "last.png",
    ];
    const FLF2V: &[&str] = &[
        "--mode",
        "flf2v",
        "--image",
        "first.png",
        "--end-image",
        "last.png",
    ];
    const R2V: &[&str] = &["--mode", "r2v", "--ref-image", "reference.png"];

    fn inline_args(flags: &[&str], prompt: &str) -> Args {
        let values = std::iter::once("x")
            .chain(flags.iter().copied())
            .chain(std::iter::once(prompt));
        Args::try_parse_from(values).expect("inline prompt arguments")
    }

    fn file_args(flags: &[&str], path: &Path) -> Args {
        let mut values = vec![OsString::from("x")];
        values.extend(flags.iter().map(|value| OsString::from(*value)));
        values.push(OsString::from("--prompt-file"));
        values.push(path.as_os_str().to_owned());
        Args::try_parse_from(values).expect("file prompt arguments")
    }

    fn expected(line: Option<&str>, body: &str) -> String {
        line.map_or_else(|| body.to_owned(), |line| format!("{line}\n\n{body}"))
    }

    fn cases() -> [(&'static [&'static str], Option<&'static str>); 6] {
        [
            (T2V, None),
            (I2V, Some(I2V_LINE)),
            (L2V, Some(L2V_LINE_8S)),
            (I2V_BOTH, Some(FLF2V_LINE_8S)),
            (FLF2V, Some(FLF2V_LINE_8S)),
            (R2V, None),
        ]
    }

    #[test]
    fn inline_custom_prompts_receive_mode_specific_alignment() {
        for (flags, alignment) in cases() {
            let prompt = build(&inline_args(flags, CUSTOM_BODY), 8.0, &[]).expect("prompt");
            assert_eq!(prompt, expected(alignment, CUSTOM_BODY));
        }
    }

    #[test]
    fn file_custom_prompts_receive_mode_specific_alignment() {
        let mut file = tempfile::NamedTempFile::new().expect("prompt file");
        write!(file, "  \n{CUSTOM_BODY}\n  ").expect("write prompt");
        for (flags, alignment) in cases() {
            let prompt = build(&file_args(flags, file.path()), 8.0, &[]).expect("prompt");
            assert_eq!(prompt, expected(alignment, CUSTOM_BODY));
        }
    }

    #[test]
    fn existing_custom_alignment_is_not_duplicated() {
        for (flags, alignment) in [
            (I2V, I2V_LINE),
            (L2V, L2V_LINE_8S),
            (I2V_BOTH, FLF2V_LINE_8S),
            (FLF2V, FLF2V_LINE_8S),
        ] {
            let source = expected(Some(alignment), CUSTOM_BODY);
            let inline = build(&inline_args(flags, &source), 8.0, &[]).expect("inline prompt");
            assert_eq!(inline, source);

            let mut file = tempfile::NamedTempFile::new().expect("prompt file");
            file.write_all(source.as_bytes()).expect("write prompt");
            let from_file = build(&file_args(flags, file.path()), 8.0, &[]).expect("file prompt");
            assert_eq!(from_file, source);
        }
    }

    #[test]
    fn r2v_has_six_ordered_sections() {
        let args = Args::try_parse_from(["x", "--mode", "r2v", "--ref-image", "a.png"]).unwrap();
        let p = build(&args, 8.0, &[]).unwrap();
        let mut last = 0;
        for section in [
            "subject_definitions:",
            "summary:",
            "retention_analysis:",
            "detailed_description:",
            "overall_soundscape:",
            "non_diegetic_music:",
        ] {
            let at = p.find(section).unwrap();
            assert!(at >= last);
            last = at;
        }
    }
}
