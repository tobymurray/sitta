use sitta_api::settings::{apply_update, persist_to_toml, RuntimeSettings, SettingsUpdate};
use std::io::Write;

fn base_settings() -> RuntimeSettings {
    RuntimeSettings {
        station_name: "Test Station".into(),
        station_latitude: Some(44.5),
        station_longitude: Some(-76.0),
        birdnet_min_confidence: Some(0.25),
        birdnet_top_k: Some(10),
        birdnet_meta_threshold: Some(0.01),
        birdnet_force_allow: Some(vec!["helgui1".into()]),
        perch_min_confidence: Some(0.25),
        perch_top_k: Some(10),
    }
}

fn empty_update() -> SettingsUpdate {
    SettingsUpdate {
        station_name: None,
        station_latitude: None,
        station_longitude: None,
        birdnet_min_confidence: None,
        birdnet_top_k: None,
        birdnet_meta_threshold: None,
        birdnet_force_allow: None,
        perch_min_confidence: None,
        perch_top_k: None,
    }
}

#[test]
fn apply_no_changes() {
    let current = base_settings();
    let update = empty_update();
    let (merged, changed) = apply_update(&current, &update);
    assert!(changed.is_empty());
    assert_eq!(merged.station_name, "Test Station");
}

#[test]
fn apply_single_field() {
    let current = base_settings();
    let mut update = empty_update();
    update.station_name = Some("New Name".into());
    let (merged, changed) = apply_update(&current, &update);
    assert_eq!(changed, vec!["station_name"]);
    assert_eq!(merged.station_name, "New Name");
    // Other fields unchanged.
    assert_eq!(merged.birdnet_min_confidence, Some(0.25));
}

#[test]
fn apply_same_value_is_not_a_change() {
    let current = base_settings();
    let mut update = empty_update();
    update.birdnet_min_confidence = Some(0.25); // same as current
    let (_, changed) = apply_update(&current, &update);
    assert!(changed.is_empty());
}

#[test]
fn apply_multiple_fields() {
    let current = base_settings();
    let mut update = empty_update();
    update.birdnet_min_confidence = Some(0.5);
    update.perch_top_k = Some(5);
    update.station_latitude = Some(45.0);
    let (merged, changed) = apply_update(&current, &update);
    assert_eq!(changed.len(), 3);
    assert_eq!(merged.birdnet_min_confidence, Some(0.5));
    assert_eq!(merged.perch_top_k, Some(5));
    assert_eq!(merged.station_latitude, Some(45.0));
}

#[test]
fn apply_force_allow_change() {
    let current = base_settings();
    let mut update = empty_update();
    update.birdnet_force_allow = Some(vec!["helgui1".into(), "redjun1".into()]);
    let (merged, changed) = apply_update(&current, &update);
    assert_eq!(changed, vec!["birdnet_force_allow"]);
    assert_eq!(
        merged.birdnet_force_allow,
        Some(vec!["helgui1".into(), "redjun1".into()])
    );
}

#[test]
fn persist_to_toml_roundtrip() {
    let toml_content = r#"# Station configuration
[station]
id = "station_01"
name = "Original Name"
latitude = 44.5

[inference.birdnet]
model_path = "/models/birdnet.onnx"
labels_path = "/models/labels.txt"
min_confidence = 0.25
top_k = 10
meta_threshold = 0.01
force_allow = ["helgui1"]

[inference.perch]
model_path = "/models/perch.onnx"
labels_path = "/models/perch.csv"
min_confidence = 0.25
top_k = 10
"#;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(toml_content.as_bytes()).unwrap();
    }

    let settings = RuntimeSettings {
        station_name: "Updated Name".into(),
        station_latitude: Some(45.0),
        station_longitude: None,
        birdnet_min_confidence: Some(0.5),
        birdnet_top_k: Some(5),
        birdnet_meta_threshold: Some(0.05),
        birdnet_force_allow: Some(vec!["helgui1".into(), "redjun1".into()]),
        perch_min_confidence: Some(0.3),
        perch_top_k: Some(8),
    };

    persist_to_toml(&path, &settings).unwrap();

    let result = std::fs::read_to_string(&path).unwrap();

    // Verify values were updated.
    assert!(result.contains(r#"name = "Updated Name""#));
    assert!(result.contains("latitude = 45.0"));
    assert!(result.contains("min_confidence = 0.5")); // birdnet
    assert!(result.contains("top_k = 5")); // birdnet
    assert!(result.contains("meta_threshold = 0.05"));
    assert!(result.contains(r#""redjun1""#));

    // Verify comment was preserved.
    assert!(result.contains("# Station configuration"));

    // Verify read-only fields were not removed.
    assert!(result.contains(r#"id = "station_01""#));
    assert!(result.contains(r#"model_path = "/models/birdnet.onnx""#));
}

#[test]
fn persist_to_toml_missing_section_is_noop() {
    // Config with no [inference.birdnet] section — persist should not crash.
    let toml_content = r#"[station]
id = "s1"
name = "Test"
"#;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml_content).unwrap();

    let settings = RuntimeSettings {
        station_name: "Updated".into(),
        station_latitude: None,
        station_longitude: None,
        birdnet_min_confidence: Some(0.5),
        birdnet_top_k: None,
        birdnet_meta_threshold: None,
        birdnet_force_allow: None,
        perch_min_confidence: None,
        perch_top_k: None,
    };

    // Should succeed — birdnet section missing is silently skipped.
    persist_to_toml(&path, &settings).unwrap();

    let result = std::fs::read_to_string(&path).unwrap();
    assert!(result.contains(r#"name = "Updated""#));
}
