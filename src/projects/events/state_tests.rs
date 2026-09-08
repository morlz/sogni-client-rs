use super::*;
use crate::SogniClient;

#[tokio::test]
async fn model_loading_phases_have_typed_views_and_keep_raw_future_payloads() {
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
    client
        .projects
        .inner
        .projects
        .write()
        .insert(project.id(), project.clone());
    for preparation in [
        json!({"phase":"downloadingAssets","assetType":"lora","requested":2,"cached":1,"total":1,"completed":0,"current":1,"currentProgress":50}),
        json!({"phase":"unloadingModel","model":"Flux","step":"start"}),
        json!({"phase":"unloadingModel","model":"Flux","step":"end","elapsedSec":65.5}),
        json!({"phase":"loadingModel","model":"WAN21","step":"start"}),
        json!({"phase":"loadingModel","model":"WAN21","step":"end","elapsedSec":30.5}),
    ] {
        handle_job_state(
            &client.projects.inner,
            &json!({
                "jobID":"PROJECT","imgID":"JOB","type":"initiatingModel","preparation":preparation,
            }),
        );
        let job = project.job("JOB").unwrap();
        assert_eq!(job.status(), JobStatus::Initiating);
        let typed = job.preparation().unwrap();
        let expected: JobPreparation = serde_json::from_value(preparation.clone()).unwrap();
        assert_eq!(typed, expected);
        assert_eq!(job.snapshot().extra["preparation"], preparation);
        assert_eq!(job.snapshot().preparation(), Some(expected));
    }
    handle_job_state(
        &client.projects.inner,
        &json!({
            "jobID":"PROJECT","imgID":"JOB","type":"initiatingModel","preparation":{"phase":"futurePhase"},
        }),
    );
    let job = project.job("JOB").unwrap();
    assert!(job.preparation().is_none());
    assert_eq!(job.snapshot().extra["preparation"]["phase"], "futurePhase");
    client.close().await.unwrap();
}

#[tokio::test]
async fn late_active_and_completion_events_do_not_regress_terminal_states() {
    let client = SogniClient::builder()
        .app_id("local-parity-fixture")
        .api_key("local-fixture")
        .defer_socket_start(true)
        .build()
        .await
        .unwrap();
    for (parent_status, child_status) in [
        (ProjectStatus::Completed, JobStatus::Completed),
        (ProjectStatus::Failed, JobStatus::Failed),
        (ProjectStatus::Canceled, JobStatus::Canceled),
    ] {
        let project = Project::new(
            "PROJECT".into(),
            json!({"type":"image", "numberOfMedia":1}),
            false,
            Arc::downgrade(&client.projects.inner),
        );
        project.update(|state| state.status = parent_status, &["status"]);
        let job = project.ensure_job("IMAGE");
        job.update(
            |state| {
                state.status = child_status;
                state.step = 28.0;
                state.result_url = Some("https://media.sogni.ai/fixture.png".into());
                state.is_nsfw = true;
                state.nsfw_detected = true;
            },
            &["status"],
        );
        client
            .projects
            .inner
            .projects
            .write()
            .insert("PROJECT".into(), project.clone());
        for kind in [
            "queued",
            "initiatingModel",
            "jobStarted",
            "jobCompleted",
            "jobProgress",
        ] {
            let event =
                json!({"jobID":"PROJECT", "imgID":"IMAGE", "type":kind, "step":2, "progress":5});
            handle_job_state(&client.projects.inner, &event);
            handle_job_progress(&client.projects.inner, &event);
            assert_eq!(project.status(), parent_status, "late {kind}");
            assert_eq!(job.status(), child_status, "late {kind}");
            assert_eq!(job.snapshot().step, 28.0);
            assert!(job.result_url().is_some());
        }
        replay_recovered(
            &project,
            &json!({
                "status":"processing", "workerJobs":[{"imgID":"IMAGE", "status":"jobProgress"}]
            }),
            false,
        );
        assert_eq!(project.status(), parent_status);
        assert_eq!(job.status(), child_status);
        assert!(job.snapshot().is_nsfw);
        assert!(job.snapshot().nsfw_detected);
    }
    client.close().await.unwrap();
}
