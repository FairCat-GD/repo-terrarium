CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;  -- JSON values
CREATE TABLE install  (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;  -- last_sim_version, first_run_done, …

CREATE TABLE terrariums (
  id              INTEGER PRIMARY KEY,
  seed            BLOB    NOT NULL UNIQUE,   -- 32 bytes, SHA-256 of root commit id (D-21)
  repo_path       TEXT    NOT NULL,          -- last known absolute path
  repo_name       TEXT    NOT NULL,
  default_branch  TEXT    NOT NULL,          -- recorded at connect; changing it forces full replay
  install_salt    BLOB    NOT NULL,          -- 16 bytes; cosmetics only; re-seed replaces it
  connected_at    INTEGER NOT NULL,          -- unix seconds UTC
  active          INTEGER NOT NULL CHECK (active IN (0,1))
) STRICT;

CREATE TABLE observations (                  -- append-only (triggers below)
  id              INTEGER PRIMARY KEY,
  terrarium_id    INTEGER NOT NULL REFERENCES terrariums(id),
  kind            TEXT    NOT NULL,          -- §4.3
  adapter_version INTEGER NOT NULL,
  recorded_at     INTEGER NOT NULL,          -- unix seconds UTC
  head_commit     TEXT    NOT NULL,          -- hex oid
  head_branch     TEXT,                      -- NULL when detached
  at_default_tip  INTEGER NOT NULL CHECK (at_default_tip IN (0,1)),
  source          TEXT    NOT NULL,          -- weather: 'ci:<branch>' | 'local'; '' for every other kind
  report_mtime    INTEGER,                   -- unix seconds; NULL when not file-based
  content_hash    BLOB    NOT NULL,          -- SHA-256 of the raw report bytes
  raw             TEXT    NOT NULL,          -- JSON, kind-specific, size-capped
  normalized      INTEGER,                   -- Q16.16 in [0, 65536], health-oriented (1 = best); NULL = unknown
  confidence      INTEGER NOT NULL,          -- Q16.16 in [0, 65536]
  contributors    TEXT    NOT NULL,          -- JSON array, at most 20 {path, value}
  UNIQUE (terrarium_id, kind, source, head_commit, recorded_at)
) STRICT;
CREATE TRIGGER observations_no_update BEFORE UPDATE ON observations
  BEGIN SELECT RAISE(ABORT, 'observations are append-only'); END;
CREATE TRIGGER observations_no_delete BEFORE DELETE ON observations
  WHEN (SELECT value FROM install WHERE key = 'deleting_terrarium') IS NULL
  BEGIN SELECT RAISE(ABORT, 'observations are append-only'); END;

CREATE TABLE sim_cache (                     -- optimisation only; never authoritative (D-27)
  terrarium_id    INTEGER PRIMARY KEY REFERENCES terrariums(id),
  sim_version     INTEGER NOT NULL,
  tips_hash       BLOB    NOT NULL,
  obs_high_water  INTEGER NOT NULL,
  tick            INTEGER NOT NULL,
  state           BLOB    NOT NULL           -- postcard(SimState)
) STRICT;
