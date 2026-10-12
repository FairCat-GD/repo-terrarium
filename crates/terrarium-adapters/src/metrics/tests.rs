//! The metrics on fictional sources, one per language, each counted by hand.

use std::fs;
use std::path::Path;

use terrarium_truth::{CommitId, Q16};

use super::count::words;
use super::*;
use crate::fixture::{Edit, FixtureRepo, Stamp};

fn facts(language: Language, source: &str) -> FileFacts {
    Metrics::new()
        .parse(language, source.as_bytes(), &mut || false)
        .expect("not cancelled")
}

fn clean(language: Language, source: &str) -> FileFacts {
    let facts = facts(language, source);
    assert_eq!(
        facts.read,
        Read::Clean,
        "{language:?} parses without errors"
    );
    facts
}

const TYPESCRIPT: &str = r"// TODO: tidy the greenhouse
export function water(plants: Plant[], dry: boolean): number {
  let poured = 0;
  for (const plant of plants) {
    if (plant.thirsty && !dry) {
      poured += plant.need ?? 1;
    } else if (plant.wilting) {
      poured += 2;
    }
  }
  return poured;
}

export const prune = (stem: number) => (stem > 3 ? stem - 1 : stem);

class Shed {
  open(key?: string): boolean {
    try {
      return key !== undefined;
    } catch (err) {
      return false;
    }
  }
}
";

#[test]
fn typescript_counts_functions_decisions_nesting_markers_and_lines() {
    let facts = clean(Language::TypeScript, TYPESCRIPT);
    // water: for-of, if, &&, ??, else if (5); prune: ?: (1); open: catch (1).
    assert_eq!(
        facts.counted,
        Counted {
            lines: 21,
            functions: 3,
            decision_points: 7,
            max_cyclomatic: 6,
            // water: for, then if; the else if continues the chain.
            max_nesting: 2,
            nesting_total: 2 + 1,
            todo: 1,
            fixme: 0,
            hack: 0,
        }
    );
    assert_eq!(facts.exports, 2, "water and prune; Shed is not exported");
}

const TSX: &str = r#"import { useState } from "react";

export const Sprout = ({ tall }: { tall: boolean }) => {
  const [leaves] = useState(tall ? 3 : 1);
  if (!leaves) {
    return null; // HACK: nothing to draw
  }
  return <div className={tall ? "tall" : "short"}>{leaves}</div>;
};
"#;

#[test]
fn tsx_counts_inside_its_markup() {
    let facts = clean(Language::Tsx, TSX);
    assert_eq!(
        facts.counted,
        Counted {
            lines: 8,
            functions: 1,
            decision_points: 3,
            max_cyclomatic: 4,
            max_nesting: 1,
            nesting_total: 1,
            todo: 0,
            fixme: 0,
            hack: 1,
        }
    );
    assert_eq!(facts.exports, 1);
}

const JAVASCRIPT: &str = r#"/* FIXME later; a todo in lower case and TODOS are not markers */
export default function Bed({ rows }) {
  return rows.length > 0 ? <ul>{rows.map((row) => <li key={row}>{row}</li>)}</ul> : null;
}

export async function* sow(seeds) {
  for await (const seed of seeds) {
    yield seed || "bare";
  }
}

function settle(pot) {
  pot.soil ??= "loam";
  do {
    pot.water -= 1;
  } while (pot.water > 3);
  switch (pot.kind) {
    case "clay":
      return 1;
    default:
      return 0;
  }
}

let count = 0;
count ||= 1;
"#;

#[test]
fn javascript_counts_generators_logical_assignment_and_cases() {
    let facts = clean(Language::JavaScript, JAVASCRIPT);
    // Bed: ?: (1); its arrow: none; sow: for-await, || (2); settle: ??=, do, one case (3).
    // `count ||= 1` stands outside any function.
    assert_eq!(
        facts.counted,
        Counted {
            lines: 22,
            functions: 4,
            decision_points: 6,
            max_cyclomatic: 4,
            max_nesting: 1,
            nesting_total: 2,
            todo: 0,
            fixme: 1,
            hack: 0,
        }
    );
    assert_eq!(facts.exports, 2, "the default and sow");
}

