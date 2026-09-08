use super::*;
use crate::SogniClient;

#[tokio::test]
async fn repeated_terminal_recovery_keeps_existing_error_and_settles_remaining_children() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let project = Project::new(
        "PROJECT".into(),
        json!({"type":"image"}),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    let pending = project.ensure_job("PENDING");
    let error = json!({"code":42,"message":"Original failure"});
    project.update(
        |s| {
            s.status = ProjectStatus::Failed;
            s.error = Some(error.clone());
        },
        &["status", "error"],
    );
    replay_recovered(
        &project,
        &json!({"status":"errored","reason":"later reason"}),
        false,
    );
    assert_eq!(pending.status(), JobStatus::Failed);
    assert_eq!(pending.snapshot().error, Some(error.clone()));
    assert_eq!(project.snapshot().error, Some(error));
    client.close().await.unwrap();
}

#[tokio::test]
async fn terminal_recovery_preserves_reasons_and_settles_unfinished_children_and_waiters() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    for (status, reason, code, message, child_status) in [
        (
            "cancelled",
            " artistCanceled ",
            0,
            "artistCanceled",
            JobStatus::Canceled,
        ),
        ("errored", " 4078 ", 4078, "4078", JobStatus::Failed),
        ("canceled", "", 0, "Project canceled", JobStatus::Canceled),
        ("failed", "", 0, "Project failed", JobStatus::Failed),
        (
            "failed",
            "9007199254740992",
            0,
            "9007199254740992",
            JobStatus::Failed,
        ),
    ] {
        let project = Project::new(
            "PROJECT".into(),
            json!({"type":"image","numberOfMedia":3}),
            false,
            Arc::downgrade(&client.projects.inner),
        );
        let completed = project.ensure_job("COMPLETED");
        completed.update(
            |s| {
                s.status = JobStatus::Completed;
                s.result_url = Some("https://example.test/result.png".into());
            },
            &["status"],
        );
        let pending = project.ensure_job("PENDING");
        let mut waiting = std::pin::pin!(project.wait_for_completion(Some(Duration::from_secs(1))));
        assert!(futures_util::poll!(&mut waiting).is_pending());
        replay_recovered(
            &project,
            &json!({"status":status,"reason":reason,"completedWorkerJobs":[]}),
            false,
        );
        let expected = json!({"code":code,"message":message});
        assert_eq!(project.snapshot().error, Some(expected.clone()));
        assert_eq!(pending.status(), child_status);
        assert_eq!(pending.snapshot().error, Some(expected));
        assert_eq!(completed.status(), JobStatus::Completed);
        for error in [
            waiting.await.unwrap_err(),
            project.wait_for_completion(None).await.unwrap_err(),
        ] {
            let Error::Project(error) = error else {
                panic!("expected terminal project error")
            };
            assert_eq!(error.payload["message"], message);
            assert_eq!(error.payload["code"], code);
        }
    }
    client.close().await.unwrap();
}
