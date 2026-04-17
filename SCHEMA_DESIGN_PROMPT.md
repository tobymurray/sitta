# Schema Design Prompt for Sitta

## Your Task

Design a SQLite database schema for **Sitta**, a Rust bioacoustics engine that identifies bird (and other animal) species from audio on edge hardware (Raspberry Pi 5, Orange Pi 5). The schema will be implemented via `rusqlite` with raw SQL -- no ORM.

## About the Application

Sitta captures audio from multiple RTSP streams (IP cameras/NVRs), runs species classification inference through multiple AI models simultaneously, and produces detection events. It is designed for local-first, privacy-oriented operation on ARM64 SBCs with 2-4 GB RAM.

Key architectural properties:
- **Rust async** (Tokio), inference on blocking threads
- **SQLite with WAL mode** -- single-writer, multiple readers
- **UUIDv7** for all IDs (time-sortable, 128-bit)
- **No ORM** -- raw SQL via `rusqlite`, simple schema, no abstraction tax
- **Edge deployment** -- must perform well on SD cards and eMMC with limited I/O bandwidth

## Models and Label Spaces

Sitta runs **multiple inference models concurrently** on the same audio stream:

### BirdNET v2.4
- **6,522 species** in its label set
- Labels are formatted as `"Scientific Name_Common Name"` (e.g., `"Tyto alba_Barn Owl"`)
- 3-second inference windows at 48 kHz (144,000 samples)
- Produces **classification scores only** (no embeddings)
- Has an associated **meta-model** for geographic/seasonal range filtering
- Label set is **birds only** (plus a few domestic fowl)

### Google Perch v2
- **14,795 labels** in its label set
- Labels include **birds, non-bird animals, environmental sounds, and noise categories**
- 5-second inference windows at 32 kHz (160,000 samples), 3-second stride (2s overlap)
- Produces both **classification scores** and **1,536-dimensional embedding vectors**
- Embeddings enable individual animal identification (cosine similarity matching)

### Future models (the schema must accommodate without migration)
- BirdNET v3.0 (expected to add embedding support)
- BSG Finland (regional specialist model)
- BatNET or other non-bird classifiers
- Custom fine-tuned models

### Label types the schema must support
- **Species** (birds, mammals, amphibians, insects) -- have scientific names, common names, eBird/taxon codes
- **Non-species sounds** -- e.g., "Engine", "Siren", "Human voice", "Rain", "Wind", "Fireworks"
- **Noise categories** -- e.g., "Background noise", "Silence"
- The same scientific name may appear in multiple models' label sets with different label indices

## Current In-Memory Data Structures (Rust)

These are the types that will feed into the persistence layer:

```rust
// sitta-audio/src/chunk.rs
pub struct AudioChunk {
    pub id: Uuid,                    // UUIDv7, time-sortable
    pub source_name: String,         // e.g., "north_paddock"
    pub timestamp_ns: u64,           // Monotonic, relative to capture start
    pub captured_at: DateTime<Utc>,  // Wall-clock time
    pub sample_rate: u32,            // 48000 or 32000
    pub channels: u16,               // Typically 1 (mono)
    pub samples: Vec<f32>,           // Not stored in DB
}

// sitta-inference/src/model.rs
pub struct Classification {
    pub label_index: usize,          // Index in model's label set
    pub species: Species,
    pub confidence: f32,             // [0.0, 1.0] post-sigmoid
}

pub struct Species {
    pub scientific_name: String,     // e.g., "Tyto alba"
    pub common_name: String,         // e.g., "Barn Owl"
    pub taxon_code: Option<String>,  // e.g., "barowl1" (eBird code)
}
```

## Planned Detection Event (API output, for reference)

