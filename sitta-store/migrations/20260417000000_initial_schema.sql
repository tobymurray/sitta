-- Initial Sitta schema: 10 tables for detection persistence,
-- embedding storage, individual recognition, and review.

-- ============================================================
-- Reference / Dimension Tables
-- ============================================================

CREATE TABLE stations (
    id          BLOB(16)    NOT NULL PRIMARY KEY,
    name        TEXT        NOT NULL,
    latitude    REAL,
    longitude   REAL
);

CREATE TABLE models (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,
    version         TEXT    NOT NULL,
    sample_rate     INTEGER NOT NULL,
    window_samples  INTEGER NOT NULL,
    has_embeddings  INTEGER NOT NULL DEFAULT 0,
    embedding_dim   INTEGER,
    UNIQUE(name, version)
);

CREATE TABLE labels (
    id              INTEGER PRIMARY KEY,
    model_id        INTEGER NOT NULL REFERENCES models(id),
    label_index     INTEGER NOT NULL,
    scientific_name TEXT,
    common_name     TEXT    NOT NULL,
    label_type      TEXT    NOT NULL DEFAULT 'species',
    taxon_code      TEXT,
    UNIQUE(model_id, label_index)
);

CREATE INDEX idx_labels_scientific ON labels(scientific_name) WHERE scientific_name IS NOT NULL;
CREATE INDEX idx_labels_model_scientific ON labels(model_id, scientific_name);

CREATE TABLE audio_sources (
    id          BLOB(16)    NOT NULL PRIMARY KEY,
    station_id  BLOB(16)    NOT NULL REFERENCES stations(id),
    name        TEXT        NOT NULL,
    source_type TEXT        NOT NULL,
    uri         TEXT,
    sample_rate INTEGER     NOT NULL,
    channels    INTEGER     NOT NULL DEFAULT 1,
    UNIQUE(station_id, name)
);

-- ============================================================
-- Core Event Tables
-- ============================================================

CREATE TABLE detections (
    id                  BLOB(16)    NOT NULL PRIMARY KEY,
    station_id          BLOB(16)    NOT NULL REFERENCES stations(id),
    source_id           BLOB(16)    REFERENCES audio_sources(id),
    model_id            INTEGER     NOT NULL REFERENCES models(id),
    label_id            INTEGER     NOT NULL REFERENCES labels(id),
    detected_at         INTEGER     NOT NULL,
    confidence          REAL        NOT NULL,
    snippet_path        TEXT,
    snippet_duration_ms INTEGER,
    snippet_sample_rate INTEGER,
    location_x          REAL,
    location_y          REAL,
    metadata            TEXT
);

CREATE INDEX idx_detections_detected_at ON detections(detected_at);
CREATE INDEX idx_detections_label_time ON detections(label_id, detected_at);
CREATE INDEX idx_detections_time_label ON detections(detected_at, label_id);
CREATE INDEX idx_detections_source_time ON detections(source_id, detected_at);
CREATE INDEX idx_detections_model_time ON detections(model_id, detected_at);

CREATE TABLE detection_predictions (
    detection_id    BLOB(16)    NOT NULL REFERENCES detections(id) ON DELETE CASCADE,
    rank            INTEGER     NOT NULL,
    label_id        INTEGER     NOT NULL REFERENCES labels(id),
    confidence      REAL        NOT NULL,
    PRIMARY KEY (detection_id, rank)
) WITHOUT ROWID;

-- ============================================================
-- Embeddings & Individual Recognition (Phase 3)
-- ============================================================

CREATE TABLE embeddings (
    detection_id    BLOB(16)    NOT NULL PRIMARY KEY REFERENCES detections(id) ON DELETE CASCADE,
    embedding       BLOB        NOT NULL,
    embedding_dim   INTEGER     NOT NULL
);

CREATE TABLE individuals (
    id                      BLOB(16)    NOT NULL PRIMARY KEY,
    scientific_name         TEXT        NOT NULL,
    label                   TEXT        NOT NULL,
    reference_embedding     BLOB,
    reference_embedding_dim INTEGER,
    enrolled_at             INTEGER     NOT NULL,
    notes                   TEXT
);

CREATE INDEX idx_individuals_species ON individuals(scientific_name);

CREATE TABLE individual_matches (
    id              BLOB(16)    NOT NULL PRIMARY KEY,
    individual_id   BLOB(16)    NOT NULL REFERENCES individuals(id) ON DELETE CASCADE,
    detection_id    BLOB(16)    NOT NULL REFERENCES detections(id) ON DELETE CASCADE,
    similarity      REAL        NOT NULL,
    matched_at      INTEGER     NOT NULL
);

CREATE INDEX idx_matches_individual ON individual_matches(individual_id, matched_at);
CREATE INDEX idx_matches_detection ON individual_matches(detection_id);

-- ============================================================
-- Review & Annotation (Phase 4)
-- ============================================================

CREATE TABLE detection_reviews (
    detection_id    BLOB(16)    NOT NULL PRIMARY KEY REFERENCES detections(id) ON DELETE CASCADE,
    status          TEXT        NOT NULL CHECK(status IN ('correct', 'false_positive')),
    reviewed_at     INTEGER     NOT NULL,
    comment         TEXT
);
