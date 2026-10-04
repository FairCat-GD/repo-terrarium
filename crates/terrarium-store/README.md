# terrarium-store

Persistence: the `SQLite` database in the app-data directory.

## Owns

- The schema and its forward-only migrations.
- Terrariums: created, found by seed to relink, activated, disconnected, and deleted only
  through the guarded path.
- Appending and querying observations.
- The sim cache.
- Settings and install facts.
- JSONL export and import of observations.
- The machine identifiers data lives under (`identifiers`).

## Why it is a crate of its own

Observations are the only record of light, soil and weather over time. They are append-only and
live in one place, together with the migrations that shape them.

## Rules

- It depends on no workspace crate except `terrarium-truth`.
- rusqlite with `bundled`, so every install runs the same `SQLite`. Migrations are forward-only;
  an applied migration is never edited, and a test pins each one's hash.
- **Observations are append-only** (I-4). Migration 1's triggers refuse every `UPDATE` and every
  `DELETE`. `recursive_triggers` is on, so an `INSERT OR REPLACE` that would replace a row fires
  the delete trigger too. The store only ever inserts.
- **One path deletes observations:** `Store::delete_terrarium`. Inside one transaction it sets
  `install.deleting_terrarium`, which the delete trigger checks, deletes the terrarium's cache,
  observations and row, and clears the key. No other API can set that key.
- Stored integers are `i64`; Q16 values are their raw `i32`. rusqlite's `fallible_uint` stays
  off, so a `u64` cannot reach a column.
- Table names and machine identifiers never change (I-12).

## The data directory

`DataDir::resolve()` uses `TERRARIUM_DATA_DIR` when it is set, and it must be an absolute path.
Otherwise it uses the OS's per-user local app-data directory, in a folder named by the bundle id:

```text
macOS    ~/Library/Application Support/io.github.faircat-gd.repo-terrarium/terrarium.sqlite3
Windows  %LOCALAPPDATA%\io.github.faircat-gd.repo-terrarium\terrarium.sqlite3
```

Under test it panics when `TERRARIUM_DATA_DIR` is not set (I-14). "Under test" means this
crate's own tests, or any process nextest runs: nextest sets `NEXTEST` in every test process.
Tests and harnesses use `DataDir::at` on a directory they own.

## Observation export (format version 1)

JSON Lines in UTF-8, one JSON object per line, each line ending in `\n`. The first line is the
header:

```json
{"format":"repo-terrarium-observations","format_version":1,"seed":"<64 hex>","sim_version":1}
```

Then each observation follows in id order, with every column of `observations` in schema order
except `terrarium_id`, which the header's seed stands for:

```json
{"id":1,"kind":"weather","adapter_version":2,"recorded_at":1700000200,
 "head_commit":"<40 or 64 hex>","head_branch":"main","at_default_tip":true,
 "source":"ci:main","report_mtime":null,"content_hash":"<64 hex>",
 "raw":"{\"state\":\"pass\"}","normalized":65536,"confidence":32768,
 "contributors":[{"path":"tests/pond.rs","value":1}]}
```

The example is wrapped here; in a file each observation is one line.

- BLOBs (`seed`, `content_hash`) are lowercase hex.
- `raw` is the stored text, itself JSON.
- `normalized` and `confidence` are Q16.16 integers in [0, 65536].
- `at_default_tip` is a boolean.
- `head_branch`, `report_mtime` and `normalized` may be `null`, but are never left out: a line
  missing any column is refused.
- A line holds at most 1 MiB, not counting its `\n` or `\r\n` ending.

Import refuses, writing nothing, any of these:
- a file whose first line is not this header;
- a `format_version` other than 1;
- a seed other than the terrarium's;
- a line that does not decode into a valid observation.

Otherwise each line merges by (`kind`, `source`, `head_commit`, `recorded_at`):
- a new key is inserted;
- a line equal to the stored row in every column is a duplicate;
- any other line is a conflict, reported with the differing columns, and the stored row is
  kept.

A line's `id` belongs to the install that exported it and is not kept.

## Raw shapes (`raw`)

Each kind's `raw` is a JSON object. Shapes are keyed by kind, not `adapter_version`, and only
ever gain optional keys; readers ignore keys they do not know.

| kind | shape | enforced |
|---|---|---|
| `coverage` | `{"format":"lcov"\|"cobertura","lines_found":n,"lines_hit":n}` | JSON only |
| `dead_code` | `{"unused_items":n,"exported_items":n}` | yes |
| `weather` | `{"state":"pass"\|"fail"\|"unknown"}` | JSON only |
| `ref_rewrite` | `{"ref":"<ref>","old":"<hex>","new":"<hex>"}`, `normalized` null | yes |
| `complexity`, `todo_density` | defined by the tree-sitter adapter | JSON only |

The store enforces what truth reads, so every stored row converts:
`StoredObservation::to_observation` gives replay its `Observation`.

## Requirements

Invariants I-4, I-12 and I-14.
