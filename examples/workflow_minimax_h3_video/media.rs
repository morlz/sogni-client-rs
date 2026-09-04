use super::config::{Args, AudioPolicy, Mode, resolved_mode};
use crate::common::files::{MediaMetadata, ffprobe, require_file, unique_path};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub struct Probed {
    pub video: Vec<MediaMetadata>,
    pub soundtracked: Vec<usize>,
}
pub fn probe(args: &Args) -> Result<Probed> {
    for path in args
        .image
        .iter()
        .chain(args.end_image.iter())
        .chain(args.ref_images.iter())
        .chain(args.ref_videos.iter())
        .chain(args.ref_audios.iter())
    {
        require_file(path, "reference media")?;
    }
    let mut videos = Vec::new();
    let mut soundtracked = Vec::new();
    for (index, path) in args.ref_videos.iter().enumerate() {
        let meta =
            ffprobe(path).with_context(|| format!("ffprobe reference video {}", index + 1))?;
        let duration = meta.duration_seconds.ok_or_else(|| {
            anyhow::anyhow!("reference video {} has no measurable duration", index + 1)
        })?;
        if !(1.95..=15.05).contains(&duration) {
            bail!("reference video {} must be 2-15 seconds", index + 1);
        }
        if meta.fps.is_none_or(|fps| (fps - 24.0).abs() > 0.001) {
            bail!("reference video {} must be exactly 24fps", index + 1);
        }
        // On probe failure, assuming audio avoids binding a standalone voice to
        // an earlier, uncounted video soundtrack ordinal.
        if has_audio(path).unwrap_or_else(|error| {
            eprintln!(
                "Warning: {error}; assuming reference video {} has audio.",
                index + 1
            );
            true
        }) {
            soundtracked.push(index + 1);
        }
        videos.push(meta);
    }
    if videos
        .iter()
        .filter_map(|v| v.duration_seconds)
        .sum::<f64>()
        > 15.05
    {
        bail!("reference videos may total at most 15 seconds");
    }
    let mut audios = Vec::new();
    for (index, path) in args.ref_audios.iter().enumerate() {
        let meta =
            ffprobe(path).with_context(|| format!("ffprobe reference audio {}", index + 1))?;
        let duration = meta
            .duration_seconds
            .ok_or_else(|| anyhow::anyhow!("reference audio has no duration"))?;
        if !(1.95..=15.05).contains(&duration) {
            bail!("reference audio {} must be 2-15 seconds", index + 1);
        }
        audios.push(meta);
    }
    if audios
        .iter()
        .filter_map(|v| v.duration_seconds)
        .sum::<f64>()
        > 15.05
    {
        bail!("reference audios may total at most 15 seconds");
    }
    // Natural-language prompting is not authority for destructive soundtrack
    // replacement; callers must choose reuse, reference, or replace explicitly.
    let source_count = soundtracked.len() + args.ref_audios.len();
    if source_count > 0 && args.source_audio_policy.is_none() {
        bail!("source audio is attached; set --source-audio-policy reuse, reference, or replace");
    }
    if source_count == 0 && args.source_audio_policy.is_some() {
        bail!("source-audio-policy was set but no source audio is attached");
    }
    if args.source_audio_policy == Some(AudioPolicy::Reuse) && source_count != 1 {
        bail!("reuse requires exactly one source soundtrack");
    }
    Ok(Probed {
        video: videos,
        soundtracked,
    })
}
fn has_audio(path: &Path) -> Result<bool> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .context("run ffprobe audio inspection")?;
    if !output.status.success() {
        bail!("ffprobe could not inspect audio in {}", path.display());
    }
    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}
pub fn reuse_source(args: &Args, probed: &Probed) -> Option<PathBuf> {
    if args.source_audio_policy != Some(AudioPolicy::Reuse) {
        return None;
    }
    probed
        .soundtracked
        .first()
        .map(|index| args.ref_videos[index - 1].clone())
        .or_else(|| args.ref_audios.first().cloned())
}
pub fn remux_exact(video: &Path, audio: &Path) -> Result<PathBuf> {
    // Preserve the generated-audio result first. If stream-copy fails, restore
    // it atomically rather than leaving a partial final deliverable.
    let backup = unique_path(video.with_extension("generated-audio.mp4"));
    fs::rename(video, &backup).with_context(|| format!("preserve {}", video.display()))?;
    let temporary = video.with_extension("remux.part.mp4");
    let output = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&backup)
        .arg("-i")
        .arg(audio)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy", "-shortest"])
        .arg(&temporary)
        .output();
    match output {
        Ok(value) if value.status.success() => {
            fs::rename(&temporary, video)?;
            Ok(backup)
        }
        Ok(value) => {
            let _ = fs::remove_file(&temporary);
            let _ = fs::rename(&backup, video);
            bail!(
                "ffmpeg remux failed: {}",
                String::from_utf8_lossy(&value.stderr).trim()
            )
        }
        Err(error) => {
            let _ = fs::rename(&backup, video);
            bail!("could not run ffmpeg for exact soundtrack reuse: {error}")
        }
    }
}
pub fn no_probe(args: &Args) -> Probed {
    Probed {
        video: vec![MediaMetadata::default(); args.ref_videos.len()],
        soundtracked: Vec::new(),
    }
}
pub fn validate_live_mode(args: &Args) -> Result<()> {
    if resolved_mode(args)? != Mode::R2v {
        for path in args.image.iter().chain(args.end_image.iter()) {
            require_file(path, "frame image")?;
        }
    }
    Ok(())
}
