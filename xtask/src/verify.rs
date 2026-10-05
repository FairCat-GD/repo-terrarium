//! `cargo xtask verify`: the one gate every commit must pass.
//!
//! Every stage runs even after one fails, so a deliberate break shows that it fails at its own
//! stage and nowhere else.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

use crate::{assets, golden, graph, sweep, windows};

enum Step {
    /// A cargo subcommand. `tool` names a third-party subcommand to check for first; `env` is
    /// set for this command only.
    Cargo {
        args: &'static [&'static str],
        tool: Option<Tool>,
        env: &'static [(&'static str, &'static str)],
    },
    Graph,
    /// `cargo xtask assets --check`, in process.
    Assets,
    /// `cargo xtask golden --check`, in process.
    Golden,
}

/// A third-party cargo subcommand, pinned to the exact version its configuration is written
/// for: `deny.toml` follows cargo-deny 0.20's schema, and a newer tool could read it
/// differently.
struct Tool {
    subcommand: &'static str,
    package: &'static str,
    version: &'static str,
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
            env: &[],
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
            env: &[],
        },
    },
    // The dev build: the debug window and its egui and wgpu dependencies.
    Stage {
        name: "clippy (dev)",
        step: Step::Cargo {
            args: &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
            tool: None,
            env: &[],
        },
    },
    Stage {
        name: "clippy (windows)",
        step: Step::Cargo {
            args: &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--target",
                windows::TARGET,
                "--",
                "-D",
                "warnings",
            ],
            tool: None,
            // The Windows target, linted on macOS since nothing else compiles its code here. A
            // lint never links, so the target's C is skipped (`windows.rs`).
            env: windows::LINT_ENV,
        },
    },
    Stage {
        name: "nextest",
        step: Step::Cargo {
            args: &["nextest", "run", "--workspace", "--all-features"],
            tool: Some(Tool {
                subcommand: "nextest",
                package: "cargo-nextest",
                version: "0.9.146",
            }),
            env: &[],
        },
    },
    // The release build's own features: what only a build without `dev` does, such as refusing
    // `--probe egui`. Only the app has features, so only its tests run again.
    Stage {
        name: "nextest (release)",
        step: Step::Cargo {
            args: &["nextest", "run", "--package", "terrarium-app"],
            tool: Some(Tool {
                subcommand: "nextest",
                package: "cargo-nextest",
                version: "0.9.146",
            }),
            env: &[],
        },
    },
    // nextest does not run doctests.
    Stage {
        name: "doctest",
        step: Step::Cargo {
            args: &["test", "--workspace", "--doc", "--all-features"],
            tool: None,
            env: &[],
        },
    },
    Stage {
        name: "deny",
        step: Step::Cargo {
            args: &["deny", "check"],
            tool: Some(Tool {
                subcommand: "deny",
                package: "cargo-deny",
                version: "0.20.2",
            }),
            env: &[],
        },
    },
    Stage {
        name: "graph",
        step: Step::Graph,
    },
    Stage {
        name: "assets",
        step: Step::Assets,
    },
    Stage {
        name: "golden",
        step: Step::Golden,
    },
];

pub fn run(root: &Path) -> ExitCode {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut failed = Vec::new();
    let mut summary = Vec::new();
    sweep::before_verify(root, &cargo);

    for stage in STAGES {
        println!("\n==> {}", stage.name);
        let started = Instant::now();
        let passed = match &stage.step {
            Step::Cargo { args, tool, env } => run_cargo(&cargo, root, args, tool.as_ref(), env),
            Step::Graph => run_graph(root),
            Step::Assets => run_assets(root),
            Step::Golden => run_golden(root),
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

fn run_cargo(
    cargo: &OsString,
    root: &Path,
    args: &[&str],
    tool: Option<&Tool>,
    env: &[(&str, &str)],
) -> bool {
    if let Some(tool) = tool
        && !has_pinned_version(cargo, root, tool)
    {
        println!(
            "{} {} is required: cargo install --locked {}@{}",
            tool.package, tool.version, tool.package, tool.version
        );
        return false;
    }
    let status = Command::new(cargo)
        .args(args)
        .envs(env.iter().copied())
        .current_dir(root)
        .status();
    match status {
        Ok(status) => status.success(),
        Err(err) => {
            println!("cannot run cargo {}: {err}", args.join(" "));
            false
        }
    }
}

/// `cargo nextest --version` prints `cargo-nextest 0.9.146 (…)`; `cargo deny --version` prints
/// `cargo-deny 0.20.2`.
fn has_pinned_version(cargo: &OsString, root: &Path, tool: &Tool) -> bool {
    let Ok(out) = Command::new(cargo)
        .args([tool.subcommand, "--version"])
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let printed = String::from_utf8_lossy(&out.stdout);
    let words: Vec<&str> = printed.split_whitespace().collect();
    out.status.success() && words.windows(2).any(|w| w == [tool.package, tool.version])
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

/// A panic in the asset pipeline fails its stage; the stages after it and the summary still
/// run.
fn run_assets(root: &Path) -> bool {
    std::panic::catch_unwind(|| assets::check(root)).unwrap_or_else(|_| {
        println!("assets: the pipeline panicked");
        false
    })
}

/// A panic in the golden replay fails its stage; the summary still runs.
fn run_golden(root: &Path) -> bool {
    std::panic::catch_unwind(|| golden::check(root)).unwrap_or_else(|_| {
        println!("golden: the replay panicked");
        false
    })
}
