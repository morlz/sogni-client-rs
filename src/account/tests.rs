use parking_lot::Mutex;
use serde_json::json;

use super::projection::{
    SubscriptionSource, apply_subscription_projection, map_socket_subscription,
};
#[cfg(feature = "wallet")]
use super::wallet_crypto::{amount_to_wei, wallet};
use super::{CurrentAccount, SubscriptionProjectionState};

mod auth_state;
#[cfg(feature = "wallet")]
mod cookie_rollback;

#[cfg(feature = "wallet")]
#[test]
fn wallet_derivation_matches_the_typescript_client() {
    use ethers_signers::Signer as _;

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
        SubscriptionSource::Socket,
        Some(2.0),
        None,
    ));
    assert!(!apply_subscription_projection(
        &current,
        &state,
        json!({"active": false, "status": "expired"}),
        SubscriptionSource::Rest,
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
        SubscriptionSource::Socket,
        None,
        None,
    ));
    assert!(!apply_subscription_projection(
        &current,
        &state,
        json!({"active": false, "status": "none"}),
        SubscriptionSource::Rest,
        None,
        Some(writes),
    ));
    assert_eq!(
        current.subscription().expect("subscription")["status"],
        "grace_period"
    );
}