```json
{
  "id": "01961074-...",
  "timestamp": "2026-04-15T08:30:00Z",
  "station_id": "station_01",
  "species": {
    "scientific_name": "Tyto alba",
    "common_name": "Barn Owl",
    "taxon_id": "barowl1",
    "model": "birdnet"
  },
  "confidence": 0.92,
  "individual": {
    "id": "a1b2c3d4-...",
    "label": "Barn Owl #1",
    "similarity": 0.91
  },
  "audio": {
    "sample_rate": 48000,
    "duration_ms": 3000,
    "channel_id": 0,
    "snippet_path": "/var/lib/sitta/snippets/01961074-....wav"
  },
  "location": null,
  "metadata": {
    "noise_floor_db": -42.5,
    "peak_frequency_hz": 3200.0,
    "inference_time_ms": 285
  }
}
```

## Feature Requirements the Schema Must Support

### Phase 2 (immediate)
1. **Detection log**: Every detection above threshold is persisted with: timestamp, source, model, label, confidence, audio snippet path, inference metadata
2. **Multi-model provenance**: The same 3-second window may produce detections from both BirdNET and Perch. Each detection must record which model produced it.
3. **Audio snippet references**: Path to saved WAV file on disk, with duration and sample rate
4. **Station metadata**: ID, name, latitude, longitude (single station per instance, but the schema should support multi-station for Phase 4 federation)
5. **Audio source tracking**: Which RTSP stream or local mic produced the audio (URI, transport, display name)
6. **Top-k secondary predictions**: When BirdNET says "92% Barn Owl", it also has ranked alternatives (e.g., "7% Tawny Owl", "3% Long-eared Owl"). Store these with rank.

### Phase 3 (individual recognition)
7. **Embedding storage**: 1,536-dimensional float32 vectors as binary blobs (~6 KB each). One embedding per Perch inference window. Must support: "find the embedding for detection X" and "find all embeddings for individual Y".
8. **Individual profiles**: User-labeled known individuals (e.g., "Barn Owl #1"). Each individual has a reference embedding and an enrollment date.
9. **Individual matches**: When a new detection's embedding matches a known individual above threshold, record: individual ID, detection ID, cosine similarity score.

### Phase 4 (multi-station, dashboard)
10. **Detection review/verification**: Mark detections as "correct" or "false_positive" with timestamps. At most one review per detection.
11. **User annotations**: Free-text comments on detections.
12. **Export-friendly**: Schema should support efficient CSV/JSON-lines export with date-range and species filters.

### Phase 5 (spatial)
13. **TDOA location estimates**: Optional (x, y) or (lat, lon) position estimate per detection from multi-microphone triangulation. Most detections will not have this.

## Constraints and Preferences

### Must have
- All IDs are UUIDv7 stored as BLOB(16) -- time-sortable, no auto-increment
- Timestamps are Unix milliseconds (INTEGER) -- not ISO strings, not separate date/time columns
- WAL mode, `PRAGMA foreign_keys = ON`, `PRAGMA journal_mode = WAL`
- Embeddings stored as binary blobs (BLOB), not as text or JSON arrays
- Schema must handle **non-species labels** (noise, environment) cleanly -- not every detection is a "species"

### Should have
- Normalized where it saves significant space (species/labels, models, sources) but **not over-normalized** -- this runs on a Pi, not a data warehouse
- Composite indexes for the primary query patterns (see below)
- Schema versioning mechanism (even if just a `schema_version` pragma or metadata table)

### Must NOT have
- No ORM conventions (no `created_at`/`updated_at` on every table unless genuinely needed)
- No GORM-style `deleted_at` soft deletes
- No JSON columns for structured data that will be queried (metadata blobs are fine for extensible diagnostics)
- No separate date and time columns (a single timestamp field only -- this was a well-documented mistake in both BirdNET-Pi and BirdNET-Go v1)
- No string-based foreign keys -- use integer or blob PKs with proper FK constraints

## Primary Query Patterns (design indexes for these)

