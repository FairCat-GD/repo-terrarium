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