const PYTHON: &str = r#"# FIXME: the pond leaks
def feed(fish, hungry):
    if hungry and fish:
        return [f for f in fish if f.awake]
    elif not fish:
        return []
    while hungry:
        hungry = False
    return None


class Pond:
    def drain(self, level):
        match level:
            case 0:
                return "dry"
            case 1 if self.rain:
                return "damp"
            case _:
                return "full"


calm = lambda depth: depth if depth > 2 else 0
"#;

#[test]
fn python_counts_comprehensions_match_arms_and_lambdas() {
    let facts = clean(Language::Python, PYTHON);
    // feed: if, and, for, if (comprehension), elif, while (6); drain: three cases (2) and a
    // guard (1); the lambda: if-else (1).
    assert_eq!(
        facts.counted,
        Counted {
            lines: 18,
            functions: 3,
            decision_points: 10,
            max_cyclomatic: 7,
            max_nesting: 1,
            nesting_total: 2,
            todo: 0,
            fixme: 1,
            hack: 0,
        }
    );
    assert_eq!(facts.exports, 0, "exports are counted for Knip's languages");
}

const RUST: &str = r#"// HACK: borrow the watering can
fn sprout(seeds: &[u32], warm: bool) -> u32 {
    let mut grown = 0;
    for seed in seeds {
        if warm || *seed > 3 {
            grown += 1;
        } else if let Some(x) = seed.checked_sub(1) && x > 0 {
            grown += x;
        }
    }
    grown
}

fn season(month: u8) -> &'static str {
    match month {
        3..=5 => "spring",
        6..=8 if month != 7 => "summer",
        _ => "rest",
    }
}

/// Doc comments are comments too: TODO counts here.
fn dig(depth: Option<u8>) -> u8 {
    let Some(d) = depth else { return 0 };
    let deeper = |x: u8| if x > 2 { x } else { 0 };
    deeper(d)
}
"#;

#[test]
fn rust_counts_let_chains_guards_let_else_and_closures() {
    let facts = clean(Language::Rust, RUST);
    // sprout: for, if, ||, else if, a let chain's && (5); season: three arms (2) and a guard
    // (1); dig: let-else (1); its closure: if (1).
    assert_eq!(
        facts.counted,
        Counted {
            lines: 23,
            functions: 4,
            decision_points: 10,
            max_cyclomatic: 6,
            max_nesting: 2,
            nesting_total: 2 + 1 + 1,
            todo: 1,
            fixme: 0,
            hack: 1,
        }
    );
}

const GO: &str = r"// TODO(gardener): label the beds
package garden

func Water(beds []int, rain bool) int {
	total := 0
	for _, bed := range beds {
		if bed > 0 && !rain {
			total += bed
		} else if bed < 0 {
			total--
		} else {
			total++
		}
	}
	switch {
	case total > 10:
		return 10
	case total < 0:
		return 0
	default:
		return total
	}
}

func (g *Gate) Swing(open bool) func() bool {
	return func() bool {
		select {
		case <-g.wind:
			return open || g.latched
		default:
			return false
		}
	}
}
";

#[test]
fn go_counts_cases_selects_and_func_literals() {
    let facts = clean(Language::Go, GO);
    // Water: for, if, &&, else if, two cases (6); Swing: none; its literal: a select case, || (2).
    assert_eq!(
        facts.counted,
        Counted {
            lines: 31,
            functions: 3,
            decision_points: 8,
            max_cyclomatic: 7,
            max_nesting: 2,
            nesting_total: 2 + 1,
            todo: 1,
            fixme: 0,
            hack: 0,
        }
    );
}

