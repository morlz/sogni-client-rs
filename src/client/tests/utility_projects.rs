use super::{
    SogniClient,
    project_wire_server::{Fixture, KEY},
};
use crate::{AssetRole, MediaSource, ProjectRequest, Sam3ImagePrompt};
use serde_json::json;
use std::time::Duration;
use url::Url;

#[tokio::test]
async fn utility_create_normalizes_project_state_and_uploads_the_starting_image() {
    for (model, format, media) in [
        ("sam3_image_segment_bf16", "png", "image"),
        ("pixal3d_int8_i23d", "glb", "model"),
    ] {
        let mut fixture = Fixture::start().await;
        let client = SogniClient::builder()
            .app_id("utility-fixture")
            .api_key(KEY)
            .rest_endpoint(Url::parse(&format!("http://{}/", fixture.address)).unwrap())
            .socket_endpoint(Url::parse(&format!("ws://{}/", fixture.address)).unwrap())
            .request_timeout(Duration::from_secs(3))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .await
            .unwrap();
        let mut request = ProjectRequest::image(model, "a teapot")
            .number_of_media(if media == "image" { 4 } else { 1 })
            .param("numberOfPreviews", 5)
            .param("outputFormat", "jpg")
            .asset(
                AssetRole::StartingImage,
                MediaSource::named_bytes(
                    b"\x89PNG\r\n\x1a\nfixture".as_slice(),
                    "source.png",
                    "image/png",
                ),
            );
        if media == "image" {
            request = request.sam3_prompt(Sam3ImagePrompt {
                text: Some("teapot".into()),
                ..Default::default()
            });
        }
        let project = client.projects.create(request).await.unwrap();
        let wire = tokio::time::timeout(Duration::from_secs(3), fixture.wire.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(wire["outputFormat"], format);
        assert_eq!(wire["keyFrames"][0]["hasStartingImage"], true);
        if media == "image" {
            assert_eq!(wire["numberOfImages"], 1);
            assert_eq!(wire["previews"], 0);
            assert_eq!(project.snapshot().params["numberOfMedia"], 1);
            assert_eq!(project.snapshot().params["numberOfPreviews"], 0);
            assert_eq!(project.snapshot().params["outputFormat"], "png");
        }
        let uploads = fixture.http.lock().clone();
        let upload = uploads
            .iter()
            .find(|r| r.path == "/v1/image/uploadUrl")
            .unwrap();
        assert_eq!(upload.query["type"], "startingImage");
        assert_eq!(upload.query["contentType"], "image/png");
        assert_eq!(
            client.projects.is_model_artifact_model_id(model),
            media == "model"
        );
        assert_eq!(wire["jobID"], json!(project.id()));
        let urls = project
            .wait_for_completion(Some(Duration::from_secs(3)))
            .await
            .unwrap();
        assert_eq!(urls, vec!["https://example.test/utility-result"]);
        assert_eq!(project.jobs().len(), 1);
        assert_eq!(project.job("UTILITY-RESULT").unwrap().media_type(), media);
        client.close().await.unwrap();
    }
}
