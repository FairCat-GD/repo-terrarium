//! `cargo xtask verify`: the one gate every commit must pass.
//!
//! Every stage runs even after one fails, so a deliberate break shows that it fails at its own
//! stage and nowhere else.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

use crate::graph;

/// The Windows target linted on macOS, since nothing else compiles the Windows code.
const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

enum Step {
    /// A cargo subcommand. `tool` names a third-party subcommand to check for first.
    Cargo {
        args: &'static [&'static str],
        tool: Option<Tool>,
    },
    Graph,
}

struct Tool {
    subcommand: &'static str,
    install: &'static str,
}

struct Stage {
    name: &'static str,
    step: Step,
}

const STAGES: &[Stage] = &[
    Stage {
        name: "fmt",
        step: Step::Cargo {
            args: &["fmt", "--all", "--check"],
            tool: None,
        },
    },
    Stage {
        name: "clippy",
        step: Step::Cargo {
            args: &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            tool: None,
        },
    },
    Stage {
        name: "clippy (windows)",
        step: Step::Cargo {
            args: &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--target",
                WINDOWS_TARGET,
                "--",
                "-D",
                "warnings",
            ],
            tool: None,
        },
    },
    Stage {
        name: "nextest",
        step: Step::Cargo {
            args: &["nextest", "run", "--workspace"],
            tool: Some(Tool {
                subcommand: "nextest",
                install: "cargo install --locked cargo-nextest",
            }),
        },
    },
    // nextest does not run doctests.
    Stage {
        name: "doctest",
        step: Step::Cargo {
            args: &["test", "--workspace", "--doc"],
            tool: None,
        },
    },
    Stage {
        name: "deny",
        step: Step::Cargo {
            args: &["deny", "check"],
            tool: Some(Tool {
                subcommand: "deny",
                install: "cargo install --locked cargo-deny",
            }),
        },
    },
    Stage {
        name: "graph",
        step: Step::Graph,
    },
];

pub fn run(root: &Path) -> ExitCode {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut failed = Vec::new();
    let mut summary = Vec::new();

    for stage in STAGES {
        println!("\n==> {}", stage.name);
        let started = Instant::now();
        let passed = match &stage.step {
            Step::Cargo { args, tool } => run_cargo(&cargo, root, args, tool.as_ref()),
            Step::Graph => run_graph(root),
        };
        let seconds = started.elapsed().as_secs_f64();
        let verdict = if passed { "ok" } else { "FAILED" };
        println!("--> {} {verdict} ({seconds:.1}s)", stage.name);
        summary.push(format!("  {:<18} {verdict}", stage.name));
        if !passed {
            failed.push(stage.name);
        }
    }

    println!("\nverify:\n{}", summary.join("\n"));
    if failed.is_empty() {
        println!("verify: all {} stages ok", STAGES.len());
        ExitCode::SUCCESS
    } else {
        println!("verify: FAILED at {}", failed.join(", "));
        ExitCode::FAILURE
    }
}

fn run_cargo(cargo: &OsString, root: &Path, args: &[&str], tool: Option<&Tool>) -> bool {
    if let Some(tool) = tool {
        let present = Command::new(cargo)
            .args([tool.subcommand, "--version"])
            .current_dir(root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !present {
            println!(
                "cargo {} is not installed: {}",
                tool.subcommand, tool.install
            );
            return false;
        }
    }
    match Command::new(cargo).args(args).current_dir(root).status() {
        Ok(status) => status.success(),
        Err(err) => {
            println!("cannot run cargo {}: {err}", args.join(" "));
            false
        }
    }
}

fn run_graph(root: &Path) -> bool {
    let workspace = match graph::load(root) {
        Ok(workspace) => workspace,
        Err(err) => {
            println!("{err}");
            return false;
        }
    };
    let violations = graph::check(&workspace);
    for violation in &violations {
        println!("{violation}");
    }
    if violations.is_empty() {
        println!(
            "{} members, {} resolved packages: the graph holds",
            workspace.members.len(),
            workspace.packages.len()
        );
    }
    violations.is_empty()
}
