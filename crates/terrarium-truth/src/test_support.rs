//! Shared test settings.

use proptest::test_runner::{Config, RngSeed};

/// Property tests run from a fixed seed, so every run checks the same cases, and write no
/// regression files into the source tree.
pub fn proptest_config() -> Config {
    Config {
        rng_seed: RngSeed::Fixed(0x7465_7272_6172_6975),
        failure_persistence: None,
        ..Config::default()
    }
}
