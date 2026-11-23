use mio_gateway::{decode, parse_teleop_frame, queues_in_outbox};
use serde_json::Value;
use std::{fs, path::PathBuf};

fn vectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../miobots-protocol/conformance")
}

#[test]
fn envelope_vectors_match_typescript() {
    for (folder, expected) in [("valid", true), ("invalid", false)] {
        for file in fs::read_dir(vectors().join(folder)).unwrap() {
            let path = file.unwrap().path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let vector: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            let raw = vector
                .get("raw")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| vector["envelope"].to_string());
            let actual = decode(raw.as_bytes());
            assert_eq!(actual.is_ok(), expected, "{}: {actual:?}", path.display());
            if let Err(err) = actual {
                if let Some(reason) = vector["expect"]["reason"].as_str() {
                    assert_eq!(err.reason, reason, "{}", path.display());
                }
                if let Some(field) = vector["expect"]["field"].as_str() {
                    assert_eq!(err.field, Some(field), "{}", path.display());
                }
            }
        }
    }
}

#[test]
fn teleop_vectors_match_typescript() {
    let vectors: Value =
        serde_json::from_slice(&fs::read(vectors().join("teleop.json")).unwrap()).unwrap();
    for vector in vectors.as_array().unwrap() {
        let parsed = parse_teleop_frame(vector["raw"].as_str().unwrap());
        assert_eq!(
            parsed.is_some(),
            vector["expect"]["parses"].as_bool().unwrap(),
            "{}",
            vector["description"]
        );
        if let Some(frame) = parsed {
            assert_eq!(
                frame, vector["expect"]["frame"],
                "{}",
                vector["description"]
            );
        }
    }
}

/// Which messages wait in the outbox. The Brain confirms by the TypeScript list; an event stored
/// here that the Brain never confirms would be re-sent after every reconnect, for ever.
#[test]
fn outbox_vectors_match_typescript() {
    let vectors: Value =
        serde_json::from_slice(&fs::read(vectors().join("outbox.json")).unwrap()).unwrap();
    for vector in vectors.as_array().unwrap() {
        let (kind, topic) = (
            vector["kind"].as_str().unwrap(),
            vector["topic"].as_str().unwrap(),
        );
        assert_eq!(
            queues_in_outbox(kind, topic),
            vector["queues"].as_bool().unwrap(),
            "{kind} {topic}: {}",
            vector["description"]
        );
    }
}
