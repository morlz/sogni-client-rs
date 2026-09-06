use super::*;

#[test]
fn lower_case_caller_preserves_its_key_and_matches_upper_case_active_registry() {
    let input = "a3f360cb-7f84-4d56-b360-a047e7cfb3cd";
    let active = active_project_ids(&json!({
        "projects": [{"id": "A3F360CB-7F84-4D56-B360-A047E7CFB3CD"}]
    }))
    .unwrap();
    let mut result = BTreeMap::new();
    classify_exhausted_404s(&mut result, vec![input.to_owned()], Some(&active));
    assert_eq!(result.get(input), Some(&ProjectResolution::Active));
    assert_eq!(result.len(), 1);
}

#[test]
fn exhausted_404_stays_unknown_when_socket_lookup_is_unavailable_or_malformed() {
    for active in [
        None,
        active_project_ids(&json!({"projects": "malformed"})),
        active_project_ids(&json!({"unexpected": []})),
        active_project_ids(&json!({"projects": [{"id": "ACTIVE"}, {"id": null}]})),
    ] {
        let mut result = BTreeMap::new();
        classify_exhausted_404s(&mut result, vec!["REST-404".into()], active.as_ref());
        assert!(matches!(
            result.get("REST-404"),
            Some(ProjectResolution::Unknown { .. })
        ));
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
        "projects": [{"id": "active"}]
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
