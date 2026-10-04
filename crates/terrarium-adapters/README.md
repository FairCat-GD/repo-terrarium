# terrarium-adapters

Reads a repository and its reports, and turns them into observations and `CommitFacts` for
truth.

## Owns

- The gix history reader and `CommitFacts`.
- Parsers for lcov, Cobertura, `JUnit` XML and Knip reports.
- Tree-sitter metrics: complexity, TODO density and lines of code.
- Discovery of what a repository has, and what it lacks.
- The GitHub Actions client.
- The single `exec` module, the only place a tool may run, behind the per-adapter opt-in.

## Why it is a crate of its own

Everything that reads the outside world sits here, so the rest of the system sees only typed
facts, and the security boundary has one home.

## The git reader (`git`)

- `GitRepo::open` opens a repository read-only and isolated: no system, global or user
  configuration, no environment, no includes, and replace refs neutralised. Nothing it reads
  can run anything: diffs read blobs from the object database with no filters, text
  conversion, external diff, drivers or attributes.
- `default_branch` picks `origin/HEAD`'s local branch, then `main`, `master`, then `HEAD`'s
  branch. `identity` walks first parents from that tip to the root (or a shallow boundary)
  and derives the seed.
- `read_history` reads every commit reachable from a local branch into `CommitFacts`, in time
  order: clamped committer time, parents, first-parent numstat over text files (the terrain's
  exclusions, `terrarium_truth::history`), renames at 50%, directories deleted whole, tags. It
  reports progress and can be cancelled.
- `read_since` brings a history up to date: `Unchanged`, `Appended` with the earliest new tick,
  or `Rewritten` with the reason a full read was needed.
- `RepoState::state_hash` is the git part of the replay cache key.

## Fixtures (`fixture`, behind the `fixture` feature)

Deterministic repositories for tests, benches and `cargo xtask fixture`: `FixtureRepo` builds
commits, branches, merges, tags, shallow cuts and missing objects with fictional authors and
fixed times; `arc` is the scripted two-year history and `synthetic` the bench's large one.
The feature writes, so only xtask and dev-dependencies may turn it on.

## Rules

- It depends on no workspace crate except `terrarium-truth`.
- It is the only crate allowed an HTTP client: `ureq`, in the `ci` module only.
  `cargo xtask verify` checks that no other crate can reach `ureq`.
- gix is pinned exactly (`=0.88.0`): a bump can change line counts and so truth. It is opened
  read-only and isolated. The git command line is never run.
- A parse failure degrades to `Unknown`, never to zero.

## Requirements

Invariants I-3, I-4, I-5 and I-6; budgets N-05, N-09 and N-10.
