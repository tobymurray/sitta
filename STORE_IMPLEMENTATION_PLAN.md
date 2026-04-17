# sitta-store Implementation Plan

Implement the SQLite persistence layer in the `sitta-store` crate so that
detections, embeddings, and reference data are durably stored instead of only
logged to tracing.

The schema is already designed in `sitta-store/schema.sql`. SQLx provides
compile-time verification that every query is valid against that schema.

---

## Step 1 — Add dependencies ✓

Add `sqlx` with the `sqlite` and `runtime-tokio` features to
`sitta-store/Cargo.toml`. The `bundled` feature on the underlying
`libsqlite3-sys` compiles SQLite from source, which avoids system-library
version mismatches when cross-compiling to aarch64 (Raspberry Pi OS, Orange
Pi Debian).

```toml
[dependencies]
sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio"] }
uuid = { workspace = true }
chrono = { workspace = true }
serde = { workspace = true }
serde_json = "1.0"
thiserror = { workspace = true }
tracing = { workspace = true }
```

Also add `serde_json` and `sqlx` to workspace dependencies in the root
`Cargo.toml`. Enable `bundled` on `libsqlite3-sys` via a Cargo feature
flag or by setting the `SQLITE_SYSTEM_LIB` override — check the sqlx docs
for the current recommended approach.

Install the CLI tool: `cargo install sqlx-cli --features sqlite`.

---

## Step 2 — Migration and database bootstrap ✓

