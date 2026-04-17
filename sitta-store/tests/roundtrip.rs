use sitta_store::db::Database;
use sitta_store::models::{
    NewAudioSource, NewDetection, NewLabel, NewModel, NewPrediction, NewStation,
};
use uuid::Uuid;

/// Helper: open a temporary database with migrations applied.
async fn open_temp_db() -> Database {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    // Leak the tempdir so the file persists for the test duration.
    std::mem::forget(dir);
    Database::open(&path).await.unwrap()
}

#[tokio::test]
async fn seed_and_insert_roundtrip() {
    let db = open_temp_db().await;

    // Seed a station.
    let station_id = Uuid::now_v7();
    db.upsert_station(&NewStation {
        id: &station_id,
        name: "Test Station",
        latitude: Some(44.5),
        longitude: Some(-76.0),
    })
    .await
    .unwrap();

    // Seed an audio source.
    let source_id = Uuid::now_v7();
    db.upsert_audio_source(&NewAudioSource {
        id: &source_id,
        station_id: &station_id,
        name: "test_mic",
        source_type: "local",
        uri: Some("/dev/snd"),
        sample_rate: 48000,
        channels: 1,
    })
    .await
    .unwrap();

    // Seed a model.
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
    assert!(model_id > 0);

    // Seed labels.
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
        NewLabel {
            model_id,
            label_index: 2,
            scientific_name: None,
            common_name: "Background noise",
            label_type: "environment",
            taxon_code: None,
        },
    ];
    db.seed_labels(&labels).await.unwrap();

    // Load label cache and verify.
    let cache = db.load_label_id_cache().await.unwrap();
    assert_eq!(cache.len(), 3);
    let barn_owl_label_id = cache[&(model_id, 0)];
    let tawny_owl_label_id = cache[&(model_id, 1)];
    assert_ne!(barn_owl_label_id, tawny_owl_label_id);

    // Insert a detection.
    let detection_id = Uuid::now_v7();
    db.insert_detection(&NewDetection {
        id: &detection_id,
        station_id: &station_id,
        source_id: Some(&source_id),
        model_id,
        label_id: barn_owl_label_id,
        detected_at: 1713168600000, // 2024-04-15T06:30:00Z
        confidence: 0.92,
        snippet_path: None,
        snippet_duration_ms: None,
        snippet_sample_rate: None,
        metadata: None,
    })
    .await
    .unwrap();

    // Insert secondary predictions.
    let predictions = vec![
        NewPrediction {
            rank: 1,
            label_id: tawny_owl_label_id,
            confidence: 0.07,
        },
    ];
    db.insert_predictions(&detection_id, &predictions)
        .await
        .unwrap();

    // Verify detection count via raw query.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM detections")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);

    let pred_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM detection_predictions")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(pred_count, 1);
}

#[tokio::test]
async fn embedding_roundtrip() {
    let db = open_temp_db().await;

    let station_id = Uuid::now_v7();
    db.upsert_station(&NewStation {
        id: &station_id,
        name: "Embed Station",
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
            embedding_dim: Some(4),
        })
        .await
        .unwrap();

    let labels = vec![NewLabel {
        model_id,
        label_index: 0,
        scientific_name: Some("Tyto alba"),
        common_name: "Barn Owl",
        label_type: "species",
        taxon_code: None,
    }];
    db.seed_labels(&labels).await.unwrap();
    let cache = db.load_label_id_cache().await.unwrap();

    let detection_id = Uuid::now_v7();
    db.insert_detection(&NewDetection {
        id: &detection_id,
        station_id: &station_id,
        source_id: None,
        model_id,
        label_id: cache[&(model_id, 0)],
        detected_at: 1713168600000,
        confidence: 0.85,
        snippet_path: None,
        snippet_duration_ms: None,
        snippet_sample_rate: None,
        metadata: None,
    })
    .await
    .unwrap();

    // Insert embedding (4-dim for test brevity).
    let embedding = vec![0.1f32, 0.2, 0.3, 0.4];
    db.insert_embedding(&detection_id, &embedding)
        .await
        .unwrap();

    // Read it back and verify dimensions.
    let det_id = detection_id.as_bytes().as_slice();
    let row = sqlx::query!("SELECT embedding_dim FROM embeddings WHERE detection_id = $1", det_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(row.embedding_dim, 4);
}

#[tokio::test]
async fn foreign_key_enforcement() {
    let db = open_temp_db().await;

    // Inserting a detection with a nonexistent station_id should fail.
    let fake_station = Uuid::now_v7();
    let fake_model_id = 999;
    let fake_label_id = 999;
    let detection_id = Uuid::now_v7();

    let result = db
        .insert_detection(&NewDetection {
            id: &detection_id,
            station_id: &fake_station,
            source_id: None,
            model_id: fake_model_id,
            label_id: fake_label_id,
            detected_at: 0,
            confidence: 0.5,
            snippet_path: None,
            snippet_duration_ms: None,
            snippet_sample_rate: None,
            metadata: None,
        })
        .await;

    assert!(result.is_err(), "FK constraint should reject bad station_id");
}

#[tokio::test]
async fn upsert_model_is_idempotent() {
    let db = open_temp_db().await;

    let id1 = db
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

    let id2 = db
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

    assert_eq!(id1, id2, "Repeated upsert should return the same ID");
}

#[tokio::test]
async fn seed_labels_is_idempotent() {
    let db = open_temp_db().await;

    let model_id = db
        .upsert_model(&NewModel {
            name: "test",
            version: "1",
            sample_rate: 16000,
            window_samples: 48000,
            has_embeddings: false,
            embedding_dim: None,
        })
        .await
        .unwrap();

    let labels = vec![NewLabel {
        model_id,
        label_index: 0,
        scientific_name: Some("Foo bar"),
        common_name: "Test Bird",
        label_type: "species",
        taxon_code: None,
    }];

    db.seed_labels(&labels).await.unwrap();
    db.seed_labels(&labels).await.unwrap(); // second call should be no-op

    let cache = db.load_label_id_cache().await.unwrap();
    assert_eq!(cache.len(), 1);
}