#[test]
fn bodiless_signatures_are_not_functions() {
    let typescript = clean(
        Language::TypeScript,
        "interface Gate { swing(): void; }\n\
         abstract class Shed { abstract open(): void; }\n\
         declare function water(): void;\n\
         function mow(): void;\n\
         function mow(height?: number): void {}\n",
    );
    assert_eq!(typescript.counted.functions, 1, "only mow's body");
    let rust = clean(
        Language::Rust,
        "trait Gate { fn swing(&self); fn shut(&self) {} }\n",
    );
    assert_eq!(rust.counted.functions, 1, "only shut has a body");
    let go = clean(
        Language::Go,
        "package g\ntype Gate interface { Swing() }\nfunc Shut() {}\n",
    );
    assert_eq!(go.counted.functions, 1, "only Shut has a body");
}

#[test]
fn lines_of_code_leave_out_blank_and_comment_lines() {
    let source = "// a comment alone\n\
                  \n\
                  fn a() { // code, then a comment\n\
                  \x20   let s = \"one\n\
                  two\";\n\
                  \x20   /* a block\n\
                  \x20      comment */\n\
                  }\n";
    let facts = clean(Language::Rust, source);
    // `fn a()`, `let s = "one`, `two";` (the string spans it) and `}`.
    assert_eq!(facts.counted.lines, 4);
    assert_eq!(clean(Language::Rust, "").counted.lines, 0);
}

#[test]
fn markers_are_whole_upper_case_words() {
    assert_eq!(words(b"TODO", b"TODO"), 1);
    assert_eq!(words(b"TODO: x, TODO(me) and (TODO)", b"TODO"), 3);
    for text in [
        &b"todo"[..],
        b"TODOS",
        b"xTODO",
        b"TODO_LIST",
        b"TODO2",
        b"_TODO",
    ] {
        assert_eq!(words(text, b"TODO"), 0, "{}", String::from_utf8_lossy(text));
    }
    let facts = clean(
        Language::Python,
        "def f():\n    return \"TODO in a string is code\"  # HACK, FIXME and HACKS\n",
    );
    assert_eq!(
        (facts.counted.todo, facts.counted.fixme, facts.counted.hack),
        (0, 1, 1),
        "only comments hold markers"
    );
}

#[test]
fn every_way_of_exporting_a_name_counts_it_once() {
    let source = r#"export function a() {}
export function over(x: string): void;
export function over(x: number): void;
export function over(x: any) {}
export const b = 1, { c, d: e, f = g } = obj, [h, ...i] = list;
export class K {}
export abstract class L {}
export interface I {}
export type T = string;
export enum E { A }
export namespace N { export const inner = 1; }
export declare const decl: number;
export { x, y as z };
export { w } from "./w";
export * from "./all";
export * as ns from "./ns";
export type { Q } from "./q";
export default 42;
const notExported = 1;
"#;
    let facts = clean(Language::TypeScript, source);
    // a, over, b, c, e, f, h, i, K, L, I, T, E, N, decl, x, z, w, ns, Q and the default; the
    // key `d`, the default value `g`, the namespace's own export and `export *` name nothing.
    assert_eq!(facts.exports, 21);
    let assignment = clean(Language::TypeScript, "const a = 1;\nexport = a;\n");
    assert_eq!(assignment.exports, 1);
    let alias = clean(
        Language::TypeScript,
        "namespace B { export const C = 1; }\nexport import A = B.C;\nexport as namespace Lib;\n",
    );
    assert_eq!(alias.exports, 1, "the alias; a UMD global is no export");
    let default_class = clean(
        Language::JavaScript,
        "export default class Shed {}\nexport { Shed as default };\n",
    );
    assert_eq!(
        default_class.exports, 1,
        "a default export is named default"
    );
}

