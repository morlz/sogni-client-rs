use super::*;
use crate::SogniClient;

const WATCHDOG: Duration = Duration::from_secs(5);

async fn client() -> SogniClient {
    SogniClient::builder()
        .api_key("fixture-key")
        .disable_socket(true)
        .build()
        .await
        .unwrap()
}

fn project(api: &ProjectsApi) -> Project {
    let project = Project::new(
        "PROJECT".into(),
        json!({"type":"image","numberOfMedia":1}),
        false,
        Arc::downgrade(&api.inner),
    );
    api.inner
        .projects
        .write()
        .insert(project.id(), project.clone());
    project
}

#[tokio::test]
async fn closing_or_aborting_retires_projects_and_ends_the_session_listener() {
    for abort in [false, true] {
        let client = client().await;
        let api = &client.projects;
        let project = project(api);
        let job = project.ensure_job("CHILD");
        project.update(
            |state| {
                state.status = ProjectStatus::Queued;
            },
            &["status"],
        );
        let reason = json!({"reason":"queued","message":"Waiting for a worker."});
        project.receive_queue(&json!({
            "waitingReason":reason,
            "jobWaitingReasons":[{"jobIndex":0,"imgID":"CHILD","waitingReason":reason}],
        }));
        assert!(project.waiting_reason().is_some());
        let listener = listen_for_project_events(&api.inner);
        let mut wait = Box::pin(project.wait_for_completion(None));
        assert!(futures_util::poll!(wait.as_mut()).is_pending());
        if abort {
            client.abort();
        } else {
            client.close().await.unwrap();
        }
        assert!(matches!(wait.await, Err(Error::Closed)));
        tokio::time::timeout(WATCHDOG, listener)
            .await
            .unwrap()
            .unwrap();
        assert!(api.tracked_projects().is_empty());
        let state = project.snapshot();
        assert_eq!(state.status, ProjectStatus::Failed);
        assert!(state.waiting_reason.is_none() && state.job_waiting_reasons.is_empty());
        let message = state.error.unwrap()["message"].as_str().unwrap().to_owned();
        assert!(message.contains("was closed") && !message.contains("session ended"));
        assert_eq!(job.status(), JobStatus::Failed);
        assert!(matches!(
            project.wait_for_completion(None).await,
            Err(Error::Closed)
        ));
        assert!(matches!(project.cancel().await, Err(Error::Closed)));
        assert!(matches!(job.get_result_url().await, Err(Error::Closed)));
    }
}

#[tokio::test]
async fn disposal_after_a_server_refusal_preserves_the_original_error_for_waiters() {
    let client = client().await;
    let api = &client.projects;
    let project = project(api);
    let mut wait = Box::pin(project.wait_for_completion(None));
    assert!(futures_util::poll!(wait.as_mut()).is_pending());
    let limitation = "Daily fair-use capacity on the Fast network is used up.";
    result::handle_job_error(
        &api.inner,
        &json!({
            "jobID":project.id(), "isFromWorker":false, "error":"4087",
            "error_message":limitation, "subscriptionLimit":true,
            "requiredPlans":["unlimited_pro"], "feature":"daily_fair_use",
            "limitation":limitation,
        }),
    );
    // The server failure and disposal happen before the notified caller polls.
    // It still gets the service's actionable refusal, not an account-change
    // error or a generic closure replacing the already known result.
    client.close().await.unwrap();
    let Error::Project(error) = wait.await.unwrap_err() else {
        panic!("the service refusal must survive disposal");
    };
    assert_eq!(error.code, Some(json!(4087)));
    assert_eq!(error.message, limitation);
    assert_eq!(error.payload["limitation"], limitation);
    api.clear_previous_sessions();
    assert_eq!(project.snapshot().error.unwrap()["message"], limitation);
    let Error::Project(error) = project.wait_for_completion(None).await.unwrap_err() else {
        panic!("the retained refusal must remain available");
    };
    assert_eq!(error.code, Some(json!(4087)));
}

