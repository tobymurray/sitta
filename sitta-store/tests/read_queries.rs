use sitta_store::db::Database;
use sitta_store::models::{NewDetection, NewLabel, NewModel, NewPrediction, NewStation};
use uuid::Uuid;

async fn open_temp_db() -> Database {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    std::mem::forget(dir);
    Database::open(&path).await.unwrap()
}

/// Seed a station + model + labels and insert several detections.
/// Returns (db, station_id, model_id, label_ids, detection_ids).
async fn seed_with_detections() -> (Database, Uuid, i64, Vec<i64>, Vec<Uuid>) {
    let db = open_temp_db().await;

    let station_id = Uuid::now_v7();
    db.upsert_station(&NewStation {
        id: &station_id,
        name: "Test",
        latitude: Some(44.5),
        longitude: Some(-76.0),
    })
    .await
    .unwrap();

    let model_id = db
        .upsert_model(&NewModel {
            name: "birdnet",
            version: "2.4",
            sample_rate: 48000,
            window_samples: 144000,
            has_embeddings: false,
            embedding_dim: None,
        })
        .await
        .unwrap();

    let labels = vec![
        NewLabel {
            model_id,
            label_index: 0,
            scientific_name: Some("Tyto alba"),
            common_name: "Barn Owl",
            label_type: "species",
            taxon_code: Some("barowl1"),
        },
        NewLabel {
            model_id,
            label_index: 1,
            scientific_name: Some("Strix aluco"),
            common_name: "Tawny Owl",
            label_type: "species",
            taxon_code: Some("tawowl1"),
        },
    ];
    db.seed_labels(&labels).await.unwrap();
    let cache = db.load_label_id_cache().await.unwrap();
    let label_ids = vec![cache[&(model_id, 0)], cache[&(model_id, 1)]];

    let mut detection_ids = Vec::new();
    let base_time = 1713168600000_i64; // some reference time

    // Detection 1: Barn Owl at t+0
    let d1 = Uuid::now_v7();
    db.insert_detection(&NewDetection {
        id: &d1,
        station_id: &station_id,
        source_id: None,
        model_id,
        label_id: label_ids[0],
        detected_at: base_time,
        confidence: 0.92,
        snippet_path: None,
        snippet_duration_ms: None,
        snippet_sample_rate: None,
        metadata: None,
        range_status: None,
    })
    .await
    .unwrap();
    detection_ids.push(d1);

    // Detection 2: Barn Owl at t+10000 (>5s dedup window)
    let d2 = Uuid::now_v7();
    db.insert_detection(&NewDetection {
        id: &d2,
        station_id: &station_id,
        source_id: None,
        model_id,
        label_id: label_ids[0],
        detected_at: base_time + 10_000,
        confidence: 0.85,
        snippet_path: None,
        snippet_duration_ms: None,
        snippet_sample_rate: None,
        metadata: None,
        range_status: None,
    })
    .await
    .unwrap();
    detection_ids.push(d2);

    // Detection 3: Tawny Owl at t+2000
    let d3 = Uuid::now_v7();
    db.insert_detection(&NewDetection {
        id: &d3,
        station_id: &station_id,
        source_id: None,
        model_id,
        label_id: label_ids[1],
        detected_at: base_time + 20_000,
        confidence: 0.78,
        snippet_path: None,
        snippet_duration_ms: None,
        snippet_sample_rate: None,
        metadata: None,
        range_status: None,
    })
    .await
    .unwrap();
    detection_ids.push(d3);

    // Add predictions for d1.
    db.insert_predictions(
        &d1,
        &[NewPrediction {
            rank: 1,
            label_id: label_ids[1],
            confidence: 0.07,
        }],
    )
    .await
    .unwrap();

    (db, station_id, model_id, label_ids, detection_ids)
}