#[test]
fn a_file_with_errors_still_counts_what_it_holds() {
    let facts = facts(
        Language::Rust,
        "fn ok(a: bool) { if a {} }\nfn broken( { let x = ; }\n",
    );
    assert_eq!(facts.read, Read::Partial);
    assert!(facts.counted.functions >= 1, "{:?}", facts.counted);
    assert!(facts.counted.decision_points >= 1, "{:?}", facts.counted);
}

/// A source long enough that its parse reaches the progress callback many times.
fn long_source() -> String {
    use std::fmt::Write as _;
    (0..2_000).fold(String::new(), |mut source, n| {
        let _ = writeln!(
            source,
            "fn f{n}(a: bool) -> u32 {{ if a {{ {n} }} else {{ 0 }} }}"
        );
        source
    })
}

#[test]
fn a_parse_past_the_limit_counts_nothing_and_leaves_the_next_file_whole() {
    let mut metrics = Metrics::with_limit(Duration::ZERO);
    let stopped = metrics
        .parse(Language::Rust, long_source().as_bytes(), &mut || false)
        .unwrap();
    assert_eq!(stopped.read, Read::Unparsed);
    assert_eq!(stopped.counted, Counted::default());
    // The same parser, now with time: the stopped parse is not resumed into this file.
    metrics.limit = PARSE_LIMIT;
    let next = metrics
        .parse(Language::Rust, RUST.as_bytes(), &mut || false)
        .unwrap();
    assert_eq!(next, clean(Language::Rust, RUST));
}

#[test]
fn a_stop_during_a_parse_cancels_and_leaves_the_parser_fresh() {
    let mut metrics = Metrics::new();
    let mut asked = 0;
    let cancelled = metrics.parse(Language::Rust, long_source().as_bytes(), &mut || {
        asked += 1;
        asked > 1
    });
    assert_eq!(cancelled, Err(GitError::Cancelled));
    let next = metrics
        .parse(Language::Rust, RUST.as_bytes(), &mut || false)
        .unwrap();
    assert_eq!(next, clean(Language::Rust, RUST));
}

#[test]
fn every_name_in_the_tables_is_in_its_grammar() {
    for language in Language::ALL {
        let grammar = language.grammar();
        assert!(
            Parser::new().set_language(&grammar).is_ok(),
            "{language:?}: tree-sitter loads its grammar"
        );
        let table = language.table();
        let kind = |name: &str| {
            assert_ne!(
                grammar.id_for_node_kind(name, true),
                0,
                "{language:?}: no node {name}"
            );
        };
        let token = |name: &str| {
            assert_ne!(
                grammar.id_for_node_kind(name, false),
                0,
                "{language:?}: no token {name}"
            );
        };
        let field = |name: &str| {
            assert!(
                grammar.field_id_for_name(name).is_some(),
                "{language:?}: no field {name}"
            );
        };
        for name in table
            .functions
            .iter()
            .chain(table.decisions)
            .chain(table.nesting)
            .chain(table.comments)
        {
            kind(name);
        }
        for (node, operators) in table.operators {
            kind(node);
            field("operator");
            operators.iter().for_each(|op| token(op));
        }
        for (node, name) in table.with_field {
            kind(node);
            field(name);
        }
        for (node, name) in table.tokens {
            kind(node);
            token(name);
        }
        if let Some(arms) = &table.arms {
            kind(arms.node);
            kind(arms.arm);
            arms.field.iter().for_each(|name| field(name));
        }
        match table.else_if {
            Some(lang::ElseIf::Inside(node)) => kind(node),
            Some(lang::ElseIf::Alternative) => field("alternative"),
            None => {}
        }
    }
}

