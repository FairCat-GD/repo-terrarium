//! What the store's tests share: a database in a temp dir of their own (I-14), and fictional
//! terrariums and observations.

use proptest::test_runner::{Config, RngSeed};
use tempfile::TempDir;
use terrarium_truth::domain::{ObservationHead, WeatherSource};
use terrarium_truth::{CommitId, ObservationKind, Q16, RepoSeed, SIM_VERSION, Tick};

use crate::{
    Contributor, DataDir, NewTerrarium, ObservationId, ObservationRecord, RepoLocation, SimCache,
    Store,
};

/// A store in a fresh temp dir. The store is declared first so it closes before the directory
/// is removed.
pub struct Fixture {
    pub store: Store,
    pub dir: TempDir,
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = tempfile::Builder::new()
            .prefix("terrarium-store-test-")
            .tempdir()
            .expect("the temp dir is writable");
        let store = Store::open(&DataDir::at(dir.path())).expect("the store opens");
        Fixture { store, dir }
    }

    pub fn data_dir(&self) -> DataDir {
        DataDir::at(self.dir.path())
    }
}

/// Property tests run from a fixed seed, so every run checks the same cases, and write no
/// regression files into the source tree.
pub fn proptest_config() -> Config {
    Config {
        rng_seed: RngSeed::Fixed(0x7465_7272_6172_6975),
        failure_persistence: None,
        ..Config::default()
    }
}

pub fn seed(n: u8) -> RepoSeed {
    RepoSeed::from_bytes([n; 32])
}

pub fn commit(digit: char) -> CommitId {
    CommitId::from_hex(&digit.to_string().repeat(40)).expect("a valid id")
}

/// A cache row whose state is a few arbitrary bytes: the store never decodes it.
pub fn sim_cache(high_water: Option<i64>, tick: u32) -> SimCache {
    SimCache {
        sim_version: SIM_VERSION,
        tips_hash: [9; 32],
        obs_high_water: high_water.map(ObservationId::new),
        tick: Tick::new(tick),
        state: vec![0, 1, 2, 255],
    }
}

pub fn location(n: u8) -> RepoLocation {
    RepoLocation {
        path: format!("/projects/meadow-{n}"),
        name: format!("meadow-{n}"),
        default_branch: "main".to_owned(),
    }
}

pub fn new_terrarium(n: u8) -> NewTerrarium {
    NewTerrarium {
        seed: seed(n),
        repo: location(n),
        install_salt: [n; 16],
        connected_at: 1_700_000_000,
    }
}

/// A `raw` of `kind`'s shape.
pub fn raw_for(kind: ObservationKind) -> String {
    match kind {
        ObservationKind::DeadCode => r#"{"unused_items":4,"exported_items":80}"#.to_owned(),
        ObservationKind::Weather => r#"{"state":"pass"}"#.to_owned(),
        ObservationKind::RefRewrite => format!(
            r#"{{"ref":"refs/heads/main","old":"{}","new":"{}"}}"#,
            "b".repeat(40),
            "c".repeat(40)
        ),
        _ => r#"{"format":"lcov","lines_found":120,"lines_hit":90}"#.to_owned(),
    }
}

/// A valid observation of `kind`; weather comes from CI on `main`.
pub fn record(kind: ObservationKind, recorded_at: i64) -> ObservationRecord {
    ObservationRecord {
        kind,
        adapter_version: 1,
        recorded_at,
        head: ObservationHead {
            commit: commit('a'),
            branch: Some("main".to_owned()),
            at_default_tip: true,
        },
        source: (kind == ObservationKind::Weather).then(|| WeatherSource::Ci {
            branch: "main".to_owned(),
        }),
        report_mtime: Some(recorded_at - 10),
        content_hash: [0x5a; 32],
        raw: raw_for(kind),
        normalized: (kind != ObservationKind::RefRewrite).then(|| Q16::from_ratio(3, 4)),
        confidence: Q16::ONE,
        contributors: vec![
            Contributor {
                path: "src/meadow.rs".to_owned(),
                value: 12,
            },
            Contributor {
                path: "src/pond/frog.rs".to_owned(),
                value: 3,
            },
        ],
    }
}

pub fn weather(recorded_at: i64) -> ObservationRecord {
    record(ObservationKind::Weather, recorded_at)
}