#[tokio::test]
async fn recent_detections_returns_all_in_range() {
    let (db, _, _, _, _) = seed_with_detections().await;
    let rows = db
        .recent_detections(0, i64::MAX, 50, 0, None, None, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    // Ordered by detected_at DESC.
    assert!(rows[0].detected_at > rows[1].detected_at);
}

#[tokio::test]
async fn recent_detections_respects_limit() {
    let (db, _, _, _, _) = seed_with_detections().await;
    let rows = db
        .recent_detections(0, i64::MAX, 2, 0, None, None, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn recent_detections_species_filter() {
    let (db, _, _, _, _) = seed_with_detections().await;
    let rows = db
        .recent_detections(0, i64::MAX, 50, 0, Some("Strix aluco"), None, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].common_name, "Tawny Owl");
}

#[tokio::test]
async fn recent_detections_joined_fields() {
    let (db, _, _, _, _) = seed_with_detections().await;
    let rows = db
        .recent_detections(0, i64::MAX, 1, 0, None, None, false)
        .await
        .unwrap();
    let row = &rows[0];
    assert_eq!(row.model_name, "birdnet");
    assert_eq!(row.model_version, "2.4");
    assert!(row.scientific_name.is_some());
}

#[tokio::test]
async fn get_detection_found_and_not_found() {
    let (db, _, _, _, detection_ids) = seed_with_detections().await;

    let found = db
        .get_detection(detection_ids[0].as_bytes().as_slice())
        .await
        .unwrap();
    assert!(found.is_some());
    let row = found.unwrap();
    assert_eq!(row.common_name, "Barn Owl");

    let missing = db
        .get_detection(Uuid::nil().as_bytes().as_slice())
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn get_predictions_returns_ranked() {
    let (db, _, _, _, detection_ids) = seed_with_detections().await;
    let preds = db
        .get_predictions(detection_ids[0].as_bytes().as_slice())
        .await
        .unwrap();
    assert_eq!(preds.len(), 1);
    assert_eq!(preds[0].rank, 1);
    assert_eq!(preds[0].common_name, "Tawny Owl");
}

#[tokio::test]
async fn species_summary_aggregates() {
    let (db, _, _, _, _) = seed_with_detections().await;
    let summary = db.species_summary(0, i64::MAX, None).await.unwrap();
    assert_eq!(summary.len(), 2);
    // Barn Owl has 2 detections, should be first (ORDER BY COUNT(*) DESC).
    assert_eq!(summary[0].common_name, "Barn Owl");
    assert_eq!(summary[0].detection_count, 2);
    assert_eq!(summary[1].common_name, "Tawny Owl");
    assert_eq!(summary[1].detection_count, 1);
}

#[tokio::test]
async fn detection_count_matches() {
    let (db, _, _, _, _) = seed_with_detections().await;
    let count = db.detection_count().await.unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn embedding_bytemuck_roundtrip() {
    let db = open_temp_db().await;

    let station_id = Uuid::now_v7();
    db.upsert_station(&NewStation {
        id: &station_id,
        name: "E",
        latitude: None,
        longitude: None,
    })
    .await
    .unwrap();

    let model_id = db
        .upsert_model(&NewModel {
            name: "perch",
            version: "2",
            sample_rate: 32000,
            window_samples: 160000,
            has_embeddings: true,
            embedding_dim: Some(8),
        })
        .await
        .unwrap();

    db.seed_labels(&[NewLabel {
        model_id,
        label_index: 0,
        scientific_name: Some("Test sp"),
        common_name: "Test",
        label_type: "species",
        taxon_code: None,
    }])
    .await
    .unwrap();
    let cache = db.load_label_id_cache().await.unwrap();

    let det_id = Uuid::now_v7();
    db.insert_detection(&NewDetection {
        id: &det_id,
        station_id: &station_id,
        source_id: None,
        model_id,
        label_id: cache[&(model_id, 0)],
        detected_at: 0,
        confidence: 0.9,
        snippet_path: None,
        snippet_duration_ms: None,
        snippet_sample_rate: None,
        metadata: None,
        range_status: None,
    })
    .await
    .unwrap();

    // Insert embedding as f32 slice — stored as LE bytes via bytemuck.
    let original: Vec<f32> = vec![0.1, -0.2, 0.3, -0.4, 0.5, -0.6, 0.7, -0.8];
    db.insert_embedding(&det_id, &original).await.unwrap();

    // Read back the raw BLOB and verify byte-level round-trip.
    let det_bytes = det_id.as_bytes().as_slice();
    let row = sqlx::query!("SELECT embedding, embedding_dim FROM embeddings WHERE detection_id = $1", det_bytes)
        .fetch_one(db.pool())
        .await
        .unwrap();

    assert_eq!(row.embedding_dim, 8);
    let blob = row.embedding;
    assert_eq!(blob.len(), 8 * 4); // 8 floats * 4 bytes each

    // Convert bytes back to f32 via bytemuck.
    let recovered: &[f32] = bytemuck::cast_slice(&blob);
    assert_eq!(recovered.len(), original.len());
    for (a, b) in original.iter().zip(recovered.iter()) {
        assert_eq!(a.to_bits(), b.to_bits(), "f32 bit-exact round-trip failed");
    }
}
