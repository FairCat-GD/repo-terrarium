# Test fixtures

Shared input files for tests: report files, fixture repositories, sample observations.

- **Fictional data only.** No real names, no real repositories, no real content, and no author's
  identity in fixtures or seeds.
- **Tests read only from here and from what they generate.** They never read `docs/` or
  `art-assets/biome-temperate/references/`; a test that depends on a document makes the
  document undeletable.
- **Tests never touch a real data directory.** They resolve it from `TERRARIUM_DATA_DIR`,
  pointed at a temporary directory the test owns (I-14).

A crate reaches this folder as `concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures")`.
