# terrarium-engine

The headless live pipeline, run on a worker thread.

## Owns

- **Connect** (`live`): `--repo <path>` creates the terrarium or relinks the one with the
  repository's seed, derived as `record-tests` derives it; a plain run takes the one
  `repo.active` names, with its recorded default branch.
- **Refresh** (`live`): git, then the reports, then replay. A report is read only once it has
  been still for 500 ms, and recorded only when its content changed since its format's last
  observation. A default branch moved without fast-forwarding is recorded as a `ref_rewrite`.
  Replay resumes the cached state before the open hour when truth accepts it, and replays in
  full otherwise; the cache is written about once an hour.
- **The worker** (`worker`): the thread, its channel and its sink. A panic in a refresh is
  caught: the scene freezes and `Halted` is sent. Shutting down stops a history read in
  progress.
- **When to refresh** (`schedule`): 500 ms after the watcher's last event, a follow-up for a
  report still being written, the wall clock's hour changing (the hourly tick, and any wake or
  clock jump), a wake from the platform, and a retry once a minute while the repository is
  missing.
- **What to watch** (`watch`): the git directories, the refs and reflogs, the directories
  reports are written into, and the recorded runs' directory; registered in one batch.
- **The clock** (`clock`): the wall clock, behind a trait so tests set it.
- The headless replay (`headless`): read a repository, import an observations export into a
  store in a directory the caller owns, and dump truth's replay. `repo-terrarium replay
  --dump` and `cargo xtask golden` run through it.
- Observations (`observe`): a report an adapter read, HEAD and a time become the store's
  record; and a ref rewrite's record. Only a known reading is recorded; dead code waits for the
  export count the tree-sitter reader brings.

## Why it is a crate of its own

Without UI types, the pipeline can be driven from tests and from `cargo xtask`, which is how the
golden replay and the integration tests run. It never resolves the data directory itself: its
caller passes one, so tests use their own.

## Rules

- It depends on `terrarium-truth`, `terrarium-adapters` and `terrarium-store`, and on no other
  workspace crate. It has no UI types.
- Only the refresh path calls adapters. Replay never runs a tool (I-4).
- Nothing here writes into the repository or runs anything from it (I-5).

## Requirements

N-04, N-06 and N-10; invariants I-4, I-5 and I-16.
