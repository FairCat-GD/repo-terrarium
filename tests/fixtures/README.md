# Test fixtures

Shared input files for tests: report files, fixture repositories, sample observations.

- **Fictional data only.** No real names, no real repositories, no real content, and no author's
  identity in fixtures or seeds.
- **Tests read only from here, from what they generate, and from the delivered art.** The asset
  pipeline's tests read the package in `art-assets/`, which they check, but never a biome's
  `references/`. No test reads `docs/`: a test that depends on a document makes the document
  undeletable.
- **Tests never touch a real data directory.** They resolve it from `TERRARIUM_DATA_DIR`,
  pointed at a temporary directory the test owns (I-14).

- `arc/observations.jsonl`: the observations recorded alongside the scripted arc repository
  (`terrarium_adapters::fixture::arc`), in the store's export format. Written by
  `cargo xtask fixture --bless`; a test fails while it is stale. Coverage jumps when the arc
  recovers, so its replay passes through every presentation state.

- `reports/`: report files as tools write them, for the parsers' tests. `lcov/` (Jest's
  relative paths, llvm-cov's absolute ones, two tracefiles to merge, a file cut inside a
  record), `cobertura/` (istanbul with its DOCTYPE, coverage.py), `junit/` (Surefire's
  `TEST-*.xml`, jest-junit, pytest with a setup error and an xfail, Vitest, a run where nothing
  ran), `knip/` (the Knip 6 format and the refused Knip 5 one) and `local-tests/` (status files
  `record-tests` writes). A test copies a file into a repository it owns and replaces `@ROOT@`
  with that repository's path, as a tool writing absolute paths would.

Beside it, `tests/golden/` holds outputs rather than inputs: `arc.dump`, the canonical dump of
the arc's replay, and `arc.sha256`, its hash. `cargo xtask golden --bless` writes them, and
`verify`'s `golden` stage fails while they differ from a fresh replay.

A crate reaches this folder as `Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")`,
and xtask joins it to the workspace root.
