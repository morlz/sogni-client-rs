use super::*;
use crate::SogniClient;

fn failure(project_id: &str, job_id: Option<&str>, category: Value) -> Value {
    let mut data = json!({
        "jobID":project_id, "isFromWorker":false,
        "error":"5061", "error_message":"Service declined this generation",
        "vendorFailureCategory":category,
        "vendorFailureReason":"Private detail", "vendorResponse":{"internal":true},
    });
    if let Some(id) = job_id {
        data["imgID"] = json!(id);
    }
    data
}

#[tokio::test]
async fn tracked_vendor_categories_survive_job_project_and_public_error_events() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    for category in [
        "content_policy",
        "input_validation",
        "timeout",
        "result_storage",
        "cancelled",
        "vendor_failed",
        "asset_resolution",
        "vendor_transient",
        "future_category",
    ] {
        for job_id in [Some("RENDER"), None] {
            let project = Project::new(
                "PROJECT".into(),
                json!({"type":"image","numberOfMedia":1}),
                false,
                Arc::downgrade(&client.projects.inner),
            );
            client
                .projects
                .inner
                .projects
                .write()
                .insert(project.id(), project.clone());
            let mut events = client.projects.subscribe();
            handle_job_error(
                &client.projects.inner,
                &failure("PROJECT", job_id, json!(category)),
            );
            let expected = json!({"code":5061,"message":"Service declined this generation","vendorFailureCategory":category});
            assert_eq!(project.snapshot().error, Some(expected.clone()));
            if let Some(job_id) = job_id {
                assert_eq!(
                    project.job(job_id).unwrap().snapshot().error,
                    Some(expected)
                );
            }
            let event = events.try_recv().unwrap();
            assert_eq!(event.name, "job");
            assert_eq!(
                event.data["error"], "5061",
                "Rust wire code stays unchanged"
            );
            assert_eq!(event.data["vendorFailureCategory"], category);
            assert!(event.data.get("vendorFailureReason").is_none());
            assert!(event.data.get("vendorResponse").is_none());
            assert!(events.try_recv().is_err());
            if job_id.is_none() {
                assert!(project.jobs().is_empty());
            }
        }
    }
    client.close().await.unwrap();
}

#[tokio::test]
async fn untracked_errors_report_public_categories_without_creating_phantom_jobs() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let mut events = client.projects.subscribe();
    for job_id in [Some("UNTRACKED-RENDER"), None] {
        handle_job_error(
            &client.projects.inner,
            &failure("UNTRACKED", job_id, json!("future_category")),
        );
        let event = events.try_recv().unwrap();
        assert_eq!(event.name, if job_id.is_some() { "job" } else { "project" });
        assert_eq!(event.data["vendorFailureCategory"], "future_category");
        assert!(event.data.get("vendorFailureReason").is_none());
        assert!(event.data.get("vendorResponse").is_none());
        assert!(client.projects.inner.projects.read().is_empty());
        assert!(events.try_recv().is_err());
    }
    for category in [
        Value::Null,
        json!(""),
        json!(" \t"),
        json!(false),
        json!(42),
        json!({}),
        json!([]),
    ] {
        for job_id in [Some("UNTRACKED-RENDER"), None] {
            handle_job_error(
                &client.projects.inner,
                &failure("UNTRACKED", job_id, category.clone()),
            );
            let event = events.try_recv().unwrap();
            assert!(event.data.get("vendorFailureCategory").is_none());
            assert!(event.data.get("vendorFailureReason").is_none());
            assert!(event.data.get("vendorResponse").is_none());
            assert!(client.projects.inner.projects.read().is_empty());
            assert!(events.try_recv().is_err());
        }
    }
    client.close().await.unwrap();
}

#[tokio::test]
async fn malformed_vendor_categories_are_omitted_and_existing_error_contracts_stay_intact() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    for category in [
        Value::Null,
        json!(""),
        json!(" \t"),
        json!(false),
        json!(42),
        json!({}),
        json!([]),
    ] {
        let project = Project::new(
            "PROJECT".into(),
            json!({"type":"image","numberOfMedia":1}),
            false,
            Arc::downgrade(&client.projects.inner),
        );
        client
            .projects
            .inner
            .projects
            .write()
            .insert(project.id(), project.clone());
        let mut events = client.projects.subscribe();
        let mut data = failure("PROJECT", None, category);
        data["error"] = json!(4081);
        data["subscriptionLimit"] = json!(true);
        data["requiredPlans"] = json!(["unlimited_pro"]);
        data["feature"] = json!("video_4k_render");
        data["limitation"] = json!("Upgrade required");
        handle_job_error(&client.projects.inner, &data);
        let error = project.snapshot().error.unwrap();
        assert_eq!(error["code"], 4081);
        assert_eq!(error["subscriptionLimit"], true);
        assert_eq!(error["requiredPlans"], json!(["unlimited_pro"]));
        assert_eq!(error["feature"], "video_4k_render");
        assert_eq!(error["limitation"], "Upgrade required");
        assert!(error.get("vendorFailureCategory").is_none());
        assert!(
            events
                .try_recv()
                .unwrap()
                .data
                .get("vendorFailureCategory")
                .is_none()
        );
    }
    let project = Project::new(
        "CANCELLED".into(),
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
    handle_job_error(
        &client.projects.inner,
        &json!({"jobID":"CANCELLED","error":"artistCanceled","error_message":"artistCanceled"}),
    );
    let error = project.snapshot().error.unwrap();
    assert_eq!(error["code"], 5004);
    assert_eq!(error["originalCode"], "artistCanceled");
    assert!(error.get("vendorFailureCategory").is_none());
    client.close().await.unwrap();
}

#[tokio::test]
async fn malformed_public_error_fields_cannot_echo_private_objects() {
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
    let mut events = client.projects.subscribe();
    let mut data = failure("PROJECT", None, json!("vendor_failed"));
    for field in [
        "error",
        "error_message",
        "isFromWorker",
        "subscriptionLimit",
        "feature",
        "limitation",
    ] {
        data[field] = json!({"private":"provider-diagnostic-marker"});
    }
    data["requiredPlans"] = json!([{"private":"provider-diagnostic-marker"}]);
    handle_job_error(&client.projects.inner, &data);
    let normalized = project.snapshot().error.unwrap();
    assert_eq!(
        normalized,
        json!({"code":5000,"message":"Project failed","vendorFailureCategory":"vendor_failed"})
    );
    let event = events.try_recv().unwrap();
    assert_eq!(
        event.data,
        json!({"jobID":"PROJECT","vendorFailureCategory":"vendor_failed"})
    );
    assert!(
        !event
            .data
            .to_string()
            .contains("provider-diagnostic-marker")
    );
    data["jobID"] = json!("UNTRACKED");
    handle_job_error(&client.projects.inner, &data);
    assert_eq!(
        events.try_recv().unwrap().data,
        json!({"jobID":"UNTRACKED","vendorFailureCategory":"vendor_failed"})
    );
    client.close().await.unwrap();
}
