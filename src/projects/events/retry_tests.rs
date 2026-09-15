use super::*;
use crate::SogniClient;

#[tokio::test]
async fn retry_diagnostics_stay_internal_while_the_same_render_completes() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let project = Project::new(
        "PROJECT".into(),
        json!({"type":"image", "numberOfMedia":1, "steps":10}),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    client
        .projects
        .inner
        .projects
        .write()
        .insert(project.id(), project.clone());
    let job = project.ensure_job("OLD");
    job.update(
        |state| {
            state.extra.insert("jobIndex".into(), json!(0));
            state.status = JobStatus::Processing;
            state.step = 8.0;
            state.preview_url = Some("https://example.test/old-preview.png".into());
            state.provenance = Some(JobProvenance {
                sha256: Some("a".repeat(64)),
                ..Default::default()
            });
        },
        &[],
    );
    let mut public_events = client.projects.subscribe();
    let mut project_events = project.subscribe();
    let mut job_events = job.subscribe();
    let abandoned = json!({
        "jobID":"PROJECT", "imgID":"OLD", "jobIndex":0,
        "error":"workerDisconnected", "error_message":"Abandoned attempt failed",
    });

    handle_job_retry(&client.projects.inner, &abandoned);

    assert_eq!(project.status(), ProjectStatus::Pending);
    assert!(project.snapshot().error.is_none());
    assert_eq!(job.status(), JobStatus::Pending);
    assert!(job.snapshot().error.is_none());
    assert_eq!(job.snapshot().step, 0.0);
    assert!(job.snapshot().preview_url.is_none());
    assert!(job.provenance().is_none());
    assert!(matches!(
        public_events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    for events in [&mut project_events, &mut job_events] {
        assert_eq!(events.try_recv().unwrap().name, "updated");
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    // Late failure/progress from the abandoned worker cannot fail or revive it.
    handle_job_error(&client.projects.inner, &abandoned);
    handle_job_progress(
        &client.projects.inner,
        &json!({"jobID":"PROJECT", "imgID":"OLD", "jobIndex":0, "step":9}),
    );
    assert_eq!(project.status(), ProjectStatus::Pending);
    assert_eq!(job.status(), JobStatus::Pending);
    assert_eq!(job.snapshot().step, 0.0);
    assert!(public_events.try_recv().is_err());

    handle_job_state(
        &client.projects.inner,
        &json!({"jobID":"PROJECT", "imgID":"NEW", "jobIndex":0, "type":"jobStarted"}),
    );
    assert_eq!(
        job.id(),
        "NEW",
        "the original handle follows the logical render"
    );
    assert_eq!(project.jobs().len(), 1);
    assert_eq!(job.status(), JobStatus::Processing);
    handle_job_error(&client.projects.inner, &abandoned);
    assert_eq!(job.status(), JobStatus::Processing);
    assert!(job.snapshot().error.is_none());
    assert!(project.snapshot().error.is_none());

    handle_job_result(
        &client.projects.inner,
        &json!({
            "jobID":"PROJECT", "imgID":"NEW", "jobIndex":0,
            "resultUrl":"https://example.test/current.png",
        }),
    )
    .await;
    handle_job_state(
        &client.projects.inner,
        &json!({"jobID":"PROJECT", "type":"jobCompleted"}),
    );
    assert_eq!(job.status(), JobStatus::Completed);
    assert_eq!(project.jobs().len(), 1);
    assert_eq!(
        project
            .wait_for_completion(Some(Duration::from_secs(1)))
            .await
            .unwrap(),
        vec!["https://example.test/current.png"],
    );
    let mut saw_result = false;
    while let Ok(event) = public_events.try_recv() {
        assert!(event.data.get("error").is_none());
        assert!(event.data.get("error_message").is_none());
        saw_result |= event.name == "job" && event.data["imgID"] == "NEW";
    }
    assert!(saw_result);
    while let Ok(event) = project_events.try_recv() {
        assert_ne!(event.name, "jobRetry");
        assert_ne!(event.name, "jobFailed");
        assert!(event.data.get("error").is_none());
    }
    client.close().await.unwrap();
}
