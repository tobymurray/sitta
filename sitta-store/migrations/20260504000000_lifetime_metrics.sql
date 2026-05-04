-- Persistent counters that need to survive process restarts. The audio
-- pipeline used to keep `clips_saved`, `clips_dropped`, and `bytes_written`
-- only as in-memory atomics, which reset to zero every time the binary
-- restarted. With frequent rolling restarts during development that meant
-- "clips_dropped: 0" said nothing useful — the count had been reset, not
-- earned.
--
-- Counters are stored as named rows so new metrics can be added without
-- a schema migration.

CREATE TABLE lifetime_metrics (
    key   TEXT    NOT NULL PRIMARY KEY,
    value INTEGER NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO lifetime_metrics (key, value) VALUES
    ('clips_saved', 0),
    ('clips_dropped', 0),
    ('clips_failed', 0),
    ('bytes_written', 0);
