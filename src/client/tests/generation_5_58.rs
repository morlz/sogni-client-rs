use super::project_wire_server::{Fixture, KEY};
use super::*;
use crate::{AssetRole, MediaSource, MinimaxH3Keyframe, ProjectRequest};
use serde_json::Value;

async fn client(fixture: &Fixture) -> SogniClient {
    SogniClient::builder()
        .app_id("generation-parity-fixture")
        .api_key(KEY)
        .rest_endpoint(format!("http://{}/", fixture.address).parse().unwrap())
        .socket_endpoint(format!("ws://{}/", fixture.address).parse().unwrap())
        .request_timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .await
        .unwrap()
}

fn image(bytes: &'static str) -> MediaSource {
    MediaSource::named_bytes(bytes, "fixture.png", "image/png")
}

async fn wire(fixture: &mut Fixture) -> Value {
    tokio::time::timeout(Duration::from_secs(3), fixture.wire.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn keyframes_upload_in_caller_order_without_displacing_references_or_placeholders() {
    let mut fixture = Fixture::start().await;
    let client = client(&fixture).await;
    let request = ProjectRequest::video("minimax-h3-ref2va-fp8_r2v", "A cut between keyframes")
        .param("frames", 243)
        .steps(20)
        .guidance(1.0)
        .asset(AssetRole::ReferenceImage, image("reference"))
        .asset(AssetRole::ContextImage(2), image("context"))
        .param(
            "keyframes",
            json!([
                {"image":true,"frameIndex":180},
                {"image":true,"frameIndex":90},
                {"image":true,"frameIndex":30},
            ]),
        )
        .asset(AssetRole::KeyframeImage(1), image("late"))
        .asset(AssetRole::KeyframeImage(3), image("early"));
    client.projects.create(request).await.unwrap();
    let actual = wire(&mut fixture).await;
    assert_eq!(
        actual["keyFrames"][0]["keyframeFrameIndices"],
        json!([180, 90, 30])
    );
    for slot in 1..=3 {
        assert_eq!(
            actual["keyFrames"][0][format!("hasKeyframeImage{slot}")],
            true
        );
        assert!(
            actual["keyFrames"][0]
                .get(format!("keyframeImage{slot}ContentType"))
                .is_none()
        );
    }
    assert_eq!(actual["keyFrames"][0]["hasContextImage2"], true);
    let requests = fixture.http.lock().clone();
    let registrations = requests
        .iter()
        .filter(|request| request.path == "/v1/image/uploadUrl")
        .collect::<Vec<_>>();
    assert_eq!(
        registrations
            .iter()
            .map(|request| request.query["type"].as_str())
            .collect::<Vec<_>>(),
        [
            "referenceImage",
            "contextImage2",
            "keyframeImage1",
            "keyframeImage3"
        ]
    );
    assert!(
        registrations
            .iter()
            .all(|request| request.query["contentType"] == "image/png")
    );
    let uploads = requests
        .iter()
        .filter(|request| request.method == "PUT")
        .collect::<Vec<_>>();
    assert_eq!(
        uploads
            .iter()
            .map(|request| request.body.as_slice())
            .collect::<Vec<_>>(),
        [b"reference".as_slice(), b"context", b"late", b"early"]
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn invalid_keyframes_never_upload_or_send() {
    let mut fixture = Fixture::start().await;
    let client = client(&fixture).await;
    for (model, frames, index) in [
        ("minimax-h3-fl2va-fp8_t2v", 243, 60),
        ("minimax-h3-fl2va-fp8_i2v", 141, 140),
    ] {
        let request = ProjectRequest::video(model, "fixture")
            .param("frames", frames)
            .asset(AssetRole::ReferenceImage, image("first"))
            .keyframes(vec![MinimaxH3Keyframe::new(image("invalid"), index)]);
        assert!(client.projects.create(request).await.is_err());
    }
    assert!(
        !fixture
            .http
            .lock()
            .iter()
            .any(|request| request.method == "PUT" || request.path.contains("uploadUrl"))
    );
    assert!(fixture.wire.try_recv().is_err());
    client.close().await.unwrap();
}

#[tokio::test]
async fn estimates_forward_keyframe_counts_and_match_krea_enhancement_strength_and_size() {
    let fixture = Fixture::start().await;
    let client = client(&fixture).await;
    for (overrides, expected) in [
        (json!({"keyframeCount":8}), Some("8")),
        (json!({"keyframes":[{},{},{}]}), Some("3")),
        (json!({"keyframeCount":0,"keyframes":[{}]}), None),
        (json!({"keyframeCount":2.9}), Some("2")),
        (json!({"keyframeCount":-1}), None),
        (json!({"keyframeCount":"3","keyframes":[{}]}), None),
    ] {
        let mut params = json!({"model":"minimax-h3-fl2va-fp8_i2v","tokenType":"spark","width":672,"height":384,"frames":243,"fps":24});
        params
            .as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        client.projects.estimate_video_cost(&params).await.unwrap();
        let requests = fixture.http.lock();
        let quote = requests
            .iter()
            .rev()
            .find(|request| request.path.starts_with("/api/v1/job-video/estimate/"))
            .unwrap();
        assert_eq!(
            quote.query.get("keyframeCount").map(String::as_str),
            expected
        );
    }
    for (strength, denoise) in [("light", 0.15), ("medium", 0.35), ("heavy", 0.49)] {
        client
            .projects
            .estimate_enhancement_cost_with_size(strength, "spark", 1152, 896)
            .await
            .unwrap();
        let requests = fixture.http.lock();
        let quote = requests
            .iter()
            .rev()
            .find(|request| request.path.starts_with("/api/v2/job/estimate/"))
            .unwrap();
        let segments = quote.path.split('/').collect::<Vec<_>>();
        assert_eq!(
            &segments[5..12],
            &[
                "spark",
                "fast",
                "krea2_turbo_fp8_scaled",
                "1",
                "8",
                "0",
                "0"
            ]
        );
        assert!((segments[12].parse::<f64>().unwrap() - denoise).abs() < 1e-9);
        assert_eq!(&segments[13..], &["1152", "896"]);
    }
    client
        .projects
        .estimate_enhancement_cost("light", "spark")
        .await
        .unwrap();
    assert!(fixture.http.lock().last().unwrap().path.ends_with("/0/0"));
    client.close().await.unwrap();
}

#[tokio::test]
async fn enhancement_resolves_parent_canvas_and_preserves_zero_seed() {
    let mut fixture = Fixture::start().await;
    let client = client(&fixture).await;
    for (strength, size, expected, denoise) in [
        (
            "light",
            json!({"sizePreset":"custom","width":1152,"height":896}),
            Some((1152, 896)),
            0.15,
        ),
        (
            "medium",
            json!({"sizePreset":"portrait","network":"relaxed"}),
            Some((896, 1152)),
            0.35,
        ),
        ("heavy", json!({}), None, 0.49),
    ] {
        let mut request =
            ProjectRequest::image("fixture-enhancement-parent", "source").param("seed", 42);
        for (key, value) in size.as_object().unwrap() {
            request = request.param(key, value.clone());
        }
        let project = client.projects.create(request).await.unwrap();
        project
            .wait_for_completion(Some(Duration::from_secs(3)))
            .await
            .unwrap();
        wire(&mut fixture).await;
        let job = project.jobs().into_iter().next().unwrap();
        tokio::time::timeout(Duration::from_secs(3), job.enhance(strength, None))
            .await
            .unwrap()
            .unwrap();
        let request = wire(&mut fixture).await;
        let frame = &request["keyFrames"][0];
        assert_eq!(frame["modelID"], "krea2_turbo_fp8_scaled");
        assert_eq!(frame["steps"], 8);
        assert_eq!(frame["seed"], 0);
        assert!((frame["strength"].as_f64().unwrap() - denoise).abs() < 1e-9);
        if let Some((width, height)) = expected {
            assert_eq!(frame["sizePreset"], "custom");
            assert_eq!(frame["width"].as_f64(), Some(f64::from(width)));
            assert_eq!(frame["height"].as_f64(), Some(f64::from(height)));
        } else {
            assert!(frame.get("sizePreset").is_none());
        }
    }
    assert!(fixture.http.lock().iter().any(|request| request.path
        == "/api/v1/size-presets/network/relaxed/model/fixture-enhancement-parent"));
    let project = client
        .projects
        .create(
            ProjectRequest::image("fixture-enhancement-parent", "source")
                .param("sizePreset", "missing"),
        )
        .await
        .unwrap();
    project
        .wait_for_completion(Some(Duration::from_secs(3)))
        .await
        .unwrap();
    wire(&mut fixture).await;
    let before = fixture
        .http
        .lock()
        .iter()
        .filter(|request| request.path == "/fixture-result" || request.method == "PUT")
        .count();
    assert!(
        project.jobs()[0]
            .enhance("light", None)
            .await
            .unwrap_err()
            .to_string()
            .contains("Size preset \"missing\" is not available")
    );
    let after = fixture
        .http
        .lock()
        .iter()
        .filter(|request| request.path == "/fixture-result" || request.method == "PUT")
        .count();
    assert_eq!(before, after);
    assert!(fixture.wire.try_recv().is_err());
    client.close().await.unwrap();
}

#[tokio::test]
async fn slow_result_signing_publishes_completion_metadata_and_preserves_a_newer_url() {
    let mut fixture = Fixture::start().await;
    fixture.signing.hold_first.store(true, Ordering::SeqCst);
    let client = client(&fixture).await;
    let mut events = client.projects.subscribe();
    let project = client
        .projects
        .create(ProjectRequest::image("fixture-held-result-parent", "source").param("seed", 42))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), fixture.signing.entered.notified())
        .await
        .unwrap();

    // The result event's first signer is held. The waiter obtains a URL through
    // its own lookup after the following socket project-completed event.
    let urls = project
        .wait_for_completion(Some(Duration::from_secs(3)))
        .await
        .unwrap();
    let job = project.jobs().into_iter().next().unwrap();
    assert_eq!(job.snapshot().seed, Some(0));
    assert_eq!(job.snapshot().step, 7.0);
    assert_eq!(
        urls,
        vec![format!("http://{}/fixture-result", fixture.address)]
    );
    wire(&mut fixture).await;

    tokio::time::timeout(Duration::from_secs(3), job.enhance("light", None))
        .await
        .unwrap()
        .unwrap();
    let enhancement = wire(&mut fixture).await;
    assert_eq!(enhancement["keyFrames"][0]["seed"], 0);

    // The delayed signer fails after a successful lookup. It must not erase
    // the URL or completed metadata that the caller has already observed.
    fixture.signing.release.notify_one();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.name == "job" && event.data["jobID"] == project.id() {
                assert_eq!(event.data["resultUrl"], urls[0]);
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(job.result_url().as_deref(), Some(urls[0].as_str()));
    assert_eq!(job.snapshot().seed, Some(0));
    assert_eq!(job.snapshot().step, 7.0);
    client.close().await.unwrap();
}
