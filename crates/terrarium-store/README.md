# terrarium-store

Persistence: the `SQLite` database in the app-data directory.

## Owns

- The schema and its forward-only migrations.
- Appending observations.
- The sim cache.
- Settings.
- JSONL export and import.

## Why it is a crate of its own

Observations are the only record of light, soil and weather over time. They are append-only and
live in one place, together with the migrations that shape them.

## Rules

- It depends on no workspace crate except `terrarium-truth`.
- rusqlite with `bundled`. Migrations are forward-only, and an applied migration is never
  edited.
- Observations are append-only, enforced by `SQLite` triggers (I-4).
- Table names never change (I-12).
- Tests resolve the data directory from `TERRARIUM_DATA_DIR`, never the real one (I-14).

## Requirements

Invariants I-4, I-12 and I-14.