#[tokio::test]
async fn zero_code_server_refusals_survive_disposal_for_waiters_and_retained_handles() {
    for code in [json!(0), json!("0")] {
        for abort in [false, true] {
            let client = client().await;
            let api = &client.projects;
            let project = project(api);
            let mut wait = Box::pin(project.wait_for_completion(None));
            assert!(futures_util::poll!(wait.as_mut()).is_pending());
            let message = "The generation could not be accepted.";
            result::handle_job_error(
                &api.inner,
                &json!({
                    "jobID":project.id(), "isFromWorker":false,
                    "error":code, "error_message":message,
                }),
            );
            let original = project.snapshot().error.unwrap();
            assert_eq!(original["code"], 0);
            if abort {
                client.abort();
            } else {
                client.close().await.unwrap();
            }
            let Error::Project(error) = wait.await.unwrap_err() else {
                panic!("a zero-code service refusal must survive disposal");
            };
            assert_eq!(error.message, message);
            assert_eq!(error.payload, original);
            api.clear_previous_sessions();
            let Error::Project(error) = project.wait_for_completion(None).await.unwrap_err() else {
                panic!("a retained zero-code refusal must remain available");
            };
            assert_eq!(error.payload, original);
        }
    }
}

#[tokio::test]
async fn compact_recovered_failure_survives_disposal_with_its_original_reason() {
    let client = client().await;
    let api = &client.projects;
    let project = project(api);
    let job = project.ensure_job("CHILD");
    let mut wait = Box::pin(project.wait_for_completion(None));
    assert!(futures_util::poll!(wait.as_mut()).is_pending());
    let reason = "The generation could not be completed.";
    replay_recovered(
        &project,
        &json!({
            "id":project.id(), "status":"failed", "finished":true, "reason":reason,
        }),
        false,
    );
    let original = project.snapshot().error.unwrap();
    assert_eq!(original, json!({"code":0,"message":reason}));
    assert_eq!(job.status(), JobStatus::Failed);
    client.close().await.unwrap();
    let Error::Project(error) = wait.await.unwrap_err() else {
        panic!("the recovered terminal failure must survive disposal");
    };
    assert_eq!(error.payload, original);
    api.clear_previous_sessions();
    let Error::Project(error) = project.wait_for_completion(None).await.unwrap_err() else {
        panic!("the retained recovered failure must remain available");
    };
    assert_eq!(error.payload, original);
}

#[tokio::test]
async fn locally_ended_account_session_remains_distinct_from_a_service_refusal_and_closure() {
    let client = client().await;
    let api = &client.projects;
    let project = project(api);
    let mut wait = Box::pin(project.wait_for_completion(None));
    assert!(futures_util::poll!(wait.as_mut()).is_pending());
    api.inner.client.clear_auth();
    api.clear_previous_sessions();
    let Error::Project(error) = wait.await.unwrap_err() else {
        panic!("sign-out must preserve the account session ending outcome");
    };
    assert!(error.message.contains("account session ended"));
    assert!(!error.message.contains("was closed"));
    assert!(matches!(
        project.wait_for_completion(None).await,
        Err(Error::Project(_))
    ));
    client.close().await.unwrap();
    // Its local code-zero session error must never be mistaken for a service
    // refusal merely because the same client was disposed afterward.
    assert!(matches!(
        project.wait_for_completion(None).await,
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn background_cancellation_does_not_hide_a_real_http_rejection() {
    let client = client().await;
    let owner = client.projects.inner.client.rest.request_session();
    assert!(!ownerless_error(
        &Error::InvalidInput("invalid request".into()),
        &owner
    ));
    client.close().await.unwrap();
    assert!(ownerless_error(&Error::Closed, &owner));
    assert!(ownerless_error(
        &Error::InvalidInput("account session changed".into()),
        &owner
    ));
    for status in [401, 4087, 429, 503] {
        let rejection = crate::ApiError::new(status, json!({"message":"request refused"}));
        assert!(!ownerless_error(&rejection.into(), &owner));
    }
    assert!(!ownerless_error(
        &Error::Transport("connection failed".into()),
        &owner
    ));
}
