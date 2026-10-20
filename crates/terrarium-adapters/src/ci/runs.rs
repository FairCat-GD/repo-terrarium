//! A branch's workflow runs, as GitHub lists them, and the verdict they give.

use std::collections::BTreeMap;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::report::MAX_CONTRIBUTOR_PATH;

/// The most failing workflows kept, as for every reading's contributors.
const MAX_FAILING: usize = 20;

/// The part of `GET /repos/{owner}/{repo}/actions/runs` read here; other keys are ignored.
#[derive(Clone, Debug, Deserialize)]
pub struct Runs {
    pub workflow_runs: Vec<Run>,
}

/// One workflow run.
#[derive(Clone, Debug, Deserialize)]
pub struct Run {
    pub id: u64,
    pub workflow_id: u64,
    pub head_sha: String,
    /// `queued`, `in_progress`, `completed`, …
    pub status: Option<String>,
    /// Set once completed: `success`, `failure`, `cancelled`, …
    pub conclusion: Option<String>,
    #[serde(default)]
    pub run_attempt: Option<u64>,
    /// The workflow's file, `.github/workflows/ci.yml`, sometimes with `@<ref>` after it.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// What a commit's runs say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub passed: bool,
    /// The commit the runs judged.
    pub sha: String,
    /// How many workflows ran on it.
    pub workflows: u64,
    /// The failing workflows' files, at most 20.
    pub failing: Vec<String>,
    /// SHA-256 of what decided it: the commit and each workflow's run, attempt and conclusion.
    pub hash: [u8; 32],
}

/// How one workflow's latest attempt on a commit came out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    /// Ended with nothing to say: cancelled, skipped, neutral, stale, waiting on a person.
    Silent,
    Running,
}

fn outcome(run: &Run) -> Outcome {
    if run.status.as_deref() != Some("completed") {
        return Outcome::Running;
    }
    match run.conclusion.as_deref() {
        Some("success") => Outcome::Passed,
        Some("failure" | "timed_out" | "startup_failure") => Outcome::Failed,
        _ => Outcome::Silent,
    }
}

/// The verdict of the newest commit whose runs decided one, newest first as GitHub lists them:
/// a failing workflow fails it, and every workflow ended with at least one passing passes it.
/// A commit still running, or whose runs all ended silent, is passed over for the one before.
/// `None` when no commit listed is decided: the branch has no run to go by.
pub fn verdict(runs: &[Run]) -> Option<Verdict> {
    let mut shas: Vec<&str> = Vec::new();
    for run in runs {
        if !shas.contains(&run.head_sha.as_str()) {
            shas.push(&run.head_sha);
        }
    }
    shas.into_iter().find_map(|sha| decide(sha, runs))
}

fn decide(sha: &str, runs: &[Run]) -> Option<Verdict> {
    // Each workflow's latest attempt: a re-run replaces the run before it.
    let mut latest: BTreeMap<u64, &Run> = BTreeMap::new();
    for run in runs.iter().filter(|run| run.head_sha == sha) {
        let key = |run: &Run| (run.run_attempt.unwrap_or(1), run.id);
        if latest
            .get(&run.workflow_id)
            .is_none_or(|known| key(known) < key(run))
        {
            latest.insert(run.workflow_id, run);
        }
    }
    let outcomes: Vec<Outcome> = latest.values().map(|run| outcome(run)).collect();
    let failed = outcomes.contains(&Outcome::Failed);
    let passed =
        !failed && !outcomes.contains(&Outcome::Running) && outcomes.contains(&Outcome::Passed);
    if !(failed || passed) {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(sha.as_bytes());
    for (workflow, run) in &latest {
        hasher.update([0]);
        hasher.update(workflow.to_le_bytes());
        hasher.update(run.id.to_le_bytes());
        hasher.update(run.run_attempt.unwrap_or(1).to_le_bytes());
        hasher.update(run.conclusion.as_deref().unwrap_or("").as_bytes());
    }
    let mut failing: Vec<String> = latest
        .values()
        .filter(|run| outcome(run) == Outcome::Failed)
        .map(|run| workflow_file(run))
        .collect();
    failing.sort();
    failing.dedup();
    failing.truncate(MAX_FAILING);
    Some(Verdict {
        passed,
        sha: sha.to_owned(),
        workflows: u64::try_from(latest.len()).unwrap_or(u64::MAX),
        failing,
        hash: hasher.finalize().into(),
    })
}

/// A run's workflow file without the `@<ref>` GitHub may add, else its name, else its id; cut
/// to the longest a contributor's path may be, since a workflow's name has no bound.
fn workflow_file(run: &Run) -> String {
    let file = match (&run.path, &run.name) {
        (Some(path), _) if !path.is_empty() => {
            path.split_once('@').map_or(path.as_str(), |(file, _)| file)
        }
        (_, Some(name)) => name.as_str(),
        _ => return run.workflow_id.to_string(),
    };
    let mut end = file.len().min(MAX_CONTRIBUTOR_PATH);
    while !file.is_char_boundary(end) {
        end -= 1;
    }
    file.get(..end).unwrap_or_default().to_owned()
}
