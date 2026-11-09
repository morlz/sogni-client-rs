use parking_lot::Mutex;
use serde_json::{Value, json};

use super::projection::{
    SubscriptionSource, apply_subscription_projection, map_socket_subscription,
};
#[cfg(feature = "wallet")]
use super::wallet_crypto::{amount_to_wei, wallet};
use super::{CurrentAccount, SubscriptionProjectionState};

mod auth_state;
#[cfg(feature = "wallet")]
mod cookie_rollback;
mod cookie_session;
#[cfg(feature = "wallet")]
mod login_572;
mod priority_history;
mod request_session;

#[cfg(feature = "wallet")]
#[test]
fn wallet_derivation_matches_the_typescript_client() {
    let derived = wallet("TestUser", "correct horse battery staple").expect("wallet");
    assert_eq!(
        format!("{:#x}", derived.address()),
        "0x930fbddbfa47bf6263330d551414750c2688be5a"
    );
    assert_eq!(
        derived.address(),
        wallet("testuser", "correct horse battery staple")
            .expect("lowercase wallet")
            .address()
    );
}

#[cfg(feature = "wallet")]
#[tokio::test]
async fn typed_signatures_match_the_typescript_wire_format() {
    let wallet = wallet("TestUser", "correct horse battery staple").unwrap();
    let client = crate::SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let fields = json!([
        {"name": "walletAddress", "type": "address"},
        {"name": "nonce", "type": "string"}
    ]);
    let message =
        json!({"walletAddress": format!("{:#x}", wallet.address()), "nonce": "alpha-test-nonce"});
    // Generated independently with ethers.js Wallet.signTypedData, using only
    // the public synthetic credentials from the wallet derivation fixture.
    let expected = "0x6b129f1e57545143aa6805c34422ab44779933880bd21c5f42e49729f6851d57355bcb5ae449c242533c59b9dfe2ccd92064dd939517609e9f1b1079d2f6c3f11b";
    let named = client
        .account
        .sign_named(&wallet, "Authentication", fields.clone(), message.clone())
        .await
        .unwrap();
    assert_eq!(named, expected);
    let permit = json!({
        "types": {"Authentication": fields},
        "domain": {"name": "Sogni AI", "version": "1", "chainId": 8453,
            "verifyingContract": "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"},
        "message": message
    });
    assert_eq!(
        super::wallet_crypto::sign_dynamic(&wallet, &permit)
            .await
            .unwrap(),
        expected
    );
    client.close().await.unwrap();
}

#[cfg(feature = "wallet")]
#[test]
fn decimal_amounts_match_ethers_parse_ether() {
    assert_eq!(amount_to_wei("1").expect("whole"), "1000000000000000000");
    assert_eq!(
        amount_to_wei(".5").expect("leading dot"),
        "500000000000000000"
    );
    assert_eq!(
        amount_to_wei("1.").expect("trailing dot"),
        "1000000000000000000"
    );
    assert_eq!(
        amount_to_wei("-0.25").expect("negative"),
        "-250000000000000000"
    );
    assert!(amount_to_wei("1.0000000000000000001").is_err());
}

#[test]
fn maps_socket_subscription_entitlements() {
    let mapped = map_socket_subscription(&json!({
        "active": true,
        "subscription": {
            "status": "cancelled",
            "tier": "unlimited_pro",
            "term": "monthly",
            "periodEnd": 1_800_000_000_000_i64,
        }
    }))
    .expect("subscription projection");
    assert_eq!(mapped["active"], true);
    assert_eq!(mapped["status"], "cancel_at_period_end");
    assert_eq!(mapped["cancelAtPeriodEnd"], true);
    assert_eq!(mapped["capabilities"]["unlimited"], true);
    assert!(mapped["currentPeriodEnd"].as_str().is_some());
}

#[test]
fn stale_subscription_reads_do_not_overwrite_socket_state() {
    let current = CurrentAccount::default();
    let state = Mutex::new(SubscriptionProjectionState::default());
    assert!(apply_subscription_projection(
        &current,
        &state,
        json!({"active": true, "status": "active"}),
        SubscriptionSource::Socket(None),
        Some(2.0),
        None,
    ));
    assert!(!apply_subscription_projection(
        &current,
        &state,
        json!({"active": false, "status": "expired"}),
        SubscriptionSource::Rest(None),
        Some(1.0),
        Some(0),
    ));
    assert_eq!(
        current.subscription().expect("subscription")["status"],
        "active"
    );

    let writes = state.lock().socket_writes;
    assert!(apply_subscription_projection(
        &current,
        &state,
        json!({"active": true, "status": "grace_period"}),
        SubscriptionSource::Socket(None),
        None,
        None,
    ));
    assert!(!apply_subscription_projection(
        &current,
        &state,
        json!({"active": false, "status": "none"}),
        SubscriptionSource::Rest(None),
        None,
        Some(writes),
    ));
    assert_eq!(
        current.subscription().expect("subscription")["status"],
        "grace_period"
    );
}

#[test]
fn priority_capability_requires_a_current_socket_offer_and_paid_plan() {
    let current = CurrentAccount::default();
    let state = Mutex::new(SubscriptionProjectionState::default());
    let offer = json!({
        "active":true, "status":"active", "tier":"unlimited",
        "capabilities":{"priorityMode":true,"unlimited":true}
    });
    let current_offer = || {
        current
            .subscription()
            .unwrap()
            .pointer("/capabilities/priorityMode")
            .and_then(Value::as_bool)
    };
    let apply = |source, subscription, version| {
        apply_subscription_projection(&current, &state, subscription, source, Some(version), None)
    };
    assert!(apply(SubscriptionSource::Rest(Some(3)), offer.clone(), 2.0));
    assert_eq!(current_offer(), None);
    assert!(apply(
        SubscriptionSource::Socket(Some(3)),
        offer.clone(),
        3.0
    ));
    assert_eq!(current_offer(), Some(true));
    assert!(!apply(SubscriptionSource::Socket(Some(3)), json!({}), 2.0));
    let mut refresh = offer.clone();
    refresh["capabilities"] = json!({"other": true});
    assert!(apply(
        SubscriptionSource::Rest(Some(3)),
        refresh.clone(),
        3.0
    ));
    assert_eq!(current_offer(), Some(true));
    assert_eq!(
        current.subscription().unwrap()["capabilities"]["other"],
        true
    );
    refresh["status"] = json!("trialing");
    assert!(apply(SubscriptionSource::Rest(Some(3)), refresh, 3.0));
    assert_eq!(current_offer(), None);
    assert!(apply(SubscriptionSource::Rest(Some(4)), offer.clone(), 3.0));
    assert_eq!(current_offer(), None);
    assert!(apply(
        SubscriptionSource::Socket(Some(4)),
        offer.clone(),
        4.0
    ));
    super::projection::clear_socket_priority(&current, &state);
    assert_eq!(current_offer(), None);
    assert!(apply(SubscriptionSource::Rest(Some(4)), offer, 4.0));
    assert_eq!(current_offer(), None);
}

#[test]
fn socket_plan_updates_keep_only_an_active_positive_member_discount() {
    for (active, discount, expected) in [
        (true, json!(1000), Some(1000.0)),
        (true, json!(0), None),
        (false, json!(1000), None),
        (true, json!("invalid"), None),
    ] {
        let mapped = map_socket_subscription(&json!({
            "active": active,
            "subscription": {"status":"active","tier":"unlimited","vendorDiscountBps":discount}
        }))
        .unwrap();
        assert_eq!(
            mapped.get("frontierDiscountBps").and_then(Value::as_f64),
            expected
        );
    }
}
