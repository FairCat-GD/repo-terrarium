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

## Rules

- It depends on no workspace crate except `terrarium-truth`.
- It is the only crate allowed an HTTP client: `ureq`, in the `ci` module only.
  `cargo xtask verify` checks that no other crate can reach `ureq`.
- gix is opened read-only and isolated. The git command line is never run.
- A parse failure degrades to `Unknown`, never to zero.

## Requirements

Invariants I-3, I-4, I-5 and I-6; budgets N-05, N-09 and N-10.