1. **Recent detections**: `SELECT ... WHERE detected_at > ? ORDER BY detected_at DESC LIMIT 50` (dashboard live feed)
2. **Species history**: `SELECT ... WHERE label_id = ? AND detected_at BETWEEN ? AND ? ORDER BY detected_at` (species detail page)
3. **Daily species list**: `SELECT DISTINCT label_id WHERE detected_at BETWEEN ? AND ?` (daily summary)
4. **Detection + predictions**: Fetch a detection with its top-k alternative predictions (detail view)
5. **Individual sightings**: `SELECT ... WHERE individual_id = ? ORDER BY detected_at DESC` (individual timeline)
6. **Embedding lookup by detection**: `SELECT embedding FROM embeddings WHERE detection_id = ?`
7. **All embeddings for an individual**: For cosine similarity re-computation during enrollment updates
8. **Source activity**: Detections grouped by audio source over a time range
9. **Model comparison**: Same time window, different models' detections side-by-side
10. **Unreviewed detections**: Detections that have not yet been verified (for review queue)

## Lessons from the BirdNET Ecosystem (incorporate these)

### From BirdNET-Pi (what to avoid)
- **Single flat table, no PK**: BirdNET-Pi uses one `detections` table with 12 columns, no primary key, no foreign keys, no normalization. Species names are repeated as raw strings in every row.
- **Separate Date + Time string columns**: Complicates range queries, requires dual indexes. Use a single integer timestamp.
- **Common name baked in at detection time**: Changing language settings leaves historical records in the old language. Store scientific name as the canonical identifier; resolve common names at query time.
- **Analysis parameters stored per-row**: Lat, lon, sensitivity, overlap, cutoff are configuration values that rarely change but are stored in every detection. Normalize these out.
- **No concurrency handling**: No WAL mode, database-busy errors under concurrent access.

### From BirdNET-Go v1 -> v2 migration (what they fixed)
- **Normalized labels**: V2 introduced a `labels` table keyed by `(scientific_name, model_id)`. Labels are model-specific because different models have different label sets.
- **Multi-model support via `ai_models` table**: `(name, version, variant)` composite unique. Each detection references its producing model.
- **Audio source normalization**: Separate `audio_sources` table with URI, node name, source type, display name. Detections reference by FK.
- **Single timestamp source of truth**: `detected_at` as Unix timestamp replaced the v1 separate `Date`/`Time` string columns.
- **Taxonomic class**: A `taxonomic_classes` table ("Aves", "Chiroptera") for birds vs. bats. Their `labels` table has a nullable FK to this -- null means non-species (noise, environment).
- **Label types**: A `label_types` table ("species", "noise", "environment", "device") to categorize what a label represents.
- **Ranked predictions**: `detection_predictions` table with a `rank` column (1 = second-best, 2 = third-best).
- **Nullable source FK**: Not all detections have a known audio source (e.g., file-based analysis). SourceID is nullable.

### From BirdNET-Go v2 migration pain points (what to get right from day one)
- **Data loss during dual-write cutover**: Hours of detections lost between "migration complete" and system restart. Lesson: get the schema right the first time so you don't need a complex migration.
- **SQLite index corruption on SD cards**: Unclean shutdowns on Raspberry Pi cause index corruption. Lesson: use WAL mode, consider periodic `PRAGMA integrity_check`, and handle `REINDEX` gracefully.
- **GORM field-name collisions**: ORM-specific issue (we avoid this by using raw SQL).
- **Browser timeout during prerequisite checks**: Large databases on slow I/O caused integrity checks to exceed HTTP timeouts. Lesson: async/background integrity checks.

## Deliverable

Provide:
1. **Complete SQL DDL** (`CREATE TABLE`, `CREATE INDEX` statements) for all tables
2. **Brief rationale** for each table explaining what it stores and why it's structured that way
3. **The `PRAGMA` statements** needed at connection open
4. **A schema version table** or mechanism
5. **Example queries** for the 10 query patterns listed above
6. **Rust struct sketches** for the main types that would map to these tables (not ORM models -- just plain structs that `rusqlite` rows would deserialize into)
7. **Migration notes**: What would need to change if a future model produces 3,072-dim embeddings instead of 1,536? What if we add weather data? Design for these without implementing them.

Keep the schema as simple as possible while fully supporting the requirements. Prefer fewer tables with nullable columns over many sparse join tables. This is a bird feeder monitor, not an enterprise data warehouse.
