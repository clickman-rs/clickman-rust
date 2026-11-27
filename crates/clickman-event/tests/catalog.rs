//! The reference events of protocol/fixtures/catalog.json, declared in Rust.

use clickman_event::{Event, Values};
use serde_json::Value;

#[derive(Event)]
enum Studio {
    /// Export wrote every file
    /// and they are ready to use.
    ExportCompleted {
        /// File format of the export.
        format: Format,
        ///   Whether one mix or one file per track was written.
        mode: Mode,
        /// How many tracks the export contains.
        tracks: u32,
        /// Length of the exported audio in seconds.
        duration_seconds: Option<f64>,
        /// Whether loudness normalization was applied.
        normalized: Option<bool>,
        /// Title the user gave the export.
        #[clickman(text)]
        title: Option<String>,
    },
    /// A new empty project was created.
    ProjectCreated,
}

#[derive(Values)]
enum Format {
    Wav,
    Flac,
}

#[derive(Values)]
enum Mode {
    FullMix,
    PerTrack,
}

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol/fixtures/catalog.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_reference_events_export_the_fixture_catalog() {
    assert_eq!(Studio::catalog().to_json(), fixture()["catalog"]);
}

#[test]
fn every_sample_tracks_its_properties() {
    let samples = [
        Studio::ExportCompleted {
            format: Format::Flac,
            mode: Mode::PerTrack,
            tracks: 3,
            duration_seconds: Some(12.5),
            normalized: Some(true),
            title: Some("Демо".to_owned()),
        },
        Studio::ExportCompleted {
            format: Format::Wav,
            mode: Mode::FullMix,
            tracks: 1,
            duration_seconds: None,
            normalized: None,
            title: None,
        },
        Studio::ProjectCreated,
    ];
    let expected = fixture()["samples"].as_array().unwrap().clone();
    assert_eq!(expected.len(), samples.len());
    for (event, case) in samples.iter().zip(expected) {
        assert_eq!(event.name(), case["event"], "{}", case["name"]);
        assert_eq!(
            Value::Object(event.properties()),
            case["properties"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn an_explicit_name_overrides_the_declaration() {
    #[derive(Event)]
    enum Renamed {
        /// Opened.
        #[clickman(name = "app_launch")]
        Launched {
            /// Where it came from.
            #[clickman(name = "origin")]
            from: Origin,
        },
    }

    #[derive(Values)]
    enum Origin {
        #[clickman(name = "dock")]
        Shortcut,
    }

    let event = Renamed::Launched {
        from: Origin::Shortcut,
    };
    assert_eq!(event.name(), "app_launch");
    assert_eq!(event.properties()["origin"], "dock");
}
