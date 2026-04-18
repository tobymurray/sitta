-- ============================================================
-- Sitta SQLite Schema v1
-- ============================================================
--
-- Connection PRAGMAs (run on every connection open):
--
--   PRAGMA journal_mode = WAL;
--   PRAGMA foreign_keys = ON;
--   PRAGMA busy_timeout = 5000;
--   PRAGMA synchronous = NORMAL;
--   PRAGMA cache_size = -8000;        -- 8 MB negative = KB
--   PRAGMA temp_store = MEMORY;
--
-- Schema version (run once at creation, bump on migration):
--
--   PRAGMA user_version = 1;
--

-- ============================================================
-- Reference / Dimension Tables
-- ============================================================

-- One row per physical monitoring location. Phase 2 has one
-- station per instance; the table supports Phase 4 federation
-- where merged databases contain multiple stations.
CREATE TABLE stations (
    id          BLOB(16)    NOT NULL PRIMARY KEY,  -- UUIDv7
    name        TEXT        NOT NULL,
    latitude    REAL,
    longitude   REAL
);

-- Each AI model that can produce detections. INTEGER PK keeps
-- the FK footprint small in the high-volume detections table.
CREATE TABLE models (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,  -- "birdnet", "perch"
    version         TEXT    NOT NULL,  -- "2.4", "2"
    sample_rate     INTEGER NOT NULL,  -- Hz
    window_samples  INTEGER NOT NULL,  -- samples per inference window
    has_embeddings  INTEGER NOT NULL DEFAULT 0,
    embedding_dim   INTEGER,           -- NULL if no embeddings
    UNIQUE(name, version)
);

-- Model-specific label sets. Every label belongs to exactly one
-- model because the same scientific name can appear at different
-- label_index positions across models. INTEGER PK for the same
-- reason as models -- this ID appears in every detection row.
CREATE TABLE labels (
    id              INTEGER PRIMARY KEY,
    model_id        INTEGER NOT NULL REFERENCES models(id),
    label_index     INTEGER NOT NULL,      -- position in model's output tensor
    scientific_name TEXT,                   -- NULL for non-species (noise, environment)
    common_name     TEXT    NOT NULL,       -- display name: "Barn Owl", "Engine", "Rain"
    label_type      TEXT    NOT NULL DEFAULT 'species',  -- species | noise | environment
    taxon_code      TEXT,                  -- eBird species code, NULL for non-species
    UNIQUE(model_id, label_index)
);

-- Lookup by scientific name across models (cross-model species queries)
CREATE INDEX idx_labels_scientific ON labels(scientific_name) WHERE scientific_name IS NOT NULL;
-- Lookup by (model_id, scientific_name) for insert-time label resolution
CREATE INDEX idx_labels_model_scientific ON labels(model_id, scientific_name);

-- RTSP streams and local microphones. Detections reference these
-- by FK so source config changes don't rewrite detection history.
CREATE TABLE audio_sources (
    id          BLOB(16)    NOT NULL PRIMARY KEY,  -- UUIDv7
    station_id  BLOB(16)    NOT NULL REFERENCES stations(id),
    name        TEXT        NOT NULL,      -- "north_paddock"
    source_type TEXT        NOT NULL,      -- "rtsp" | "local"
    uri         TEXT,                      -- RTSP URL or device path
    sample_rate INTEGER     NOT NULL,
    channels    INTEGER     NOT NULL DEFAULT 1,
    UNIQUE(station_id, name)
);

-- ============================================================
-- Core Event Tables
-- ============================================================

-- One row per classification above threshold. The primary (top-1)
-- prediction is stored inline; secondary predictions go in
-- detection_predictions. station_id is denormalized from
-- audio_sources for efficient per-station queries without joins.
CREATE TABLE detections (
    id                  BLOB(16)    NOT NULL PRIMARY KEY,  -- UUIDv7
    station_id          BLOB(16)    NOT NULL REFERENCES stations(id),
    source_id           BLOB(16)    REFERENCES audio_sources(id),  -- nullable: file-based analysis
    model_id            INTEGER     NOT NULL REFERENCES models(id),
    label_id            INTEGER     NOT NULL REFERENCES labels(id),
    detected_at         INTEGER     NOT NULL,  -- Unix milliseconds
    confidence          REAL        NOT NULL,  -- [0.0, 1.0]
    snippet_path        TEXT,                  -- path to saved WAV on disk
    snippet_duration_ms INTEGER,
    snippet_sample_rate INTEGER,
    location_x          REAL,                  -- Phase 5: TDOA x or longitude
    location_y          REAL,                  -- Phase 5: TDOA y or latitude
    metadata            TEXT                   -- JSON blob: noise_floor_db, peak_frequency_hz, inference_time_ms, etc.
);

-- Query 1: recent detections (dashboard live feed)
CREATE INDEX idx_detections_detected_at ON detections(detected_at);
-- Query 2: species history within date range
CREATE INDEX idx_detections_label_time ON detections(label_id, detected_at);
-- Query 3: daily species list (scan date range, extract distinct label_ids)
-- Also covers Query 1 more efficiently as a covering index
CREATE INDEX idx_detections_time_label ON detections(detected_at, label_id);
-- Query 8: source activity over time range
CREATE INDEX idx_detections_source_time ON detections(source_id, detected_at);
-- Query 9: model comparison for same time window
CREATE INDEX idx_detections_model_time ON detections(model_id, detected_at);

