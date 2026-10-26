use super::*;

fn fixtures() -> Value {
    serde_json::from_str(include_str!("fixtures/wallet-typescript.json")).unwrap()
}

#[tokio::test]
async fn wallet_addresses_hashes_and_signatures_match_typescript() {
    let fixtures = fixtures();
    assert_eq!(fixtures["version"], "5.69.1");
    assert_eq!(
        fixtures["commit"],
        "5970a1bd40b88f678719188969c8633bc313b665"
    );
    for case in fixtures["wallets"].as_array().unwrap() {
        let wallet = wallet(
            case["username"].as_str().unwrap(),
            case["password"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            format!("{:#x}", wallet.address()),
            case["address"].as_str().unwrap()
        );
    }
    for case in fixtures["cases"].as_array().unwrap() {
        let owner = &fixtures["wallets"][case["wallet"].as_u64().unwrap() as usize];
        let wallet = wallet(
            owner["username"].as_str().unwrap(),
            owner["password"].as_str().unwrap(),
        )
        .unwrap();
        let data: TypedData = serde_json::from_value(case["data"].clone()).unwrap();
        assert_eq!(
            format!("0x{}", hex::encode(data.eip712_signing_hash().unwrap())),
            case["hash"],
            "{}",
            case["name"]
        );
        assert_eq!(
            format!("0x{}", wallet.sign_typed_data(&data).await.unwrap()),
            case["signature"],
            "{}",
            case["name"]
        );
        let signature =
            hex::decode(case["signature"].as_str().unwrap().trim_start_matches("0x")).unwrap();
        let recovery =
            secp256k1::ecdsa::RecoveryId::try_from(i32::from(signature[64] - 27)).unwrap();
        let signature =
            secp256k1::ecdsa::RecoverableSignature::from_compact(&signature[..64], recovery)
                .unwrap();
        let public = secp256k1::Secp256k1::new()
            .recover_ecdsa(
                secp256k1::Message::from_digest(data.eip712_signing_hash().unwrap()),
                &signature,
            )
            .unwrap()
            .serialize_uncompressed();
        assert_eq!(
            format!(
                "0x{}",
                hex::encode(&typed_data::keccak256(&public[1..])[12..])
            ),
            owner["address"]
        );
    }
}

#[test]
fn invalid_typed_values_return_errors_before_signing() {
    let base = fixtures()["cases"][0]["data"].clone();
    for (kind, value) in [
        ("uint8", json!(256)),
        ("uint256", json!(-1)),
        ("uint256", json!(("1".repeat(80)))),
        ("int8", json!(-129)),
        ("int8", json!(128)),
        ("bool", json!("true")),
        ("bytes2", json!("0x010203")),
        ("bytes2", json!([1, 256])),
        ("address", json!("not-an-address")),
        ("uint256[2]", json!([1])),
        ("uint256[]", json!("not-an-array")),
        ("Missing[]", json!([])),
        ("uint7[]", json!([])),
        ("bytes33[]", json!([])),
        ("uint256[0]", json!([])),
        ("uint256[broken][]", json!([])),
        ("Missing", json!({})),
    ] {
        let mut data = base.clone();
        data["types"] = json!({"Input":[{"name":"value","type":kind}]});
        data["primaryType"] = json!("Input");
        data["message"] = json!({"value":value});
        let data: TypedData = serde_json::from_value(data).unwrap();
        assert!(data.eip712_signing_hash().is_err(), "{kind}");
    }
    let mut data = base;
    data["primaryType"] = json!("Undefined");
    assert!(
        serde_json::from_value::<TypedData>(data)
            .unwrap()
            .eip712_signing_hash()
            .is_err()
    );
}

#[test]
fn byte_array_domain_salt_preserves_existing_wallet_behavior() {
    let fixtures = fixtures();
    let case = fixtures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "domain-salt")
        .unwrap();
    let mut data = case["data"].clone();
    data["domain"]["salt"] = json!(vec![0x42; 32]);
    let data: TypedData = serde_json::from_value(data).unwrap();
    assert_eq!(
        format!("0x{}", hex::encode(data.eip712_signing_hash().unwrap())),
        case["hash"]
    );
}
