//! Every case of protocol/fixtures/identity.json: visitors, their links and how
//! links leave the queue.

use rusqlite::params;
use serde_json::{Value, json};

use crate::batch::{self, Kind, Urgency};
use crate::clock::unix_millis;
use crate::outcome::Outcome;
use crate::store::Store;
use crate::tests::support::{Harness, START, fixture, scratch_config, status};

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

/// The fixture's names for actors: `visitor N` for the Nth visitor a case started.
#[derive(Default)]
struct Visitors(Vec<String>);

impl Visitors {
    fn label(&mut self, actor: &str) -> String {
        if !actor.starts_with("anon:") {
            return actor.to_owned();
        }
        let index = match self.0.iter().position(|seen| seen == actor) {
            Some(index) => index,
            None => {
                self.0.push(actor.to_owned());
                self.0.len() - 1
            }
        };
        format!("visitor {}", index + 1)
    }
}

fn actor(harness: &Harness) -> String {
    harness
        .raw()
        .query_row("SELECT external_id FROM identity", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn identity_json_cases_track_and_link_as_listed() {
    for case in fixture("identity.json")["cases"].as_array().unwrap() {
        let name = text(&case["name"]);
        let harness = Harness::new();
        let mut visitors = Visitors::default();
        for step in case["steps"].as_array().unwrap() {
            let result = if step.get("identifyInstallation").is_some() {
                harness.clickman.identify_installation()
            } else if let Some(event) = step.get("track") {
                harness.clickman.track(text(event), json!({})).map(drop)
            } else if let Some(external_id) = step.get("identify") {
                harness.clickman.identify(text(external_id))
            } else if step.get("reset").is_some() {
                harness.clickman.reset()
            } else {
                panic!("{name}: unknown step {step}");
            };
            let refused = step.get("error").is_some_and(|error| error == true);
            assert_eq!(result.is_err(), refused, "{name}: {step}");
            visitors.label(&actor(&harness));
        }
        let queue: Vec<Value> = harness
            .stored()
            .iter()
            .map(|row| match text(&row["type"]) {
                "track" => json!({
                    "track": row["event"],
                    "externalId": visitors.label(text(&row["externalId"])),
                }),
                "identify" => json!({"link": {
                    "anonymousId": visitors.label(text(&row["anonymousId"])),
                    "externalId": row["externalId"],
                }}),
                other => panic!("{name}: a row of type {other}"),
            })
            .collect();
        assert_eq!(
            visitors.label(&actor(&harness)),
            text(&case["actor"]),
            "{name}"
        );
        assert_eq!(Value::Array(queue), case["queue"], "{name}");
    }
}

#[test]
fn identity_json_takes_stop_before_a_link() {
    for case in fixture("identity.json")["takes"]["cases"]
        .as_array()
        .unwrap()
    {
        let name = text(&case["name"]);
        let dir = tempfile::tempdir().unwrap();
        let config = scratch_config(&dir);
        let now = unix_millis(START);
        let store = Store::open(&config.path).unwrap();
        store
            .write(|transaction| {
                for row in case["rows"].as_array().unwrap() {
                    let body = match text(row) {
                        "track" => json!({"type": "track", "event": "a"}),
                        _ => {
                            json!({"type": "identify", "anonymousId": "anon:x", "externalId": "u"})
                        }
                    };
                    transaction.execute(
                        "INSERT INTO events (created_at, body) VALUES (?1, ?2)",
                        params![now, body.to_string()],
                    )?;
                }
                Ok(())
            })
            .unwrap();
        let mut taken = Vec::new();
        while let Some(take) = store
            .write(|transaction| batch::take(transaction, &config, now, Urgency::Forced))
            .unwrap()
        {
            let kind = match take.kind {
                Kind::Events => "track",
                Kind::Link => "link",
            };
            taken.push(json!(vec![kind; take.bodies.len()]));
            store
                .write(|transaction| batch::remove(transaction, &take))
                .unwrap();
        }
        assert_eq!(Value::Array(taken), case["taken"], "{name}");
    }
}

#[tokio::test]
async fn a_link_goes_alone_and_uncompressed_to_identify() {
    let harness = Harness::new();
    harness.clickman.identify_installation().unwrap();
    let visitor = actor(&harness);
    harness.clickman.track("before", json!({})).unwrap();
    harness.clickman.identify("user_42").unwrap();
    harness.clickman.track("after", json!({})).unwrap();
    harness.clickman.flush().await.unwrap();

    let posted = harness.http.posted();
    assert_eq!(posted.len(), 3);
    assert!(posted[0].url.ends_with("/v1/batch"));
    assert_eq!(posted[0].event_names(), ["before"]);
    let link = &posted[1];
    assert!(link.url.ends_with("/v1/identify"), "{}", link.url);
    assert!(
        !link
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
    );
    let body: Value = serde_json::from_slice(&link.body).unwrap();
    let shape = &fixture("identity.json")["request"]["body"];
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        shape.as_object().unwrap().keys().collect::<Vec<_>>()
    );
    assert_eq!(
        body,
        json!({"anonymousId": visitor, "externalId": "user_42"})
    );
    assert_eq!(posted[2].event_names(), ["after"]);
    assert_eq!(harness.clickman.pending().unwrap(), 0);
}

#[tokio::test]
async fn identity_json_outcomes_decide_what_stays_queued() {
    for case in fixture("identity.json")["outcomes"]["cases"]
        .as_array()
        .unwrap()
    {
        let name = text(&case["name"]);
        let code = u16::try_from(case["status"].as_u64().unwrap()).unwrap();
        let expected = match text(&case["outcome"]) {
            "delivered" => Outcome::Delivered,
            "refused" => Outcome::Refused,
            _ => Outcome::Retry,
        };
        assert_eq!(Outcome::of_link(code), expected, "{name}");

        let answer = if code == 0 {
            Err("no response".to_owned())
        } else {
            Ok(status(code))
        };
        let harness = Harness::with(|_| {}, [answer]);
        harness.clickman.identify_installation().unwrap();
        harness.clickman.identify("user_42").unwrap();
        harness.clickman.flush().await.unwrap();
        let left = usize::from(expected == Outcome::Retry);
        assert_eq!(harness.clickman.pending().unwrap(), left, "{name}");
    }
}
