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

A crate reaches this folder as `concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures")`.
