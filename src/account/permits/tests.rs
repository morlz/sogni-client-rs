use super::*;

#[test]
fn permits_match_compiled_typescript_acceptance_and_refusal_vectors() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/permits-5-86.json")).unwrap();
    assert_eq!(fixture["version"], "5.86.2");
    assert_eq!(
        fixture["commit"],
        "e830ed42e6b6973e659aa98c226b05f864e6ea97"
    );
    assert!(fixture["cases"].as_array().unwrap().len() > 150);
    for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let permit = &case["permit"];
        let testnet = case["testnet"].as_bool().unwrap();
        let result = if case["approve"] == true {
            approve(
                permit,
                case["wallet"].as_str().unwrap(),
                case["provider"].as_str().unwrap(),
                testnet,
                case["now"].as_u64().unwrap(),
            )
        } else {
            withdraw(permit, &case["payload"], testnet)
        };
        assert_eq!(result.is_err(), case["rejected"] == true, "case {index}");
        if let Err(crate::Error::Api(error)) = result {
            assert_eq!(error.status, 400);
            assert_eq!(error.error_code, 0);
            assert_eq!(
                error.message,
                "Unexpected signing request; nothing was signed"
            );
        }
    }
}
