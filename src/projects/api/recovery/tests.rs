use super::*;

#[test]
fn exhausted_404_is_lost_when_socket_lookup_is_unavailable_or_malformed() {
    for active in [
        None,
        active_project_ids(&json!({"projects": "malformed"})),
        active_project_ids(&json!({"unexpected": []})),
    ] {
        let mut result = BTreeMap::new();
        classify_exhausted_404s(&mut result, vec!["REST-404".into()], active.as_ref());
        assert_eq!(result.get("REST-404"), Some(&ProjectResolution::Lost));
    }
}

#[test]
fn final_socket_verdict_preserves_non_404_and_completed_results() {
    let finished = json!({"id": "FINISHED"});
    let mut result = BTreeMap::from([
        (
            "REST-500".into(),
            ProjectResolution::Unknown {
                error: "project status could not be verified".into(),
            },
        ),
        (
            "FINISHED".into(),
            ProjectResolution::Finished {
                project: finished.clone(),
            },
        ),
    ]);
    let active = active_project_ids(&json!({
        "projects": [{"id": "ACTIVE"}, {"invalid": true}]
    }))
    .expect("valid active-project fixture");

    classify_exhausted_404s(
        &mut result,
        vec!["ACTIVE".into(), "REST-404".into()],
        Some(&active),
    );

    assert_eq!(result.get("ACTIVE"), Some(&ProjectResolution::Active));
    assert_eq!(result.get("REST-404"), Some(&ProjectResolution::Lost));
    assert_eq!(
        result.get("REST-500"),
        Some(&ProjectResolution::Unknown {
            error: "project status could not be verified".into(),
        })
    );
    assert_eq!(
        result.get("FINISHED"),
        Some(&ProjectResolution::Finished { project: finished })
    );
}
