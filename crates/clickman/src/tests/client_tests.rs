use serde_json::{Value, json};

use crate::tests::support::Harness;
use crate::{ClickMan, Error};

fn library() -> Value {
    json!({"name": "clickman-rust", "version": env!("CARGO_PKG_VERSION")})
}

#[test]
fn properties_must_be_an_object_and_null_is_none() {
    let harness = Harness::new();
    for properties in [json!([1]), json!("mp4"), json!(3)] {
        assert!(matches!(
            harness.clickman.track("event", properties),
            Err(Error::InvalidProperties)
        ));
    }
    harness.clickman.track("event", Value::Null).unwrap();
    assert_eq!(harness.clickman.pending().unwrap(), 1);
    assert_eq!(harness.stored()[0]["properties"], json!({}));
}

#[test]
fn traits_must_be_an_object_and_null_changes_none() {
    let harness = Harness::new();
    harness.clickman.set_traits(json!({"plan": "pro"})).unwrap();
    assert!(matches!(
        harness.clickman.set_traits(json!(["plan"])),
        Err(Error::InvalidTraits)
    ));
    harness.clickman.set_traits(Value::Null).unwrap();
    harness.clickman.track("event", json!({})).unwrap();
    assert_eq!(
        harness.stored()[0]["context"]["traits"],
        json!({"plan": "pro"})
    );
}

#[test]
fn a_new_context_replaces_the_last() {
    let harness = Harness::new();
    harness
        .clickman
        .set_context(json!({"os": {"name": "macOS"}}))
        .unwrap();
    harness
        .clickman
        .set_context(json!({"app": {"name": "Example"}}))
        .unwrap();
    assert!(matches!(
        harness.clickman.set_context(json!("macOS")),
        Err(Error::InvalidContext)
    ));
    harness.clickman.track("event", json!({})).unwrap();
    assert_eq!(
        harness.stored()[0]["context"],
        json!({"app": {"name": "Example"}, "library": library()})
    );
}

#[test]
fn the_context_lives_in_memory_only() {
    let harness = Harness::new();
    harness
        .clickman
        .set_context(json!({"os": {"name": "macOS"}}))
        .unwrap();
    let harness = harness.reopen([]);
    harness.clickman.track("event", json!({})).unwrap();
    assert_eq!(
        harness.stored()[0]["context"],
        json!({"library": library()})
    );
}

#[tokio::test]
async fn pending_counts_the_events_until_they_are_delivered() {
    let harness = Harness::new();
    harness.clickman.track("first", json!({})).unwrap();
    harness.clickman.track("second", json!({})).unwrap();
    assert_eq!(harness.clickman.pending().unwrap(), 2);
    harness.clickman.flush().await.unwrap();
    assert_eq!(harness.clickman.pending().unwrap(), 0);
}

#[test]
fn a_client_and_its_sends_cross_threads() {
    fn shared<T: Send + Sync>() {}
    fn sent<T: Send>(_: &T) {}
    shared::<ClickMan>();
    let harness = Harness::new();
    sent(&harness.clickman.flush());
    sent(&harness.clickman.send_due());
}

#[test]
fn actor_names_whom_later_events_are_about() {
    let harness = Harness::new();
    assert_eq!(harness.clickman.actor().unwrap(), "*");
    harness.clickman.identify_installation().unwrap();
    assert!(harness.clickman.actor().unwrap().starts_with("anon:"));
    harness.clickman.identify("user_42").unwrap();
    assert_eq!(harness.clickman.actor().unwrap(), "user_42");
}

#[test]
fn clear_deletes_what_waits_and_later_events_queue() {
    let harness = Harness::new();
    harness.clickman.identify_installation().unwrap();
    harness.clickman.track("before", json!({})).unwrap();
    harness.clickman.identify("user_42").unwrap();
    harness.clickman.clear().unwrap();
    assert_eq!(harness.clickman.pending().unwrap(), 0);
    harness.clickman.track("after", json!({})).unwrap();
    assert_eq!(harness.stored_names(), ["after"]);
}