#[test]
fn languages_are_read_from_extensions() {
    let of = |path: &str| Language::of_path(path.as_bytes());
    assert_eq!(of("src/a.ts"), Some(Language::TypeScript));
    assert_eq!(of("src/a.mts"), Some(Language::TypeScript));
    assert_eq!(of("src/a.cts"), Some(Language::TypeScript));
    assert_eq!(of("src/a.tsx"), Some(Language::Tsx));
    assert_eq!(of("a.js"), Some(Language::JavaScript));
    assert_eq!(of("a.mjs"), Some(Language::JavaScript));
    assert_eq!(of("a.cjs"), Some(Language::JavaScript));
    assert_eq!(of("a.jsx"), Some(Language::JavaScript));
    assert_eq!(of("tools/a.py"), Some(Language::Python));
    assert_eq!(of("src/lib.rs"), Some(Language::Rust));
    assert_eq!(of("main.go"), Some(Language::Go));
    for path in [
        "types/a.d.ts",
        "a.d.mts",
        "a.d.cts",
        "a.pyi",
        "README.md",
        ".ts",
        "dir.rs/file",
        "Makefile",
    ] {
        assert_eq!(of(path), None, "{path}");
    }
}

// Commits.

fn repo(dir: &Path) -> FixtureRepo {
    FixtureRepo::init(&dir.join("repo")).expect("init")
}

fn commit(repo: &mut FixtureRepo, day: i64, edits: &[Edit]) -> CommitId {
    repo.commit(
        "main",
        Stamp::at(1_700_000_000 + day * 86_400),
        "grow",
        edits,
    )
    .expect("commit")
}

fn measure(metrics: &mut Metrics, repo: &FixtureRepo, commit: &CommitId) -> Measurement {
    let git = GitRepo::open(repo.path()).expect("open");
    metrics
        .measure(&git, commit, &mut || false)
        .expect("measured")
}

#[test]
fn only_the_committed_blobs_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = repo(dir.path());
    let tip = commit(&mut fixture, 0, &[Edit::write("src/a.rs", "fn a() {}\n")]);
    // The worktree says otherwise, and holds a file never committed.
    fs::create_dir_all(fixture.path().join("src")).unwrap();
    fs::write(
        fixture.path().join("src/a.rs"),
        "fn a() {}\nfn b() {}\nfn c() {}\n",
    )
    .unwrap();
    fs::write(fixture.path().join("src/new.rs"), "fn d() {}\n").unwrap();
    let measured = measure(&mut Metrics::new(), &fixture, &tip);
    assert_eq!(
        measured.files.keys().collect::<Vec<_>>(),
        ["src/a.rs"],
        "the worktree's own file is not read"
    );
    assert_eq!(measured.files["src/a.rs"].counted.functions, 1);
}

#[test]
fn excluded_binary_oversized_and_foreign_files_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = repo(dir.path());
    let big = format!(
        "// {}\n",
        "x".repeat(usize::try_from(MAX_SOURCE_BYTES).unwrap())
    );
    let tip = commit(
        &mut fixture,
        0,
        &[
            Edit::write("src/keep.ts", "export const a = 1;\n"),
            Edit::write("node_modules/dep/index.js", "export const b = 1;\n"),
            Edit::write("vendor/lib.go", "package lib\n"),
            Edit::write("dist/app.js", "let c = 1;\n"),
            Edit::write("static/app.min.js", "let d=1;\n"),
            Edit::write("types/api.d.ts", "export declare const e: number;\n"),
            Edit::write("README.md", "# moss\n"),
            Edit::Write("src/blob.ts".into(), b"let f = 1;\0\n".to_vec()),
            Edit::write("src/huge.rs", &big),
        ],
    );
    let measured = measure(&mut Metrics::new(), &fixture, &tip);
    assert_eq!(
        measured.files.keys().collect::<Vec<_>>(),
        ["src/huge.rs", "src/keep.ts"]
    );
    assert_eq!(measured.files["src/huge.rs"].read, Read::Oversized);
    let complexity = Measurement {
        files: [("src/f.rs".to_owned(), clean(Language::Rust, "fn f() {}\n"))]
            .into_iter()
            .chain(measured.files.clone())
            .collect(),
    }
    .complexity()
    .expect("a function");
    assert_eq!(
        complexity.sources,
        Sources {
            files: 3,
            partial: 0,
            unparsed: 0,
            oversized: 1,
        }
    );
    assert_eq!(
        complexity.sources.confidence(),
        Q16::ONE,
        "oversized is left out"
    );
}