Use SQLx's built-in migration system instead of manual DDL + `PRAGMA
user_version`. This gives us versioned, ordered migrations with a
`_sqlx_migrations` tracking table.

### Create the initial migration

```
cargo sqlx migrate add initial_schema
```

This creates `sitta-store/migrations/<timestamp>_initial_schema.sql`. Paste
the full DDL from `sitta-store/schema.sql` into it (the `CREATE TABLE` and
`CREATE INDEX` statements — not the PRAGMAs, those run per-connection).

### Database struct

Create `sitta-store/src/db.rs` with a `Database` struct that wraps a
`sqlx::SqlitePool`.

### `Database::open(path) -> Result<Database>`

1. Build the connection pool via `SqlitePoolOptions`:
   ```rust
   let pool = SqlitePoolOptions::new()
       .max_connections(4)            // 1 writer + readers under WAL
       .after_connect(|conn, _| Box::pin(async move {
           // PRAGMAs run on every new connection in the pool
           sqlx::raw_sql(
               "PRAGMA foreign_keys = ON;
                PRAGMA busy_timeout = 5000;
                PRAGMA synchronous = NORMAL;
                PRAGMA cache_size = -8000;
                PRAGMA temp_store = MEMORY;"
           ).execute(&mut *conn).await?;
           Ok(())
       }))
       .connect(&format!("sqlite://{}?mode=rwc", path))
       .await?;
   ```
2. Set WAL mode (only needs to run once per database file, but is
   idempotent):
   ```rust
   sqlx::raw_sql("PRAGMA journal_mode = WAL;")
       .execute(&pool).await?;
   ```
3. Run pending migrations:
   ```rust
   sqlx::migrate!("./migrations")
       .run(&pool).await?;
   ```
4. Return `Database { pool }`.

### Design notes

- `SqlitePool` is `Clone + Send + Sync` — pass it freely across async
  tasks. The pool serializes writes internally; WAL mode allows concurrent
  reads.
- No dedicated writer thread or mpsc channel needed. The pool handles
  connection lifecycle and write serialization. Consumers call insert
  methods directly with `&self.pool`.
- The migration system replaces `PRAGMA user_version`. Future schema
  changes are new migration files, applied automatically on startup.

---

## Step 3 — Row types and conversions ✓

Create `sitta-store/src/models.rs` with plain structs that map to database
rows, using `sqlx::FromRow` for query results.

```rust
#[derive(sqlx::FromRow)]
pub struct DetectionRow {
    pub id: Vec<u8>,             // BLOB(16) → Vec<u8>, convert to Uuid
    pub station_id: Vec<u8>,
    pub source_id: Option<Vec<u8>>,
    pub model_id: i64,
    pub label_id: i64,
    pub detected_at: i64,        // Unix ms
    pub confidence: f64,         // SQLite REAL is f64
    pub snippet_path: Option<String>,
    // ...
}
```

Key conversions:

| Rust                    | SQLite          | Approach                                      |
|-------------------------|-----------------|-----------------------------------------------|
| `Uuid`                  | `BLOB(16)`      | Bind as `uuid.as_bytes().as_slice()`, read as `Vec<u8>` then `Uuid::from_slice()` |
| `DateTime<Utc>`         | `INTEGER` (ms)  | `dt.timestamp_millis()` / `DateTime::from_timestamp_millis()` |
| `f32` confidence        | `REAL`          | Bind as `f64`, cast on read                   |
| `Vec<f32>` embedding    | `BLOB`          | `bytemuck::cast_slice::<f32, u8>` to bind, reverse on read |
| `LabelType` enum        | `TEXT`          | `impl sqlx::Type + Encode + Decode`, or use `String` in the row struct and convert |
| metadata JSON           | `TEXT`          | `serde_json::to_string` / `from_str`          |

Provide thin conversion functions (`DetectionRow → Detection`, etc.) that
handle the `Vec<u8>` → `Uuid` mapping. Keep the row structs close to the
wire format so `sqlx::FromRow` derive works without custom impls.

---

## Step 4 — Insert and query functions ✓

Add methods to `Database`. Each method uses `sqlx::query!` for compile-time
checked SQL. Group by concern:

### Seeding (called once at startup)

- `upsert_station(&self, id: &[u8], name: &str, lat: Option<f64>,
  lon: Option<f64>)` — `INSERT OR REPLACE`.
- `upsert_audio_source(&self, ...)` — `INSERT OR REPLACE` per source in
  config.
- `upsert_model(&self, name: &str, version: &str, ...) -> i64`
  — `INSERT OR IGNORE`, then `SELECT id` to return the INTEGER PK.
- `seed_labels(&self, model_id: i64, labels: &[LabelEntry])`
  — Bulk `INSERT OR IGNORE` inside a transaction. Use
  `sqlx::query!` in a loop within `pool.begin()` / `tx.commit()`.
- `load_label_id_cache(&self) -> HashMap<(i64, usize), i64>`
  — `SELECT id, model_id, label_index FROM labels`. Called once at startup,
  cached in memory. Avoids per-detection label lookups.

### Detection writes (called on every inference result)

- `insert_detection(&self, detection: &NewDetection)`
  — Single-row `INSERT` via `sqlx::query!`.
- `insert_predictions(&self, detection_id: &[u8], predictions: &[Prediction])`
  — Batch `INSERT` in a transaction for top-k secondary predictions.
- `insert_embedding(&self, detection_id: &[u8], embedding: &[u8], dim: u32)`
  — `INSERT` the raw bytes.

### Reads (for future API, not needed in Step 5)

Implement as needed when building `sitta-api`. The 10 query patterns from
the schema design document serve as the starting point. All will use
`sqlx::query_as!` for compile-time checking.

---

## Step 5 — Wire into the main pipeline

Modify `sitta-bin/src/main.rs` to open the database at startup and pass
the pool to inference consumers.

### Architecture

SQLx's `SqlitePool` handles connection management and write serialization
internally. No dedicated writer thread or mpsc channel needed:

```
[BirdNET consumer] ──┐
                     ├──► Database (SqlitePool) ──► SQLite WAL
