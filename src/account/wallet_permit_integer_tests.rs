use serde_json::Value;

use super::{sign_dynamic, wallet};

#[tokio::test]
async fn integer_valued_json_decimals_match_compiled_typescript_permit_signatures() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/permit-integers-5-86.json")).unwrap();
    assert_eq!(fixture["version"], "5.86.2");
    assert_eq!(
        fixture["commit"],
        "e830ed42e6b6973e659aa98c226b05f864e6ea97"
    );
    let wallet = wallet("TestUser", "correct horse battery staple").unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let permit: Value = serde_json::from_str(case["wire"].as_str().unwrap()).unwrap();
        // The validator and signer must agree even when JSON preserves `.0`.
        if case["approve"] == true {
            super::super::permits::approve(
                &permit,
                case["payload"]["walletAddress"].as_str().unwrap(),
                "base",
                false,
                1_800_000_000,
            )
            .unwrap();
        } else {
            super::super::permits::withdraw(&permit, &case["payload"], false).unwrap();
        }
        let before = permit.clone();
        let signature = sign_dynamic(&wallet, &permit).await.unwrap();
        assert_eq!(signature, case["signature"].as_str().unwrap());
        assert_eq!(
            permit, before,
            "signing must not rewrite the submitted deadline"
        );
    }
}
