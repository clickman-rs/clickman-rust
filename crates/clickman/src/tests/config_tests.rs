use std::path::PathBuf;
use std::time::Duration;

use crate::tests::support::StubHttp;
use crate::{ClickMan, Config};

#[test]
fn new_takes_the_protocol_defaults() {
    let config = Config::new(
        "https://ingest.example",
        "wk_ios",
        "/tmp/clickman/queue.sqlite",
    );
    let expected = Config {
        endpoint: "https://ingest.example".to_owned(),
        write_key: "wk_ios".to_owned(),
        path: PathBuf::from("/tmp/clickman/queue.sqlite"),
        flush_at: 20,
        flush_interval: Duration::from_secs(30),
        max_queue: 10_000,
        max_age: Duration::from_secs(30 * 24 * 60 * 60),
        max_batch_events: 100,
        max_batch_bytes: 900_000,
    };
    assert_eq!(config, expected);
}

#[test]
fn unusable_limits_are_rejected_before_creating_a_queue() {
    let directory = tempfile::tempdir().unwrap();
    let base = Config::new(
        "https://ingest.example",
        "wk_test",
        directory.path().join("queue.sqlite"),
    );
    for configure in [
        |config: &mut Config| config.flush_at = 0,
        |config: &mut Config| config.flush_interval = Duration::ZERO,
        |config: &mut Config| config.max_queue = 0,
        |config: &mut Config| config.max_age = Duration::ZERO,
        |config: &mut Config| config.max_batch_events = 0,
        |config: &mut Config| config.max_batch_events = 501,
        |config: &mut Config| config.max_batch_bytes = 0,
        |config: &mut Config| config.path = PathBuf::new(),
    ] {
        let mut config = base.clone();
        configure(&mut config);
        assert!(
            ClickMan::open(config.clone(), StubHttp::default()).is_err(),
            "accepted {config:?}"
        );
        assert!(!base.path.exists(), "invalid configuration created a queue");
    }
}

#[test]
fn ambiguous_or_credential_bearing_endpoints_and_invalid_keys_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    for endpoint in [
        "ingest.example",
        "file:///tmp/queue",
        "https://user:password@ingest.example",
        "https://ingest.example?other=1",
        "https://ingest.example#other",
    ] {
        let config = Config::new(endpoint, "wk_test", directory.path().join("queue.sqlite"));
        assert!(
            ClickMan::open(config, StubHttp::default()).is_err(),
            "accepted {endpoint}"
        );
    }
    for write_key in ["", " ", "wk_test\nother", "non-ascii-é"] {
        let config = Config::new(
            "https://ingest.example",
            write_key,
            directory.path().join("queue.sqlite"),
        );
        assert!(
            ClickMan::open(config, StubHttp::default()).is_err(),
            "accepted an invalid write key"
        );
    }
    assert!(!directory.path().join("queue.sqlite").exists());
}
