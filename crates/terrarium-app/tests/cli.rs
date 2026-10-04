//! The binary's command line, run the way a person runs it. Never without arguments, and never
//! with a whole `--probe` command: those open the strip.
#![forbid(unsafe_code)]

use std::process::{Command, Output};

fn repo_terrarium(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_repo-terrarium"))
        .args(args)
        .output()
        .expect("the binary runs")
}

#[test]
fn version_prints_the_binary_name_and_workspace_version() {
    let out = repo_terrarium(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("repo-terrarium {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(out.stderr.is_empty());
}

#[test]
fn an_unexpected_argument_exits_with_status_2() {
    let out = repo_terrarium(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8(out.stderr).unwrap().contains("'--bogus'"));
}

#[test]
fn a_probe_without_a_kind_exits_with_status_2() {
    let out = repo_terrarium(&["--probe"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("--probe pinning")
    );
}

#[test]
fn an_unknown_probe_kind_exits_with_status_2() {
    let out = repo_terrarium(&["--probe", "weather"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8(out.stderr).unwrap().contains("'weather'"));
}

#[test]
fn the_probe_readme_is_printed_whole_and_opens_no_window() {
    let out = repo_terrarium(&["--probe-readme"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("Repo Terrarium: the Windows pinning probe\n"));
    assert!(text.contains("run-probe.cmd"));
    assert!(out.stderr.is_empty());
}

mod replay {
    use std::path::Path;

    use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
    use terrarium_engine::headless::{Request, replay_dump};
    use terrarium_store::DataDir;
    use terrarium_truth::{ReplayDump, from_canonical_dump};

    use super::repo_terrarium;

    /// Monday 2024-01-08, 09:00 UTC.
    const FIRST: i64 = 1_704_704_400;
    const DAY: i64 = 86_400;

    fn small_repo(path: &Path) {
        let mut repo = FixtureRepo::init(path).unwrap();
        for day in 0..20 {
            let text = format!("fn fern_{day}() {{}}\n");
            repo.commit(
                "main",
                Stamp::at(FIRST + day * DAY),
                "tend",
                &[Edit::write(&format!("src/fern_{day}.rs"), &text)],
            )
            .unwrap();
        }
    }

    #[test]
    fn replay_prints_the_dump_of_the_headless_replay() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        small_repo(&repo);
        let at = FIRST + 30 * DAY;
        let out = repo_terrarium(&[
            "replay",
            repo.to_str().unwrap(),
            "--dump",
            "--at",
            &at.to_string(),
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty());
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.starts_with("sim_version = 1\n"));
        let printed: ReplayDump = from_canonical_dump(&text).unwrap();
        let expected = replay_dump(Request {
            repo: &repo,
            data: &DataDir::at(dir.path().join("data")),
            observations: None,
            at_secs: at,
            install_salt: [0; 16],
        })
        .unwrap();
        assert_eq!(printed, expected);
        assert_eq!(printed.habitat.terrain, 20);
        assert!(!text.contains(repo.to_str().unwrap()));
    }

    #[test]
    fn replay_explains_what_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().to_str().unwrap();
        let out = repo_terrarium(&["replay", empty, "--dump"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        let message = String::from_utf8(out.stderr).unwrap();
        assert!(
            message.contains("could not be read as a git repository"),
            "{message}"
        );
        let repo = dir.path().join("repo");
        small_repo(&repo);
        let out = repo_terrarium(&["replay", repo.to_str().unwrap(), "--dump", "--at", "0"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(
            String::from_utf8(out.stderr)
                .unwrap()
                .contains("outside the history")
        );
    }

    #[test]
    fn replay_without_dump_exits_with_status_2() {
        let out = repo_terrarium(&["replay", "moss"]);
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8(out.stderr).unwrap().contains("--dump"));
    }
}
