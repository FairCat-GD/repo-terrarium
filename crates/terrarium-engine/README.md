# terrarium-engine

The headless live pipeline, run on a worker thread.

## Owns

- The file watcher and its 500 ms debounce.
- Refresh orchestration: adapters, then the store, then replay.
- Incremental and full replay, cache use, and catch-up after sleep.
- The repository-not-found state.
- Panic containment: a truth panic freezes the scene and never crashes the app.
- The headless replay (`headless`): read a repository, import an observations export into a
  store in a directory the caller owns, and dump truth's replay. `repo-terrarium replay
  --dump` and `cargo xtask golden` run through it.

## Why it is a crate of its own

Without UI types, the pipeline can be driven from tests and from `cargo xtask`, which is how the
golden replay and the integration tests run.

## Rules

- It depends on `terrarium-truth`, `terrarium-adapters` and `terrarium-store`, and on no other
  workspace crate. It has no UI types.
- Only the refresh path calls adapters. Replay never runs a tool (I-4).

## Requirements

N-04, N-06 and N-10; invariants I-4 and I-16.
