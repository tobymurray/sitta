-- Effort tracking: records time windows when each audio source was actively
-- receiving audio. A session starts when the first chunk arrives (or after a
-- gap) and ends when chunks stop arriving or the source is removed/shutdown.

CREATE TABLE source_sessions (
    id              BLOB    PRIMARY KEY,
    source_id       BLOB    NOT NULL REFERENCES audio_sources(id),
    started_at      INTEGER NOT NULL,   -- epoch milliseconds
    ended_at        INTEGER,            -- NULL while session is active
    end_reason      TEXT,               -- 'gap', 'shutdown', 'removed'
    chunks_received INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_source_sessions_source_time
    ON source_sessions(source_id, started_at);

CREATE INDEX idx_source_sessions_time_range
    ON source_sessions(started_at, ended_at);
