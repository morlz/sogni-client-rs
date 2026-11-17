use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

struct Release(Arc<AtomicBool>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn resources(released: &Arc<AtomicBool>) -> AssetResources {
    AssetResources {
        _guard: Arc::new(Release(released.clone())),
    }
}

async fn wait_released(released: &AtomicBool) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !released.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;

    Ok(())
}

#[tokio::test]
async fn request_clones_and_zero_copy_http_bytes_retain_admission_without_wire_state()
-> anyhow::Result<()> {
    let released = Arc::new(AtomicBool::new(false));
    let original = ProjectRequest::image("model", "prompt");
    let request = original
        .clone()
        .retain_asset_resources(Release(released.clone()));
    let cloned = request.clone();
    assert_eq!(request.params(), original.params());
    assert!(format!("{request:?}").contains("AssetResources([redacted])"));
    let input = Bytes::from(vec![31; 4096]);
    let pointer = input.as_ptr();
    let source = MediaSource::bytes(input);
    let media = source
        .read_with_resources(cloned.asset_resources.clone())
        .await?;
    assert_eq!(
        media.data.as_ptr(),
        pointer,
        "wrapping must not copy the payload"
    );
    let http_body = reqwest::Body::from(media.data.clone());
    drop(media);
    drop(request);
    drop(cloned);
    assert!(!released.load(Ordering::SeqCst));
    drop(http_body);
    assert!(released.load(Ordering::SeqCst));
    assert!(original.asset_resources.is_none());

    Ok(())
}

#[test]
fn cancelled_path_read_retains_admission_until_blocking_work_really_settles() -> anyhow::Result<()>
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.png");
        std::fs::write(&path, b"bounded fixture")?;
        let (continue_work, blocked) = std::sync::mpsc::channel();
        let (entered, started) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = blocked.recv();
        });
        started.await?;
        let released = Arc::new(AtomicBool::new(false));
        let retained = resources(&released);
        let read = tokio::spawn(async move {
            MediaSource::Path(path)
                .read_with_resources(Some(retained))
                .await
        });
        // The real filesystem read is queued behind the occupied blocking pool.
        tokio::time::sleep(Duration::from_millis(30)).await;
        read.abort();
        assert!(read.await.is_err());
        assert!(!released.load(Ordering::SeqCst));
        continue_work.send(())?;
        blocker.await?;
        wait_released(&released).await?;

        Ok(())
    })
}

#[tokio::test]
async fn cancelled_http_upload_frees_retained_bytes_after_transport_settles() -> anyhow::Result<()>
{
    use tokio::io::AsyncReadExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (entered, started) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut headers = [0; 1024];
        socket.read_exact(&mut headers).await?;
        let _ = entered.send(());
        std::future::pending::<()>().await;

        Ok::<(), std::io::Error>(())
    });
    let released = Arc::new(AtomicBool::new(false));
    let media = MediaSource::bytes(vec![17; 8 * 1024 * 1024])
        .read_with_resources(Some(resources(&released)))
        .await?;
    let upload = tokio::spawn(async move {
        reqwest::Client::new()
            .put(format!("http://{address}/"))
            .body(media.data)
            .send()
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), started).await??;
    assert!(!released.load(Ordering::SeqCst));
    upload.abort();
    assert!(upload.await.is_err());
    server.abort();
    let _ = server.await;
    wait_released(&released).await?;

    Ok(())
}
