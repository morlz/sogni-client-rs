use super::*;
use crate::SogniClient;

#[tokio::test]
async fn late_active_and_completion_events_do_not_regress_terminal_states() {
    let client = SogniClient::builder()
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
