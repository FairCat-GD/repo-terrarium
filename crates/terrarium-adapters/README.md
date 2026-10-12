# terrarium-adapters

Reads a repository and its reports, and turns them into observations and `CommitFacts` for
truth.

## Owns

- The gix history reader and `CommitFacts`.
- Parsers for lcov, Cobertura, `JUnit` XML and Knip reports.
- The source metrics (`metrics`): complexity, TODO density, lines of code and exported items,
  parsed with tree-sitter in process.
- Discovery of what a repository has, and what it lacks.
- The GitHub Actions client (`ci`), the only code that names `ureq`: which repository
  (`Slug`, typed or from `origin`), what a branch's runs say (`verdict`), the client behind
  `Fetch` (`api.github.com` only, Mozilla's roots compiled in, no proxy, no redirect but one
  back to the API, an `ETag` per branch), when to ask next (`Backoff`), and the token in the
  OS keychain (`Keychain`, `Token`, which never shows itself).
- The repository a setting was given for (`binding`): the Knip opt-in and the GitHub
  repository typed hold for one repository at one folder.
- The single `exec` module, the only place a process starts:
  - the command a person gives `record-tests` (on Windows, a bare name is looked up with
    `PATHEXT`, so `.cmd` scripts such as `npm` run);
  - a tool the person installed, fed on its stdin with nothing inherited and a time limit:
    their ffmpeg, which writes the time-lapse's MP4. It is looked for on `PATH`, then on macOS
    in `/opt/homebrew/bin`, `/usr/local/bin` and `/opt/local/bin`, since an app opened from
    Finder or at login gets only the system's `PATH`; never inside the repository, and on
    Windows only as an `.exe`;
  - the Knip a repository installed, on the opt-in given for that repository
    (`KnipRun::permitted`), run by the person's node (found as ffmpeg is, never inside the
    repository) on the program Knip's manifest names, which `node_modules/.bin/knip` runs:
    `--reporter json --no-progress`, an environment of `PATH` alone, 3 minutes at most, stdout
    kept up to 16 MiB. A Knip before 6 is never run. Knip's own cache writes in
    `node_modules/.cache` are the one exemption from N-10.

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
- `RepoState::state_hash` is written to the replay cache's `tips_hash` column, which nothing
  reads: truth's own check of what a cached state consumed decides whether it resumes.
- `default_identity` gives the default branch and the identity together, as connect, a
  headless replay and `record-tests` each need them.
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
- A report is read as UTF-8, with or without a byte-order mark, or as UTF-16 with one.
- Paths a report names are made repository-relative; a relative one is read from the deepest
  directory holding the report under which it names an existing file, else from the root. A
  path outside the repository still counts in the totals but never becomes a contributor, so
  no home directory reaches the store.
- `local_tests` reads and writes the status file `record-tests` keeps in the data directory,
  whole or not at all.
- Knip's report lists only what is unused, so dead code's value is computed against the
  exports the source metrics count (`Measurement::dead_code`). Its unused exports are also kept
  per file, so a file the metrics do not parse (a `.vue` one, say) weighs on neither side.

## The source metrics (`metrics`)

`Metrics::measure(git, commit, stop)` reads the files of one commit's tree from the object
database, never the worktree, and parses those in the tier-one languages with tree-sitter, in
process. Nothing runs.

- **Languages** by extension: TypeScript (`.ts`, `.mts`, `.cts`), TSX, JavaScript (`.js`,
  `.mjs`, `.cjs`, `.jsx`), Python, Rust and Go. Declaration files (`.d.ts`) hold no code.
  Each language is one table of node kinds (`metrics/lang.rs`); a test checks every name
  against its grammar. A tier-two language would be one more table behind a cargo feature.
- **Exclusions:** the terrain's, as it applies them (`node_modules/`, `vendor/`, `dist/`,
  `build/`, `target/`, `third_party/`, `*.min.*`, lockfiles), binary blobs, and files over 1
  MiB, which are counted as oversized and not read.
- **Complexity:** every function-like node with a body is a function (declarations, methods,
  closures, arrows, lambdas). Each decision (`if`, `elif`, loops, `catch`/`except`, ternaries,
  non-default cases, `&&`, `||`, `??` and their assignments, comprehension clauses, match
  guards, `let … else`, a match's arms but one) counts toward the innermost function; its
  cyclomatic complexity is 1 plus them. Nesting is each function's deepest nesting of control
  structures; an `else if` chain does not deepen it. Normalized as `1 − clamp((mean − 1) / 14)`.
- **TODO density:** `TODO`, `FIXME` and `HACK`, upper case, as whole words, in comments, per
  1,000 lines of code (lines with a token that is not a comment). Normalized as
  `1 − clamp(per_kloc / 20)`.
- **Exports** (TypeScript, TSX, JavaScript): the distinct names a file's top-level `export`
  statements give, as Knip counts exports and exported types.
- **Errors:** a file with syntax errors still counts what it holds; a parse that runs past 2 s
  counts nothing, and its parser is reset. Confidence is the share of files read that parsed
  cleanly. A blob the object database lacks costs that file alone.
- **Incremental:** each file's facts are kept by blob and language while a measurement or the
  one before it used them, so a commit that changed three files costs three parses. They live
  in memory only.
- `languages_at` counts the files per language from paths alone, for discovery.

## Discovery (`discovery`)

`discover(root, git, overrides, status_file)`, given the caller's open repository and its default
branch, says what a repository has: git and its default
branch, a coverage report (lcov first, then Cobertura), a `JUnit` report, a `record-tests`
status file, a Knip report and configuration, CI workflows and test runners, and the source
files at the default tip per language. It looks only at fixed lists of paths (a `*` matches
within one segment) and the configured overrides, which must stay inside the repository; it
never walks the worktree, runs anything or writes. The source files are counted from the
committed tree, by path.

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
- tree-sitter (0.26.x) and the grammars change only future observations, never replay; a change
  to how the same commit is counted bumps `METRICS_VERSION`.
- A parse failure degrades to `Unknown`, never to zero, and an unknown report is never
  recorded: the last reading holds and turns stale.
- `std::process::Command` appears only in `exec` among the `src/` files of the code that ships
  (`cargo xtask verify`'s graph stage); integration tests, benches and xtask may start
  processes.

## Requirements

Invariants I-3, I-4, I-5 and I-6; budgets N-05, N-09 and N-10.
