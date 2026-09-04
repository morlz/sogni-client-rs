use std::{
    ffi::OsStr,
    fs,
    io::IsTerminal,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt as _;
use serde_json::Value;
use tokio::io::AsyncWriteExt as _;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MediaMetadata {
    pub duration_seconds: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
}

pub fn require_file(path: &Path, label: &str) -> Result<()> {
    if !path.is_file() {
        bail!(
            "{label} does not exist or is not a file: {}",
            path.display()
        );
    }
    Ok(())
}

/// Resolve a media input, prompting for a path only in interactive mode.
pub fn choose_file(path: Option<PathBuf>, label: &str, interactive: bool) -> Result<PathBuf> {
    let path = match path {
        Some(path) => path,
        None if interactive && std::io::stdin().is_terminal() => {
            PathBuf::from(super::cli::prompt(&format!("Path to {label}"), None)?)
        }
        None => bail!("{label} is required; provide its CLI option"),
    };
    require_file(&path, label)?;
    Ok(path)
}

pub fn ensure_output_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create output directory {}", path.display()))
}

/// Return an unused path by appending `-2`, `-3`, and so on when necessary.
pub fn unique_path(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    if !path.exists() {
        return path;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or("result");
    let extension = path.extension().and_then(OsStr::to_str);
    for index in 2.. {
        let name = match extension {
            Some(extension) => format!("{stem}-{index}.{extension}"),
            None => format!("{stem}-{index}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("the integer path suffix space is effectively unbounded")
}

/// Stream an HTTP(S) result to a new local file without buffering it in memory.
pub async fn download(url: &str, destination: impl Into<PathBuf>) -> Result<PathBuf> {
    let destination = unique_path(destination);
    if let Some(parent) = destination.parent() {
        ensure_output_dir(parent)?;
    }
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("download {url}"))?;
    let temporary = destination.with_extension(format!(
        "{}.part",
        destination
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("download")
    ));
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .with_context(|| format!("create {}", temporary.display()))?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        output.write_all(&chunk?).await?;
    }
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    if let Err(error) = tokio::fs::rename(&temporary, &destination).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error).with_context(|| format!("finish {}", destination.display()));
    }
    Ok(destination)
}

pub async fn download_results(
    urls: &[String],
    output_dir: &Path,
    prefix: &str,
    extension: &str,
) -> Result<Vec<PathBuf>> {
    ensure_output_dir(output_dir)?;
    let mut paths = Vec::with_capacity(urls.len());
    for (index, url) in urls.iter().enumerate() {
        let desired = output_dir.join(format!("{prefix}-{}.{}", index + 1, extension));
        let path = download(url, desired).await?;
        println!("Saved {}", path.display());
        paths.push(path);
    }
    Ok(paths)
}

pub fn image_dimensions(path: &Path) -> Result<(u32, u32)> {
    image::ImageReader::open(path)
        .with_context(|| format!("open image {}", path.display()))?
        .with_guessed_format()?
        .into_dimensions()
        .with_context(|| format!("read image dimensions from {}", path.display()))
}

/// Read duration, dimensions, and frame rate using an installed `ffprobe`.
pub fn ffprobe(path: &Path) -> Result<MediaMetadata> {
    require_file(path, "media input")?;
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,width,height,avg_frame_rate",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .context("run ffprobe (install FFmpeg and ensure ffprobe is on PATH)")?;
    if !output.status.success() {
        bail!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value: Value = serde_json::from_slice(&output.stdout).context("parse ffprobe JSON")?;
    let duration_seconds = value
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok());
    let video = value
        .get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"));
    Ok(MediaMetadata {
        duration_seconds,
        width: video
            .and_then(|stream| stream.get("width"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        height: video
            .and_then(|stream| stream.get("height"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        fps: video
            .and_then(|stream| stream.get("avg_frame_rate"))
            .and_then(Value::as_str)
            .and_then(parse_ratio),
    })
}

fn parse_ratio(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    (denominator != 0.0).then_some(numerator / denominator)
}

pub fn slug(value: &str, maximum: usize) -> String {
    let mut output = String::with_capacity(value.len().min(maximum));
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            separator = false;
        } else if !output.is_empty() && !separator {
            output.push('-');
            separator = true;
        }
        if output.len() >= maximum {
            break;
        }
    }
    output.trim_matches('-').to_owned()
}