#[test]
fn an_unchanged_blob_is_never_parsed_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = repo(dir.path());
    let first = commit(
        &mut fixture,
        0,
        &[
            Edit::write("src/a.rs", "fn a() {}\n"),
            Edit::write("src/b.rs", "fn b() {}\n"),
            Edit::write("web/c.ts", "export function c() {}\n"),
        ],
    );
    let mut metrics = Metrics::new();
    measure(&mut metrics, &fixture, &first);
    assert_eq!(metrics.parsed(), 3);
    let second = commit(
        &mut fixture,
        1,
        &[Edit::write("src/b.rs", "fn b() { if true {} }\n")],
    );
    let measured = measure(&mut metrics, &fixture, &second);
    assert_eq!(metrics.parsed(), 4, "only b changed");
    assert_eq!(measured.files["src/b.rs"].counted.decision_points, 1);
    // The first commit again: b's old blob was used by the measurement before, so it is kept.
    measure(&mut metrics, &fixture, &first);
    assert_eq!(metrics.parsed(), 4);
}

#[test]
fn a_cancelled_measurement_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = repo(dir.path());
    let tip = commit(&mut fixture, 0, &[Edit::write("a.py", "x = 1\n")]);
    let git = GitRepo::open(fixture.path()).unwrap();
    assert_eq!(
        Metrics::new().measure(&git, &tip, &mut || true),
        Err(GitError::Cancelled)
    );
}

#[test]
fn languages_are_counted_from_paths() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = repo(dir.path());
    let tip = commit(
        &mut fixture,
        0,
        &[
            Edit::write("a.rs", ""),
            Edit::write("b.rs", ""),
            Edit::write("c.tsx", ""),
            Edit::write("node_modules/d.js", ""),
            Edit::write("e.md", ""),
        ],
    );
    let git = GitRepo::open(fixture.path()).unwrap();
    assert_eq!(
        languages_at(&git, &tip).unwrap(),
        [(Language::Tsx, 1), (Language::Rust, 2)].into()
    );
}

// Readings.

fn file(language: Language, counted: Counted, exports: u64) -> FileFacts {
    FileFacts {
        language,
        read: Read::Clean,
        counted,
        exports,
    }
}

fn measured(files: &[(&str, FileFacts)]) -> Measurement {
    Measurement {
        files: files
            .iter()
            .map(|(path, facts)| ((*path).to_owned(), *facts))
            .collect(),
    }
}

#[test]
fn complexity_is_the_mean_cyclomatic_complexity_on_section_4_3s_curve() {
    let functions = |functions, decision_points| Counted {
        functions,
        decision_points,
        ..Counted::default()
    };
    // A mean of 8: (8 − 1) / 14 = 0.5.
    let reading = measured(&[
        ("a.rs", file(Language::Rust, functions(4, 30), 0)),
        ("b.go", file(Language::Go, functions(6, 40), 0)),
    ])
    .complexity()
    .unwrap();
    assert_eq!(reading.normalized, Q16::HALF);
    assert_eq!((reading.functions, reading.decision_points), (10, 70));
    assert_eq!(
        reading.contributors,
        [
            Contributor {
                path: "b.go".into(),
                value: 40
            },
            Contributor {
                path: "a.rs".into(),
                value: 30
            },
        ]
    );
    // A mean of 15 or more is 0; one of 1 is 1.
    let worst = measured(&[("a.rs", file(Language::Rust, functions(1, 20), 0))]);
    assert_eq!(worst.complexity().unwrap().normalized, Q16::ZERO);
    let simplest = measured(&[("a.rs", file(Language::Rust, functions(3, 0), 0))]);
    assert_eq!(simplest.complexity().unwrap().normalized, Q16::ONE);
    // No function: unknown, never 0.
    let none = measured(&[("a.py", file(Language::Python, Counted::default(), 0))]);
    assert_eq!(none.complexity(), None);
}

