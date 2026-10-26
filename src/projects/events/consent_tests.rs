use super::*;
use crate::{SogniClient, is_model_consent_required_error};

fn refusal(project_id: &str, job_id: Option<&str>, code: Value, requirement: Value) -> Value {
    let mut data = json!({
        "jobID":project_id, "error":code,
        "error_message":"Review and accept the agreement in a Sogni app.",
        "consentRequired":requirement,
        "providerResponse":{"private":true}, "diagnostics":"private detail",
    });
    if let Some(job_id) = job_id {
        data["imgID"] = json!(job_id);
    }
    data
}

#[tokio::test]
async fn model_consent_5_63_survives_tracked_failures_and_sanitized_public_events() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    for model_id in [
        "seedance-2-5-uncensored",
        "seedance-2-0-mini-uncensored",
        "seedream-5-0-pro-uncensored",
    ] {
        for job_id in [Some("RENDER"), None] {
            for code in [json!(4103), json!("4103")] {
                let project = Project::new(
                    "PROJECT".into(),
                    json!({"type":if crate::is_seedream_model(model_id) {"image"} else {"video"},"modelId":model_id,"numberOfMedia":1}),
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
                let public =
                    json!({"key":"seedance-2-5-uncensored","version":2,"modelId":model_id});
                let mut incoming = public.clone();
                incoming["privateContext"] = json!({"internal":true});
                handle_job_error(
                    &client.projects.inner,
                    &refusal("PROJECT", job_id, code.clone(), incoming),
                );
                let normalized = json!({"code":4103,"message":"Review and accept the agreement in a Sogni app.","consentRequired":public});
                assert_eq!(project.status(), ProjectStatus::Failed);
                assert_eq!(project.snapshot().error, Some(normalized.clone()));
                if let Some(job_id) = job_id {
                    assert_eq!(
                        project.job(job_id).unwrap().snapshot().error,
                        Some(normalized.clone())
                    );
                    assert_eq!(project.job(job_id).unwrap().status(), JobStatus::Failed);
                } else {
                    assert!(project.jobs().is_empty());
                }
                let error = project.wait_for_completion(None).await.unwrap_err();
                assert!(is_model_consent_required_error(&error));
                let Error::Project(error) = error else {
                    panic!("expected project refusal")
                };
                assert_eq!(
                    error.consent_required().unwrap().model_id.as_deref(),
                    Some(model_id)
                );
                assert_eq!(error.payload, normalized);
                let event = events.try_recv().unwrap();
                assert_eq!(
                    event.data["error"], code,
                    "Rust raw wire codes retain their representation"
                );
                assert_eq!(event.data["consentRequired"], public);
                assert!(event.data.get("providerResponse").is_none());
                assert!(event.data.get("diagnostics").is_none());
                assert!(
                    events.try_recv().is_err(),
                    "the refusal must not be automatically retried"
                );
            }
        }
    }
    client.close().await.unwrap();
}

#[tokio::test]
async fn model_consent_5_63_untracked_events_keep_only_the_public_agreement() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let mut events = client.projects.subscribe();
    for job_id in [Some("UNTRACKED-RENDER"), None] {
        handle_job_error(
            &client.projects.inner,
            &refusal(
                "UNTRACKED",
                job_id,
                json!("4103"),
                json!({"key":"agreement","version":1,"privateFields":{"secret":true}}),
            ),
        );
        let event = events.try_recv().unwrap();
        assert_eq!(event.name, if job_id.is_some() { "job" } else { "project" });
        assert_eq!(
            event.data["consentRequired"],
            json!({"key":"agreement","version":1})
        );
        assert!(event.data.get("providerResponse").is_none());
        assert!(client.projects.inner.projects.read().is_empty());
        assert!(events.try_recv().is_err());
    }
    for malformed in [
        Value::Null,
        json!(false),
        json!("agreement"),
        json!({"key":false,"version":1}),
        json!({"key":"agreement"}),
        json!({"key":"agreement","version":"1"}),
    ] {
        handle_job_error(
            &client.projects.inner,
            &refusal("UNTRACKED", None, json!("4103"), malformed),
        );
        assert!(
            events
                .try_recv()
                .unwrap()
                .data
                .get("consentRequired")
                .is_none()
        );
    }
    handle_job_error(
        &client.projects.inner,
        &refusal("UNAVAILABLE", None, json!("4104"), Value::Null),
    );
    let event = events.try_recv().unwrap();
    assert_eq!(event.data["error"], "4104");
    assert!(event.data.get("consentRequired").is_none());
    assert!(client.projects.inner.projects.read().is_empty());
    client.close().await.unwrap();
}
