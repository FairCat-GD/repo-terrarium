# terrarium-adapters

Reads a repository and its reports, and turns them into observations and `CommitFacts` for
truth.

## Owns

- The gix history reader and `CommitFacts`.
- Parsers for lcov, Cobertura, `JUnit` XML and Knip reports.
- Tree-sitter metrics: complexity, TODO density and lines of code.
- Discovery of what a repository has, and what it lacks.
- The GitHub Actions client.
- The single `exec` module, the only place a process starts: the command a person gives
  `record-tests`, and later a tool behind its per-adapter opt-in.

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
- `head` says where HEAD is for an observation: its commit, its branch or none when detached,
  and whether that commit is the default tip. `GitRepo::discover` finds the repository around a
  directory, for `record-tests`.

## Reports (`report`, `local_tests`)

- `Report::read(root, format, files)` reads lcov, Cobertura, `JUnit` XML or a Knip JSON report
  from files relative to the repository root, merged when there are several. It is either
  known (a reading, the top 20 contributors, a content hash and the newest mtime) or unknown
  with the reason: missing, outside the repository, not a regular file, over its size cap,
  malformed or cut short, empty, or from a Knip before 6.
- Every parser insists on the end of its document (`end_of_record` for lcov, the root's end
  tag for XML, the whole JSON value), so a file caught half-written is unknown, never a partial
  reading.
- XML goes through quick-xml with no DTD processing: a `<!DOCTYPE>` is skipped, and only the
  predefined entities and character references resolve.
- Paths a report names are made repository-relative; a path outside the repository still
  counts in the totals but never becomes a contributor, so no home directory reaches the store.
- `local_tests` reads and writes the status file `record-tests` keeps in the data directory,
  whole or not at all.
- Knip's report lists only what is unused, so dead code has no normalized value until the
  tree-sitter reader counts exports.

## Discovery (`discovery`)

`discover(root, overrides, status_file)` says what a repository has: git and its default
branch, a coverage report (lcov first, then Cobertura), a `JUnit` report, a `record-tests`
status file, a Knip report and configuration, CI workflows and test runners. It looks only at
fixed lists of paths (a `*` matches within one segment) and the configured overrides, which must
stay inside the repository; it never walks the tree, runs anything or writes.

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
- A parse failure degrades to `Unknown`, never to zero, and an unknown report is never
  recorded: the last reading holds and turns stale.
- `std::process::Command` appears only in `exec` (`cargo xtask verify`'s graph stage).

## Requirements

Invariants I-3, I-4, I-5 and I-6; budgets N-05, N-09 and N-10.
