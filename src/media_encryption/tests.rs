use super::*;
use crate::{Error, SogniClient};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode, header},
    routing::get,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn vector() -> Value {
    serde_json::from_str(include_str!("fixtures/upstream-5-84.json")).unwrap()
}

fn field<const N: usize>(fixture: &Value, key: &str) -> [u8; N] {
    let encoded = fixture[key].as_str().unwrap();
    assert_eq!(encoded.len(), N * 2);
    std::array::from_fn(|i| u8::from_str_radix(&encoded[i * 2..i * 2 + 2], 16).unwrap())
}

fn plaintext(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i * 31 + (i >> 8) * 7 + 3) & 0xff) as u8)
        .collect()
}

fn vector_account_key(fixture: &Value) -> MediaKey {
    // The upstream fixture's synthetic wallet seed is not the account media key.
    let seed = field::<32>(fixture, "walletPrivateKey");
    let key = codec::derive::<32>(&seed, b"sogni-media-encryption", b"account-key-v1").unwrap();
    MediaKey::from_bytes(*key)
}

#[test]
fn media_container_matches_upstream_known_answers_and_rejects_modifications() {
    use MediaEncryptionError::*;
    let v = vector();
    let key = MediaKey::from_bytes(field(&v, "projectKey"));
    let account = recipient::AccountKey::new(vector_account_key(&v)).unwrap();
    let old_recipient = account.wrap(&key, field(&v, "recipientNonce")).unwrap();
    assert_eq!(old_recipient, field::<69>(&v, "recipient"));
    let new_account = recipient::AccountKey::new(MediaKey::from_bytes([7; 32])).unwrap();
    assert!(matches!(
        recipient::unwrap(std::slice::from_ref(&new_account), &old_recipient),
        Err(WrongAccountKey)
    ));
    assert_eq!(
        recipient::unwrap(&[new_account, account], &old_recipient)
            .unwrap()
            .as_bytes(),
        key.as_bytes()
    );

    for case in v["files"].as_array().unwrap() {
        let data = plaintext(case["plaintextLength"].as_u64().unwrap() as usize);
        let mime = case["contentType"].as_str().unwrap();
        let sealed = codec::seal(
            &data,
            &key,
            mime,
            if case["hasRecipient"] == true {
                &old_recipient
            } else {
                &[]
            },
            case["segmentSizeLog2"].as_u64().unwrap() as u8,
            field(&v, "salt"),
            field(&v, "noncePrefix"),
        )
        .unwrap();
        assert_eq!(sealed.len() as u64, case["ciphertextLength"]);
        assert_eq!(
            format!("{:x}", Sha256::digest(&sealed)),
            case["ciphertextSha256"].as_str().unwrap()
        );
        let result = decrypt_media(&sealed, &key).unwrap();
        assert_eq!(result.data, data);
        assert_eq!(result.content_type, mime);
        let mut corrupt = sealed.to_vec();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(matches!(
            decrypt_media(&corrupt, &key),
            Err(AuthenticationFailed)
        ));
        assert!(decrypt_media(&sealed[..sealed.len() - 1], &key).is_err());
        if !mime.is_empty() {
            let mut corrupt = sealed.to_vec();
            corrupt[50] = b'X';
            assert!(matches!(
                decrypt_media(&corrupt, &key),
                Err(AuthenticationFailed)
            ));
        }
    }
}

#[test]
fn input_reuse_and_key_wrapping_match_upstream_vectors() {
    let v = vector();
    let project = MediaKey::from_bytes(field(&v, "projectKey"));
    let account = recipient::AccountKey::new(vector_account_key(&v)).unwrap();
    for case in v["inputs"].as_array().unwrap() {
        let data = plaintext(case["plaintextLength"].as_u64().unwrap() as usize);
        let mime = case["contentType"].as_str().unwrap();
        let input = account.input_material(&data, mime).unwrap();
        assert_eq!(*input.key.as_bytes(), field::<32>(case, "inputKey"));
        assert_eq!(input.salt, field::<32>(case, "salt"));
        assert_eq!(input.nonce_prefix, field::<7>(case, "noncePrefix"));
        let recipient = account.wrap(&input.key, input.recipient_nonce).unwrap();
        let wrapped =
            recipient::wrap_input(&project, &input.key, field(&v, "inputWrapNonce")).unwrap();
        assert_eq!(wrapped, field::<60>(case, "wrappedInputKey"));
        let bytes = codec::seal(
            &data,
            &input.key,
            mime,
            &recipient,
            20,
            input.salt,
            input.nonce_prefix,
        )
        .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            case["ciphertextSha256"].as_str().unwrap()
        );
        assert_ne!(
            *account
                .input_material(&data, "application/octet-stream")
                .unwrap()
                .key
                .as_bytes(),
            *input.key.as_bytes()
        );
    }
}