#[test]
fn todo_density_is_markers_per_thousand_lines_on_section_4_3s_curve() {
    let lines = |lines, todo, fixme, hack| Counted {
        lines,
        todo,
        fixme,
        hack,
        ..Counted::default()
    };
    // 10 markers in 1,000 lines: 10 / 20 = 0.5.
    let reading = measured(&[
        ("a.ts", file(Language::TypeScript, lines(600, 4, 2, 0), 0)),
        ("b.py", file(Language::Python, lines(400, 0, 0, 4), 0)),
    ])
    .todo_density()
    .unwrap();
    assert_eq!(reading.normalized, Q16::HALF);
    assert_eq!(
        (reading.lines, reading.todo, reading.fixme, reading.hack),
        (1_000, 4, 2, 4)
    );
    assert_eq!(reading.contributors[0].path, "a.ts");
    let clear = measured(&[("a.rs", file(Language::Rust, lines(50, 0, 0, 0), 0))]);
    assert_eq!(clear.todo_density().unwrap().normalized, Q16::ONE);
    assert!(clear.todo_density().unwrap().contributors.is_empty());
    let empty = measured(&[("a.rs", file(Language::Rust, Counted::default(), 0))]);
    assert_eq!(empty.todo_density(), None);
}

#[test]
fn a_partial_or_unparsed_file_lowers_confidence() {
    let one = Counted {
        lines: 1,
        functions: 1,
        ..Counted::default()
    };
    let mut files = measured(&[
        ("a.rs", file(Language::Rust, one, 0)),
        ("b.rs", file(Language::Rust, one, 0)),
        ("c.rs", file(Language::Rust, one, 0)),
        ("d.rs", file(Language::Rust, one, 0)),
    ]);
    files.files.get_mut("b.rs").unwrap().read = Read::Partial;
    files.files.get_mut("c.rs").unwrap().read = Read::Unparsed;
    let reading = files.complexity().unwrap();
    assert_eq!(reading.sources.confidence(), Q16::HALF);
    assert_eq!(
        reading.functions, 3,
        "the partial file counts; the unparsed one cannot"
    );
}

#[test]
fn dead_code_weighs_knips_unused_items_against_the_exports_counted_here() {
    let exports = |n| file(Language::TypeScript, Counted::default(), n);
    let measurement = measured(&[
        ("src/math.ts", exports(10)),
        ("src/legacy.ts", exports(4)),
        ("src/app.tsx", exports(6)),
        (
            "tools/report.py",
            file(Language::Python, Counted::default(), 0),
        ),
    ]);
    let knip = KnipCounts {
        unused_exports: 5,
        unused_exports_per_file: [
            ("src/math.ts".to_owned(), 3),
            // A file not parsed here weighs on neither side.
            ("src/widget.vue".to_owned(), 2),
        ]
        .into(),
        unused_files: vec!["src/legacy.ts".into(), "src/old.vue".into()],
        ..KnipCounts::default()
    };
    let reading = measurement.dead_code(&knip).unwrap();
    // 3 unused exports and legacy's 4, against 20 exported.
    assert_eq!((reading.unused_items, reading.exported_items), (7, 20));
    assert_eq!(
        reading.normalized,
        Q16::ONE.saturating_sub(Q16::from_ratio(7, 20))
    );
    assert_eq!(
        reading.contributors,
        [
            Contributor {
                path: "src/legacy.ts".into(),
                value: 4
            },
            Contributor {
                path: "src/math.ts".into(),
                value: 3
            },
        ]
    );
    assert_eq!(reading.confidence, Q16::ONE);
    // No export counted: unknown, never 0.
    let none = measured(&[(
        "tools/report.py",
        file(Language::Python, Counted::default(), 0),
    )]);
    assert_eq!(none.dead_code(&knip), None);
}
