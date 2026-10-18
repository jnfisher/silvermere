-- ============================================================================
-- World import progress (T-109)
-- ============================================================================

-- One row: `silvermere import` writes started_at (and clears finished_at) before its first
-- table and finished_at after its last, so a start can tell a world an import left half
-- written from a whole one. No row: never imported by this version (a test's seeded world).
CREATE TABLE world_import (
    id          INTEGER PRIMARY KEY, -- always 1
    started_at  TEXT NOT NULL,       -- RFC 3339 UTC, when the last import began writing
    finished_at TEXT                 -- RFC 3339 UTC, when it finished; NULL while it writes or after it was cut off
);
