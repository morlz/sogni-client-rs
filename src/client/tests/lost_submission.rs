use super::{
    SogniClient,
    project_wire_server::{Fixture, KEY},
};
use crate::{EventReceiver, ProjectRequest, ProjectResolution, ResolveMissingOptions};
use serde_json::{Value, json};
use std::time::Duration;

const ID: &str = "AAAA0000-0000-4000-8000-000000000001";

async fn submit(fixture: &Fixture) -> (SogniClient, EventReceiver) {
    let client = SogniClient::builder()
        .app_id("lost-submission-fixture")
        .api_key(KEY)
        .rest_endpoint(format!("http://{}/", fixture.address).parse().unwrap())
        .socket_endpoint(format!("ws://{}/", fixture.address).parse().unwrap())
        .defer_socket_start(true)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .await
        .unwrap();
    let events = client.subscribe();
    client
        .projects
        .create_with_id(
            ID,
            ProjectRequest::image("z_image_turbo_bf16", "fixture")
                .steps(8)
                .guidance(1.0)
                .dimensions(1024, 1024),
        )
        .await
        .unwrap();
    (client, events)
}

async fn connections(events: &mut EventReceiver, count: usize) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut received = 0;
        while received < count {
            if events.recv().await.unwrap().name == "authenticated" {
                received += 1;
            }
        }
    })
    .await
    .expect("reconnected fixture");
}

async fn resolve(client: &SogniClient) -> ProjectResolution {
    client
        .projects
        .resolve_missing(
            &[ID],
            Some(ResolveMissingOptions {
                attempts: 1,
                retry_delay: Duration::ZERO,
            }),
        )
        .await
        .remove(ID)
        .unwrap()
}

async fn next_request(fixture: &mut Fixture) -> Value {
    tokio::time::timeout(Duration::from_secs(10), fixture.wire.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn confirmed_absent_request_is_resent_once_after_a_dropped_connection() {
    let mut fixture = Fixture::with_disconnects(2, false).await;
    let (client, mut events) = submit(&fixture).await;
    let original = next_request(&mut fixture).await;
    connections(&mut events, 2).await;
    // Concurrent recovery calls must claim the retry before either awaits a write.
    let (first, second) = tokio::join!(resolve(&client), resolve(&client));
    assert_eq!(first, ProjectResolution::Active);
    assert_eq!(second, ProjectResolution::Active);
    assert_eq!(next_request(&mut fixture).await, original);
    connections(&mut events, 1).await;
    assert_eq!(resolve(&client).await, ProjectResolution::Active);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), fixture.wire.recv())
            .await
            .is_err(),
        "a second disconnect cannot cause another resend"
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn live_connections_and_acknowledged_requests_are_never_resent() {
    for (disconnects, acknowledged) in [(0, false), (1, true)] {
        let mut fixture = Fixture::with_disconnects(disconnects, acknowledged).await;
        let (client, mut events) = submit(&fixture).await;
        next_request(&mut fixture).await;
        connections(&mut events, if disconnects == 0 { 1 } else { 2 }).await;
        assert_eq!(resolve(&client).await, ProjectResolution::Lost);
        assert!(fixture.wire.try_recv().is_err());
        client.close().await.unwrap();
    }
}

#[tokio::test]
async fn unknown_registry_does_not_resend_and_positive_lookups_permanently_retire_eligibility() {
    for lookup in ["unknown", "active", "owner-status"] {
        let mut fixture = Fixture::with_disconnects(1, false).await;
        let (client, mut events) = submit(&fixture).await;
        next_request(&mut fixture).await;
        connections(&mut events, 2).await;
        match lookup {
            "unknown" => *fixture.active.lock() = json!({"projects":"malformed"}),
            "active" => *fixture.active.lock() = json!({"projects":[{"id":ID}]}),
            _ => {
                *fixture.status.lock() =
                    Some(json!({"id":ID,"status":"processing","finished":false}))
            }
        }
        if lookup == "unknown" {
            assert!(matches!(
                resolve(&client).await,
                ProjectResolution::Unknown { .. }
            ));
        } else {
            assert_eq!(resolve(&client).await, ProjectResolution::Active);
            *fixture.active.lock() = json!({"projects":[]});
            *fixture.status.lock() = None;
            assert_eq!(resolve(&client).await, ProjectResolution::Lost);
        }
        assert!(fixture.wire.try_recv().is_err());
        client.close().await.unwrap();
    }
}
