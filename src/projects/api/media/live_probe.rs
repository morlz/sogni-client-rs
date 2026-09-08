//! Opt-in asset transfer diagnostic. It never creates a generation project.
use std::{path::PathBuf, time::Duration};

use super::*;
use crate::SogniClient;

#[tokio::test]
#[ignore = "requires SOGNI_MEDIA_PROBE=1 and authorization to upload the configured local image"]
async fn registered_guide_round_trip_without_generation() -> Result<()> {
    if std::env::var("SOGNI_MEDIA_PROBE").as_deref() != Ok("1") {
        return Err(Error::InvalidInput("media probe is not enabled".into()));
    }
    let source = std::env::var_os("SOGNI_MEDIA_PROBE_SOURCE")
        .map(PathBuf::from)
        .ok_or_else(|| Error::InvalidInput("media probe source is required".into()))?;
    let key = std::env::var("SOGNI_API_KEY")
        .map_err(|_| Error::InvalidInput("API key is required".into()))?;
    let proxy = std::env::var("SOGNI_PROXY_URL")
        .map_err(|_| Error::InvalidInput("explicit proxy is required".into()))?;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/media-transport-probe")
        .join(new_id());
    tokio::fs::create_dir_all(&workspace).await?;
    let input = prepare_input(&source, &workspace)?;
    let bytes = tokio::fs::read(&input).await?;
    let client = SogniClient::builder()
        .app_id("local-parity-fixture")
        .api_key(key)
        .proxy_url(proxy)
        .strict_media_destinations(true)
        .defer_socket_start(true)
        .connect_timeout(Duration::from_secs(10))
        .request_timeout(Duration::from_secs(120))
        .build()
        .await?;
    let started = std::time::Instant::now();
    let result = probe(&client, &workspace, &bytes).await;
    println!(
        "media probe elapsed_ms={} completed={} paid_projects=0 realtime_connected={}",
        started.elapsed().as_millis(),
        result.is_ok(),
        client.is_socket_connected(),
    );
    client.close().await?;
    result.map_err(|_| Error::Transport("media probe failed; see redacted stage".into()))
}

async fn probe(client: &SogniClient, workspace: &std::path::Path, bytes: &[u8]) -> Result<()> {
    checked(
        "model_lookup",
        client
            .projects
            .get_model_options("z_image_turbo_bf16", false)
            .await,
    )?;
    let project_id = new_id();
    let image_id = new_id();
    let identity = json!({ "projectId": project_id, "imageId": image_id });
    tokio::fs::write(
        workspace.join("identity-private.json"),
        serde_json::to_vec(&identity)?,
    )
    .await?;
    let query = json!({
        "jobId": project_id, "imageId": image_id, "type": "startingImage", "contentType": "image/png",
    });
    let url = checked(
        "guide_registration",
        client.projects.upload_url(&query).await,
    )?;
    checked(
        "guide_put",
        client
            .projects
            .inner
            .client
            .rest
            .put_bytes(url, bytes::Bytes::copy_from_slice(bytes), Some("image/png"))
            .await,
    )?;
    let url = checked(
        "guide_download_url",
        client.projects.download_url(&query).await,
    )?;
    let downloaded = checked(
        "guide_download",
        client.projects.inner.client.rest.get_bytes(url).await,
    )?;
    if downloaded.as_ref() != bytes {
        return Err(Error::Protocol("guide bytes do not match".into()));
    }
    tokio::fs::write(workspace.join("verified-guide.png"), &downloaded).await?;
    println!(
        "media probe round_trip_bytes={} identical=true",
        downloaded.len()
    );
    Ok(())
}

fn prepare_input(source: &std::path::Path, workspace: &std::path::Path) -> Result<PathBuf> {
    let mut reader = image::ImageReader::open(source)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| Error::InvalidInput("probe image cannot be decoded".into()))?;
    if image.width() < 1024 || image.height() < 1024 {
        return Err(Error::InvalidInput(
            "probe requires at least 1024 square pixels".into(),
        ));
    }
    let path = workspace.join("guide-input.png");
    image
        .crop_imm(0, 0, 1024, 1024)
        .to_rgba8()
        .save(&path)
        .map_err(|_| Error::Io(std::io::Error::other("probe image cannot be saved")))?;
    Ok(path)
}

fn checked<T>(stage: &'static str, result: Result<T>) -> Result<T> {
    match &result {
        Ok(_) => println!("media probe stage={stage} completed=true"),
        Err(error) => match error {
            Error::Http(error) => {
                let detail = format!("{error:?}").to_lowercase();
                let flags = [
                    "certificate",
                    "dns",
                    "socks",
                    "proxy",
                    "refused",
                    "unreachable",
                    "reset",
                    "general socks server failure",
                    "host unreachable",
                    "network unreachable",
                    "connection refused",
                    "invalid peer certificate",
                    "unexpected eof",
                ];
                eprintln!(
                    "media probe stage={stage} category=http connect={} timeout={} status={:?} flags={:?}",
                    error.is_connect(),
                    error.is_timeout(),
                    error.status().map(|status| status.as_u16()),
                    flags
                        .into_iter()
                        .filter(|flag| detail.contains(flag))
                        .collect::<Vec<_>>(),
                );
            }
            Error::Api(error) => eprintln!(
                "media probe stage={stage} category=api status={}",
                error.status
            ),
            Error::Transport(_) => eprintln!("media probe stage={stage} category=transport"),
            Error::InvalidInput(_) => eprintln!("media probe stage={stage} category=input"),
            _ => eprintln!("media probe stage={stage} category=protocol"),
        },
    }
    result
}
