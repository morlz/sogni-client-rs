use super::{
    SogniClient,
    project_wire_server::{Fixture, KEY},
};
use crate::{AssetRole, Error, MediaSource, ProjectRequest, SubmissionPhase};
use std::{path::PathBuf, time::Duration};
use url::Url;

#[tokio::test]
async fn detailed_submission_separates_local_preparation_assets_and_uncertain_send() {
    let mut fixture = Fixture::start().await;
    let client = SogniClient::builder()
        .api_key(KEY)
        .rest_endpoint(Url::parse(&format!("http://{}/", fixture.address)).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{}/", fixture.address)).unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(2))
        .connect_timeout(Duration::from_millis(30))
        .build()
        .await
        .unwrap();
    let request = || {
        ProjectRequest::image("z_image_turbo_bf16", "fixture")
            .steps(8)
            .guidance(1.0)
            .dimensions(1024, 1024)
    };
    let invalid = client
        .projects
        .create_with_id_detailed("invalid", request())
        .await
        .unwrap_err();
    assert_eq!(invalid.phase(), SubmissionPhase::Prepare);
    assert!(matches!(invalid.cause(), Error::InvalidInput(_)));
    let absent = tempfile::tempdir().unwrap().path().join("absent.png");
    let media = client
        .projects
        .create_with_id_detailed(
            "00000000-0000-4000-8000-000000000001",
            request().asset(
                AssetRole::StartingImage,
                MediaSource::Path(PathBuf::from(&absent)),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(media.phase(), SubmissionPhase::AssetUpload);
    assert!(matches!(media.cause(), Error::Io(_)));
    assert!(client.projects.tracked_projects().is_empty());
    assert!(fixture.wire.try_recv().is_err());
    client.abort();
    let send = client
        .projects
        .create_with_id_detailed("00000000-0000-4000-8000-000000000002", request())
        .await
        .unwrap_err();
    assert_eq!(send.phase(), SubmissionPhase::Send);
    assert!(send.phase().request_may_have_been_sent());
    assert!(client.projects.tracked_projects().is_empty());
    client.close().await.unwrap();
    assert!(fixture.wire.try_recv().is_err());
}