[Perch consumer]   ──┘
```

1. `main()` calls `Database::open(&config.store.path).await?`.
2. Clone `Database` (wraps `Arc<SqlitePool>`) into each consumer closure.
3. Each consumer calls `db.insert_detection(...)` directly after inference.
   The pool serializes concurrent writes.

### Startup seeding

Before spawning consumers:

1. `db.upsert_station(...)` with `config.station`.
2. For each `config.audio.sources`: `db.upsert_audio_source(...)`.
3. For each loaded model: `db.upsert_model(...)`, then `db.seed_labels(...)`
   from the model's label set.
4. `let label_cache = db.load_label_id_cache().await?`
   — `Arc<HashMap<(i64, usize), i64>>`, cloned into each consumer.

### Consumer changes

In `handle_chunk` and `spawn_perch_consumer`, after a successful inference:

1. For each `Classification` above threshold:
   - Look up `label_id` from cache using `(model_id, classification.label_index)`.
   - Build a `NewDetection` struct (generate UUIDv7, capture `detected_at`
     from `chunk.captured_at`, fill snippet fields as `None` for now).
   - `db.insert_detection(&detection).await`
   - `db.insert_predictions(detection_id, &secondary_preds).await`
   - If embeddings present: `db.insert_embedding(detection_id, &bytes, dim).await`
2. Keep the existing `tracing::info!` call — logging and persistence are
   independent.

---

## Step 6 — Expose model labels for seeding

The `Classifier` trait currently exposes `name()`, `sample_rate()`, and
`window_samples()`. The database seeding step needs the full label set.

Add a method to the `Classifier` trait (or a separate `LabelSource` trait):

```rust
fn labels(&self) -> &[LabelEntry];
fn model_version(&self) -> &str;
```

`BirdNet` already has a `labels()` method that returns the raw label
strings. Extend it (or add a wrapper) to return structured entries that
include `label_index`, `scientific_name`, `common_name`, `label_type`, and
`taxon_code`.

This is the only change outside `sitta-store` and `sitta-bin`.

---

## Step 7 — Config: database path

Add a `[store]` section to `config.toml`:

```toml
[store]
path = "/var/lib/sitta/sitta.db"
```

And a corresponding `StoreConfig` struct in `sitta-bin/src/config.rs`:

```rust
pub struct StoreConfig {
    pub path: String,
}
```

Default to `"./sitta.db"` if the section is omitted.

---

## Step 8 — Compile-time query workflow ✓

SQLx's `query!` macros check SQL against a real database at compile time.
For this to work in CI and cross-compilation, use offline mode:

### Development (local machine)

Set `DATABASE_URL` to a local SQLite file:

```bash
export DATABASE_URL="sqlite://dev.db"
```

Create the database and run migrations:

```bash
cargo sqlx database create
cargo sqlx migrate run --source sitta-store/migrations
```

Now `cargo build` runs `query!` macros against this database. Column names,
types, and nullability are all checked at compile time.

### Preparing for CI / cross-compilation

After any schema or query change:

```bash
cargo sqlx prepare --workspace
```

This caches query metadata in `.sqlx/` (committed to the repo). With
`SQLX_OFFLINE=true`, builds use the cache instead of a live database.

### CI pipeline

```bash
SQLX_OFFLINE=true cargo build --target aarch64-unknown-linux-gnu
```

No database needed at build time.

---

## Step 9 — Smoke test

Write integration tests in `sitta-store/tests/` using SQLx's test support:

```rust
#[sqlx::test(migrations = "migrations")]
async fn test_detection_roundtrip(pool: SqlitePool) {
    let db = Database::from_pool(pool);
    // ... seed, insert, read back, assert
}
```

`sqlx::test` creates a temporary database, runs migrations, and tears down
after the test. Tests should verify:

1. Seed a station, model, and labels.
2. Insert a detection with predictions and an embedding.
3. Read them back and assert field values.
4. Verify foreign key constraints reject bad references.
5. Verify `PRAGMA integrity_check` passes.

---

## What this plan does NOT cover (future work)

- **Audio snippet saving** — Writing WAV files to disk and populating
  `snippet_path`. Prerequisite: decide on directory layout and retention
  policy. Likely a separate `sitta-store` or `sitta-bin` concern.
- **REST/MQTT API** — The `sitta-api` crate will share the `SqlitePool`
  and run the query patterns from the schema design. Depends on the store
  layer being operational first.
- **Individual recognition** — Embedding comparison, individual enrollment,
  and match recording. The tables exist in the schema; the logic lives in a
  future recognition module.
- **Review/annotation UI** — Requires the API and a frontend.
- **Database maintenance** — Periodic `PRAGMA optimize`, `PRAGMA
  integrity_check` on startup, retention-based deletion of old detections.
  Not urgent for Phase 2 but should be added before long-running
  deployments.