async fn serve_seekable(State(data): State<bytes::Bytes>, headers: HeaderMap) -> Response<Body> {
    assert!(!headers.contains_key(header::AUTHORIZATION));
    assert!(!headers.contains_key("api-key"));
    assert!(!headers.contains_key(header::COOKIE));
    assert!(!headers.contains_key("x-sogni-media"));
    let (start, end) = headers[header::RANGE]
        .to_str()
        .unwrap()
        .strip_prefix("bytes=")
        .unwrap()
        .split_once('-')
        .unwrap();
    let start = start.parse::<usize>().unwrap();
    let end = end.parse::<usize>().unwrap().min(data.len() - 1);
    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", data.len()),
        )
        .body(Body::from(data.slice(start..=end)))
        .unwrap()
}

#[tokio::test]
async fn encrypted_range_reads_cross_segments_without_credentials_and_stop_when_locked() {
    let v = vector();
    let data = plaintext(150_000);
    let key = MediaKey::from_bytes(field(&v, "projectKey"));
    let account_key = vector_account_key(&v);
    let recipient = field::<69>(&v, "recipient");
    let ciphertext = codec::seal(
        &data,
        &key,
        "video/mp4",
        &recipient,
        16,
        field(&v, "salt"),
        field(&v, "noncePrefix"),
    )
    .unwrap();
    let app = Router::new()
        .route(
            "/v1/account/me",
            get(|| async { axum::Json(json!({"data":{"username":"fixture"}})) }),
        )
        .route("/range", get(serve_seekable))
        .route(
            "/bad",
            get(|| async { ([(header::CONTENT_RANGE, "bytes 0-818/1000")], "no ranges") }),
        )
        .with_state(ciphertext);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = SogniClient::builder()
        .disable_socket(true)
        .api_key("fixture-account")
        .rest_endpoint(url.parse().unwrap())
        .build()
        .await
        .unwrap();
    client
        .media_encryption
        .set_account_key(account_key)
        .unwrap();
    let media = client
        .media_encryption
        .open_decrypted(format!("{url}/range").parse().unwrap())
        .await
        .unwrap();
    assert_eq!(media.content_type(), "video/mp4");
    assert_eq!(media.size(), data.len() as u64);
    assert_eq!(
        media.read(65_500..65_600).await.unwrap(),
        &data[65_500..65_600]
    );
    assert!(matches!(
        client
            .media_encryption
            .open_decrypted(format!("{url}/bad").parse().unwrap())
            .await,
        Err(Error::MediaEncryption(MediaEncryptionError::Unsupported(_)))
    ));
    client.media_encryption.lock();
    assert!(matches!(
        media.read(0..32).await,
        Err(Error::MediaEncryption(MediaEncryptionError::Locked))
    ));
    client.close().await.unwrap();
    server.abort();
}

#[test]
fn redaction_preserves_recovery_fields_without_exposing_the_wire_key() {
    let raw = json!({
        "network":"fast","mediaEncryption":{"version":1,"key":"fixture-key","recipient":"recipient"},
        "keyFrames":[{"modelID":"image-model"}]
    });
    let mut record = json!({
        "clientRequestData":crate::utils::b64_json_encode(&raw).unwrap(),
        "other":[{"mediaEncryption":{"key":"fixture-key"}}]
    });
    redact_media_keys(&mut record);
    let decoded =
        crate::utils::b64_json_decode(record["clientRequestData"].as_str().unwrap()).unwrap();
    assert_eq!(decoded["mediaEncryption"], true);
    assert_eq!(decoded["keyFrames"], raw["keyFrames"]);
    assert_eq!(record["other"][0]["mediaEncryption"], true);
    assert!(!decoded.to_string().contains("fixture-key"));
}
