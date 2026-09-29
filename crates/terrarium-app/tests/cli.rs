//! The binary's command line, run the way a person runs it. Never without arguments: that opens
//! a window.
#![forbid(unsafe_code)]

use std::process::{Command, Output};

fn repo_terrarium(arg: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_repo-terrarium"))
        .arg(arg)
        .output()
        .expect("the binary runs")
}

#[test]
fn version_prints_the_binary_name_and_workspace_version() {
    let out = repo_terrarium("--version");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("repo-terrarium {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(out.stderr.is_empty());
}

#[test]
fn an_unexpected_argument_exits_with_status_2() {
    let out = repo_terrarium("--bogus");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8(out.stderr).unwrap().contains("'--bogus'"));
}
