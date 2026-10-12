//! The binary's command line, run the way a person runs it. Never without arguments, never with
//! a whole `--probe` command, and never with `--repo` naming a repository: those open the strip.
#![forbid(unsafe_code)]

use std::path::Path;
use std::process::{Command, Output};

fn repo_terrarium(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_repo-terrarium"))
        .args(args)
        .output()
        .expect("the binary runs")
}

/// The binary run in `dir`, with its data directory at `data`.
fn repo_terrarium_in(dir: &Path, data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_repo-terrarium"))
        .args(args)
        .current_dir(dir)
        .env("TERRARIUM_DATA_DIR", data)
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

#[test]
fn repo_naming_no_repository_exits_before_the_strip_opens() {
    let dir = tempfile::tempdir().unwrap();
    let nowhere = dir.path().join("nowhere");
    std::fs::create_dir_all(&nowhere).unwrap();
    let out = repo_terrarium_in(
        dir.path(),
        &dir.path().join("data"),
        &["--repo", nowhere.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.starts_with("repo-terrarium: ")
            && stderr.contains("could not be connected as a git repository"),
        "{stderr}"
    );
    assert_eq!(
        repo_terrarium(&["--repo"]).status.code(),
        Some(2),
        "a missing path is a usage error"
    );
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
        assert!(text.starts_with(&format!(
            "sim_version = {}\n",
            terrarium_truth::SIM_VERSION.0
        )));
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

    #[test]
    fn replay_writes_the_whole_history_as_a_time_lapse_gif_and_nothing_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        small_repo(&repo);
        let gif = dir.path().join("fern.gif");
        let at = (FIRST + 30 * DAY).to_string();
        let run = |extra: &[&str]| {
            let mut args = vec!["replay", repo.to_str().unwrap(), "--at", &at];
            args.extend_from_slice(extra);
            repo_terrarium(&args)
        };
        let out = run(&["--gif", gif.to_str().unwrap(), "--scale", "2", "--night"]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
        let bytes = std::fs::read(&gif).unwrap();
        let mut decoder = gif::DecodeOptions::new()
            .read_info(bytes.as_slice())
            .unwrap();
        assert_eq!((decoder.width(), decoder.height()), (1280, 148));
        assert_eq!(decoder.repeat(), gif::Repeat::Infinite);
        let mut delays = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            delays.push(frame.delay);
        }
        assert_eq!(delays.len(), 450);
        assert_eq!(delays[..4], [7, 7, 6, 7]);
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 2, "{entries:?}");
        // The same command writes the same bytes.
        run(&["--gif", gif.to_str().unwrap(), "--scale", "2", "--night"]);
        assert_eq!(std::fs::read(&gif).unwrap(), bytes);
        // Before the first day ends there is nothing to play.
        let early = (FIRST + 3600).to_string();
        let out = repo_terrarium(&[
            "replay",
            repo.to_str().unwrap(),
            "--at",
            &early,
            "--gif",
            gif.to_str().unwrap(),
        ]);
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8(out.stderr).unwrap().contains("no day"));
        assert_eq!(std::fs::read(&gif).unwrap(), bytes);
    }

    /// A stand-in ffmpeg in `dir`: it notes its arguments and how many bytes its stdin held,
    /// and writes a few bytes to the file its last argument names.
    #[cfg(unix)]
    fn fake_ffmpeg(dir: &Path) {
        use std::os::unix::fs::PermissionsExt as _;
        let script = "#!/bin/sh\n\
            here=$(/usr/bin/dirname \"$0\")\n\
            for arg; do last=$arg; printf '%s\\n' \"$arg\" >> \"$here/args\"; done\n\
            /usr/bin/wc -c > \"$here/count\"\n\
            printf video > \"${last#file:}\"\n";
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join("ffmpeg");
        std::fs::write(&file, script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn replay_feeds_the_time_lapse_to_ffmpeg_as_raw_rgb_and_names_the_mp4_once_whole() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        small_repo(&repo);
        let tools = dir.path().join("tools");
        fake_ffmpeg(&tools);
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let mp4 = out_dir.join("fern.mp4");
        let at = (FIRST + 30 * DAY).to_string();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_repo-terrarium"))
            .args([
                "replay",
                repo.to_str().unwrap(),
                "--at",
                &at,
                "--mp4",
                mp4.to_str().unwrap(),
                "--scale",
                "2",
            ])
            .env("PATH", &tools)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
        let args = std::fs::read_to_string(tools.join("args")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        let after = |flag: &str| args[args.iter().position(|arg| *arg == flag).unwrap() + 1];
        assert_eq!(after("-video_size"), "2560x296", "2× asked, 4× written");
        assert_eq!(after("-pix_fmt"), "rgb24");
        assert_eq!(after("-i"), "pipe:0");
        assert_eq!(
            args.last().copied(),
            Some(format!("file:{}.part", mp4.display()).as_str())
        );
        let count: usize = std::fs::read_to_string(tools.join("count"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(count, 450 * 2560 * 296 * 3, "every frame, whole");
        assert_eq!(std::fs::read(&mp4).unwrap(), b"video");
        let entries: Vec<_> = std::fs::read_dir(&out_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["fern.mp4"], "nothing beside it");
    }
}

mod discover {
    use std::fs;

    use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
    use terrarium_engine::{EngineConfig, Live};
    use terrarium_store::identifiers::SETTING_COVERAGE_REPORT_PATH;
    use terrarium_store::{DataDir, Store};

    use super::repo_terrarium;

    #[test]
    fn discover_reports_what_a_repository_has_and_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        let mut repo = FixtureRepo::init(&path).unwrap();
        repo.commit(
            "main",
            Stamp::at(1_704_704_400),
            "seed",
            &[Edit::write("src/fern.rs", "fn fern() {}\n")],
        )
        .unwrap();
        fs::create_dir_all(path.join("coverage")).unwrap();
        fs::write(
            path.join("coverage/lcov.info"),
            "SF:src/fern.rs\nDA:1,1\nDA:2,0\nend_of_record\n",
        )
        .unwrap();
        fs::write(path.join("Cargo.toml"), "[package]\nname = \"fern\"\n").unwrap();
        let out = super::repo_terrarium_in(
            dir.path(),
            &dir.path().join("data"),
            &["discover", path.to_str().unwrap()],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(
            text.starts_with("Found\n  git               default branch main\n"),
            "{text}"
        );
        assert!(
            text.contains("  coverage report   coverage/lcov.info, lcov: 1 of 2 lines covered\n"),
            "{text}"
        );
        assert!(
            text.contains("  test runner       Cargo (Cargo.toml)\n"),
            "{text}"
        );
        assert!(text.contains("To connect\n"), "{text}");
        assert!(
            text.contains("stdout"),
            "the Knip hint says where Knip writes"
        );
        assert!(
            text.ends_with(
                "\nLight, soil and weather are recorded on this machine only. On a new machine \
                 or after a reinstall, the git history replays, and they read as unknown before \
                 that day.\n"
            ),
            "{text}"
        );
        assert!(!text.contains(dir.path().to_str().unwrap()), "{text}");
        assert!(out.stderr.is_empty());

        let not_a_repo = repo_terrarium(&["discover", dir.path().to_str().unwrap()]);
        assert_eq!(not_a_repo.status.code(), Some(1));
        assert!(not_a_repo.stdout.is_empty());
        let missing = repo_terrarium(&["discover"]);
        assert_eq!(missing.status.code(), Some(2));
    }

    fn repo_with_report(path: &std::path::Path) {
        let mut repo = FixtureRepo::init(path).unwrap();
        repo.commit(
            "main",
            Stamp::at(1_704_704_400),
            "seed",
            &[Edit::write(
                &format!("src/{}.rs", path.file_name().unwrap().to_str().unwrap()),
                "fn fern() {}\n",
            )],
        )
        .unwrap();
        fs::create_dir_all(path.join("out")).unwrap();
        fs::write(
            path.join("out/cov.info"),
            "SF:a.rs\nDA:1,1\nend_of_record\n",
        )
        .unwrap();
    }

    #[test]
    fn discover_reads_the_connected_repository_s_report_paths_and_makes_no_database() {
        let dir = tempfile::tempdir().unwrap();
        let (moss, fern) = (dir.path().join("moss"), dir.path().join("fern"));
        repo_with_report(&moss);
        repo_with_report(&fern);
        let data = dir.path().join("data");
        let discover = |repo: &std::path::Path| {
            let out =
                super::repo_terrarium_in(dir.path(), &data, &["discover", repo.to_str().unwrap()]);
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap()
        };
        assert!(!discover(&moss).contains("out/cov.info"));
        assert!(
            !data.join("terrarium.sqlite3").exists(),
            "discover made a database"
        );
        // Connected, with the report's path in its settings.
        let config = EngineConfig::new(DataDir::at(&data), Some(moss.clone()));
        drop(Live::connect(&config).unwrap().expect("connected"));
        Store::open(&DataDir::at(&data))
            .unwrap()
            .set_setting(SETTING_COVERAGE_REPORT_PATH, "out/cov.info")
            .unwrap();
        let text = discover(&moss);
        assert!(text.contains("  coverage report   out/cov.info"), "{text}");
        // Another repository is not the connected one: its usual places are read.
        assert!(!discover(&fern).contains("out/cov.info"));
    }
}

mod record_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
    use terrarium_adapters::git::GitRepo;
    use terrarium_store::DataDir;

    use super::repo_terrarium_in;

    const BINARY: &str = env!("CARGO_BIN_EXE_repo-terrarium");

    fn repo(path: &Path) -> PathBuf {
        let mut repo = FixtureRepo::init(path).unwrap();
        repo.commit(
            "main",
            Stamp::at(1_704_704_400),
            "seed",
            &[Edit::write("src/fern.rs", "fn fern() {}\n")],
        )
        .unwrap();
        let git = GitRepo::open(path).unwrap();
        let seed = git.identity("main").unwrap().seed;
        DataDir::at(path.parent().unwrap().join("data")).local_tests(&seed)
    }

    fn status(path: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn record_tests_runs_the_command_and_records_how_it_ended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        let status_file = repo(&path);
        let data = dir.path().join("data");
        // The binary itself is the test command: it runs everywhere this test does.
        let passed = repo_terrarium_in(&path, &data, &["record-tests", "--", BINARY, "--version"]);
        assert_eq!(passed.status.code(), Some(0));
        assert_eq!(
            String::from_utf8(passed.stdout).unwrap(),
            format!("repo-terrarium {}\n", env!("CARGO_PKG_VERSION")),
            "the command's own output passes through"
        );
        assert!(passed.stderr.is_empty());
        let recorded = status(&status_file);
        assert_eq!(recorded["format"], "repo-terrarium-local-tests");
        assert_eq!(recorded["exit_code"], 0);

        // From a folder inside the repository, a failing run, its exit code kept.
        let nested = path.join("src");
        fs::create_dir_all(&nested).unwrap();
        let failed = repo_terrarium_in(&nested, &data, &["record-tests", "--", BINARY, "--bogus"]);
        assert_eq!(failed.status.code(), Some(2));
        assert!(
            String::from_utf8(failed.stderr)
                .unwrap()
                .contains("'--bogus'")
        );
        assert_eq!(status(&status_file)["exit_code"], 2);
        assert!(
            !path.join("local-tests").exists(),
            "nothing is written into the repository"
        );
    }

    /// Ctrl-C reaches the wrapper and the command alike. Here only the wrapper gets the
    /// interrupt, so a wrapper that keeps waiting is plainly seen: it ends when the command ends,
    /// with its code, and records the run.
    #[cfg(unix)]
    #[test]
    fn record_tests_waits_for_its_command_through_an_interrupt() {
        use std::process::Command;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        let status_file = repo(&path);
        // The command marks that it started, so the interrupt reaches the wrapper only once it
        // is waiting (a freshly built binary can take a while to start).
        let started = dir.path().join("started");
        let script = format!("touch '{}'; sleep 2; exit 4", started.display());
        let mut wrapper = Command::new(BINARY)
            .args(["record-tests", "--", "sh", "-c", &script])
            .current_dir(&path)
            .env("TERRARIUM_DATA_DIR", dir.path().join("data"))
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if started.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(started.exists(), "the command started");
        let sent = Command::new("kill")
            .args(["-INT", &wrapper.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
        let ended = wrapper.wait().unwrap();
        assert_eq!(ended.code(), Some(4), "{ended:?}");
        assert_eq!(status(&status_file)["exit_code"], 4);
    }

    #[test]
    fn record_tests_runs_nothing_it_cannot_record_and_reports_a_missing_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        let status_file = repo(&path);
        let data = dir.path().join("data");
        let missing = repo_terrarium_in(
            &path,
            &data,
            &["record-tests", "--", "repo-terrarium-test-no-such-command"],
        );
        assert_eq!(missing.status.code(), Some(127));
        assert!(
            String::from_utf8(missing.stderr)
                .unwrap()
                .contains("command not found")
        );
        assert!(!status_file.exists());

        // Outside any repository nothing runs: the command would have printed its version.
        let outside = dir.path().join("elsewhere");
        fs::create_dir_all(&outside).unwrap();
        let refused = repo_terrarium_in(
            &outside,
            &data,
            &["record-tests", "--", BINARY, "--version"],
        );
        assert_eq!(refused.status.code(), Some(125));
        assert!(refused.stdout.is_empty());
        assert!(
            String::from_utf8(refused.stderr)
                .unwrap()
                .contains("nothing ran")
        );

        let usage = repo_terrarium_in(&path, &data, &["record-tests", BINARY]);
        assert_eq!(usage.status.code(), Some(2));
        assert!(usage.stdout.is_empty());
        assert!(!status_file.exists());
    }
}