-- Top-k alternative predictions for a detection. Rank 1 = second-best
-- (the best prediction is inline on the detections row). WITHOUT ROWID
-- because the composite PK is the only access pattern and rows are small.
CREATE TABLE detection_predictions (
    detection_id    BLOB(16)    NOT NULL REFERENCES detections(id) ON DELETE CASCADE,
    rank            INTEGER     NOT NULL,  -- 1 = second-best, 2 = third-best, ...
    label_id        INTEGER     NOT NULL REFERENCES labels(id),
    confidence      REAL        NOT NULL,
    PRIMARY KEY (detection_id, rank)
) WITHOUT ROWID;

-- ============================================================
-- Embeddings & Individual Recognition (Phase 3)
-- ============================================================

-- One embedding per detection that produces one (Perch today,
-- possibly BirdNET v3 later). 1:1 with detections, keyed by
-- detection_id. BLOB stores raw little-endian f32 bytes;
-- embedding_dim records the vector length for validation.
CREATE TABLE embeddings (
    detection_id    BLOB(16)    NOT NULL PRIMARY KEY REFERENCES detections(id) ON DELETE CASCADE,
    embedding       BLOB        NOT NULL,  -- raw f32 LE bytes, e.g. 1536 * 4 = 6144 bytes
    embedding_dim   INTEGER     NOT NULL   -- 1536 for Perch v2
);

-- User-labeled known individuals for re-identification.
CREATE TABLE individuals (
    id                      BLOB(16)    NOT NULL PRIMARY KEY,  -- UUIDv7
    scientific_name         TEXT        NOT NULL,  -- species this individual belongs to
    label                   TEXT        NOT NULL,  -- user-assigned name: "Barn Owl #1"
    reference_embedding     BLOB,                  -- centroid or exemplar embedding
    reference_embedding_dim INTEGER,
    enrolled_at             INTEGER     NOT NULL,  -- Unix milliseconds
    notes                   TEXT
);

CREATE INDEX idx_individuals_species ON individuals(scientific_name);

-- Records when a detection's embedding matched a known individual.
CREATE TABLE individual_matches (
    id              BLOB(16)    NOT NULL PRIMARY KEY,  -- UUIDv7
    individual_id   BLOB(16)    NOT NULL REFERENCES individuals(id) ON DELETE CASCADE,
    detection_id    BLOB(16)    NOT NULL REFERENCES detections(id) ON DELETE CASCADE,
    similarity      REAL        NOT NULL,  -- cosine similarity [0.0, 1.0]
    matched_at      INTEGER     NOT NULL   -- Unix milliseconds
);

-- Query 5: individual sightings timeline
CREATE INDEX idx_matches_individual ON individual_matches(individual_id, matched_at);
-- Query 7: look up match(es) for a specific detection
CREATE INDEX idx_matches_detection ON individual_matches(detection_id);

-- ── Candidate clustering for individual identification ──────

-- Unmatched Perch embeddings awaiting clustering.
CREATE TABLE candidate_embeddings (
    detection_id    BLOB(16) NOT NULL PRIMARY KEY REFERENCES detections(id) ON DELETE CASCADE,
    scientific_name TEXT     NOT NULL,
    embedding       BLOB     NOT NULL,  -- raw f32 LE bytes
    cluster_id      INTEGER,            -- NULL = unclustered, set by clustering pass
    created_at      INTEGER  NOT NULL   -- Unix milliseconds
);
CREATE INDEX idx_candidates_species ON candidate_embeddings(scientific_name);
CREATE INDEX idx_candidates_cluster ON candidate_embeddings(cluster_id) WHERE cluster_id IS NOT NULL;

-- Discovered embedding clusters. Each cluster is a potential individual.
CREATE TABLE candidate_clusters (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    scientific_name TEXT    NOT NULL,
    centroid        BLOB   NOT NULL,    -- mean embedding (f32 LE bytes)
    centroid_dim    INTEGER NOT NULL,
    member_count    INTEGER NOT NULL DEFAULT 0,
    distinct_days   INTEGER NOT NULL DEFAULT 0,
    first_seen_at   INTEGER NOT NULL,   -- Unix milliseconds
    last_seen_at    INTEGER NOT NULL,   -- Unix milliseconds
    status          TEXT    NOT NULL DEFAULT 'pending'
                    CHECK(status IN ('pending', 'enrolled', 'dismissed')),
    individual_id   BLOB(16) REFERENCES individuals(id)  -- set when enrolled
);
CREATE INDEX idx_clusters_status ON candidate_clusters(status, scientific_name);

-- ============================================================
-- Review & Annotation (Phase 4)
-- ============================================================

-- At most one review per detection (PK = detection_id).
-- Combines verification status and optional free-text comment.
CREATE TABLE detection_reviews (
    detection_id    BLOB(16)    NOT NULL PRIMARY KEY REFERENCES detections(id) ON DELETE CASCADE,
    status          TEXT        NOT NULL CHECK(status IN ('correct', 'false_positive')),
    reviewed_at     INTEGER     NOT NULL,  -- Unix milliseconds
    comment         TEXT
);
