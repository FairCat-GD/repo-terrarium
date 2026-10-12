//! The tier-one languages and what each grammar's nodes mean to the counts: one table per
//! language. A tier-two language would be one more table behind a cargo feature.
//!
//! Node and field names are the grammars' own (`node-types.json` at the pinned versions); the
//! table test checks that each one exists, so a grammar upgrade that renames one fails there
//! rather than counting nothing.

/// A language the source metrics read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Rust,
    Go,
}

impl Language {
    pub const ALL: [Language; 6] = [
        Language::TypeScript,
        Language::Tsx,
        Language::JavaScript,
        Language::Python,
        Language::Rust,
        Language::Go,
    ];

    /// The language of a repository path, by its extension. Declaration files (`.d.ts` and its
    /// module forms) hold types, not code, and are not source.
    pub fn of_path(path: &[u8]) -> Option<Language> {
        let name = path.rsplit(|&b| b == b'/').next().unwrap_or(path);
        if [&b".d.ts"[..], b".d.mts", b".d.cts"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            return None;
        }
        let dot = name.iter().rposition(|&b| b == b'.')?;
        if dot == 0 {
            return None;
        }
        Some(match &name[dot + 1..] {
            b"ts" | b"mts" | b"cts" => Language::TypeScript,
            b"tsx" => Language::Tsx,
            b"js" | b"mjs" | b"cjs" | b"jsx" => Language::JavaScript,
            b"py" => Language::Python,
            b"rs" => Language::Rust,
            b"go" => Language::Go,
            _ => return None,
        })
    }

    /// TypeScript, TSX and JavaScript: the languages Knip reads, whose exports are counted.
    pub const fn exports_counted(self) -> bool {
        matches!(
            self,
            Language::TypeScript | Language::Tsx | Language::JavaScript
        )
    }

    pub(super) fn grammar(self) -> tree_sitter::Language {
        tree_sitter::Language::new(match self {
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX,
            Language::JavaScript => tree_sitter_javascript::LANGUAGE,
            Language::Python => tree_sitter_python::LANGUAGE,
            Language::Rust => tree_sitter_rust::LANGUAGE,
            Language::Go => tree_sitter_go::LANGUAGE,
        })
    }

    pub(super) const fn table(self) -> &'static Table {
        match self {
            Language::TypeScript | Language::Tsx | Language::JavaScript => &ECMASCRIPT,
            Language::Python => &PYTHON,
            Language::Rust => &RUST,
            Language::Go => &GO,
        }
    }
}

/// What a grammar's nodes count for.
pub(super) struct Table {
    /// Nodes with a body that are a function of their own: declarations, methods, closures,
    /// lambdas. A bodiless signature is not one.
    pub functions: &'static [&'static str],
    /// Nodes that are one decision each.
    pub decisions: &'static [&'static str],
    /// Nodes that are a decision only with one of these operators in their `operator` field.
    pub operators: &'static [(&'static str, &'static [&'static str])],
    /// Nodes that are a decision when they have this field: a match guard, `let … else`.
    pub with_field: &'static [(&'static str, &'static str)],
    /// Nodes whose unnamed tokens of this kind are a decision each: the `&&` of a let chain.
    pub tokens: &'static [(&'static str, &'static str)],
    /// A `match`, whose arms are a decision each but the first.
    pub arms: Option<Arms>,
    /// Control structures, whose nesting inside a function is measured.
    pub nesting: &'static [&'static str],
    /// How an `else if` is written, so that a chain of them does not deepen the nesting; `None`
    /// where the language has a clause of its own for it.
    pub else_if: Option<ElseIf>,
    pub comments: &'static [&'static str],
}

/// The arms of a `match`.
pub(super) struct Arms {
    /// The node holding the arms.
    pub node: &'static str,
    /// The field of `node` they sit in, when they are not its own children.
    pub field: Option<&'static str>,
    pub arm: &'static str,
}

pub(super) enum ElseIf {
    /// The `if` sits inside a node of this kind (an `else` clause).
    Inside(&'static str),
    /// The `if` is the `alternative` field of the `if` before it.
    Alternative,
}

/// JavaScript, TypeScript and TSX share their node names: TypeScript's grammar extends
/// JavaScript's.
static ECMASCRIPT: Table = Table {
    functions: &[
        "function_declaration",
        "function_expression",
        "generator_function",
        "generator_function_declaration",
        "arrow_function",
        "method_definition",
    ],
    decisions: &[
        "if_statement",
        "for_statement",
        "for_in_statement",
        "while_statement",
        "do_statement",
        "catch_clause",
        "ternary_expression",
        "switch_case",
    ],
    operators: &[
        ("binary_expression", &["&&", "||", "??"]),
        ("augmented_assignment_expression", &["&&=", "||=", "??="]),
    ],
    with_field: &[],
    tokens: &[],
    arms: None,
    nesting: &[
        "if_statement",
        "for_statement",
        "for_in_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "try_statement",
    ],
    else_if: Some(ElseIf::Inside("else_clause")),
    comments: &["comment", "html_comment"],
};

static PYTHON: Table = Table {
    functions: &["function_definition", "lambda"],
    decisions: &[
        "if_statement",
        "elif_clause",
        "for_statement",
        "while_statement",
        "except_clause",
        "conditional_expression",
        "boolean_operator",
        "for_in_clause",
        "if_clause",
    ],
    operators: &[],
    with_field: &[],
    tokens: &[],
    arms: Some(Arms {
        node: "match_statement",
        field: Some("body"),
        arm: "case_clause",
    }),
    nesting: &[
        "if_statement",
        "for_statement",
        "while_statement",
        "try_statement",
        "match_statement",
    ],
    // `elif`.
    else_if: None,
    comments: &["comment"],
};

static RUST: Table = Table {
    functions: &["function_item", "closure_expression"],
    decisions: &["if_expression", "while_expression", "for_expression"],
    operators: &[("binary_expression", &["&&", "||"])],
    with_field: &[
        ("match_pattern", "condition"),
        ("let_declaration", "alternative"),
    ],
    tokens: &[("let_chain", "&&")],
    arms: Some(Arms {
        node: "match_block",
        field: None,
        arm: "match_arm",
    }),
    nesting: &[
        "if_expression",
        "while_expression",
        "for_expression",
        "loop_expression",
        "match_expression",
    ],
    else_if: Some(ElseIf::Inside("else_clause")),
    comments: &["line_comment", "block_comment"],
};

static GO: Table = Table {
    functions: &["function_declaration", "method_declaration", "func_literal"],
    decisions: &[
        "if_statement",
        "for_statement",
        "expression_case",
        "type_case",
        "communication_case",
    ],
    operators: &[("binary_expression", &["&&", "||"])],
    with_field: &[],
    tokens: &[],
    arms: None,
    nesting: &[
        "if_statement",
        "for_statement",
        "expression_switch_statement",
        "type_switch_statement",
        "select_statement",
    ],
    else_if: Some(ElseIf::Alternative),
    comments: &["comment"],
};
