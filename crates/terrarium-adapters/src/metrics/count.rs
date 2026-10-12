//! One file's counts from its syntax tree, in one walk: functions and their cyclomatic
//! complexity and nesting, TODO markers in comments, and lines of code.

use std::num::NonZeroU16;

use tree_sitter::{Node, Tree, TreeCursor};

use super::lang::{ElseIf, Table};

/// What a file's tree counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counted {
    /// Lines holding code: blank lines and lines of comment alone are not counted.
    pub lines: u64,
    pub functions: u64,
    /// Decisions inside functions; a function's cyclomatic complexity is 1 plus its own.
    pub decision_points: u64,
    pub max_cyclomatic: u64,
    /// The deepest nesting of control structures in any one function.
    pub max_nesting: u64,
    /// Each function's deepest nesting, summed.
    pub nesting_total: u64,
    pub todo: u64,
    pub fixme: u64,
    pub hack: u64,
}

/// What one node kind counts for, resolved from a [`Table`] against a grammar's ids.
#[derive(Clone, Copy, Debug, Default)]
struct Role {
    part: Part,
    decision: bool,
    /// A decision only with one of these in its `operator` field.
    operators: Option<&'static [&'static str]>,
    /// A decision when this field is present.
    with_field: Option<NonZeroU16>,
    /// Each unnamed child of this kind is a decision.
    token: Option<&'static str>,
    /// Holds `match` arms: a decision each but the first.
    arms: Option<(Option<NonZeroU16>, &'static str)>,
    /// A control structure, which nests what it holds.
    nesting: bool,
}

/// What a node is, beside the decisions it makes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Part {
    #[default]
    Code,
    Function,
    Comment,
    /// An `else` clause, inside which an `if` continues a chain rather than nesting.
    ElseClause,
}

/// A grammar's node kinds, each with its role.
pub(super) struct Roles {
    by_kind: Vec<Role>,
    operator: Option<NonZeroU16>,
    alternative: Option<NonZeroU16>,
    else_if_alternative: bool,
}

impl Roles {
    pub(super) fn new(grammar: &tree_sitter::Language, table: &'static Table) -> Roles {
        let field = |name: &str| grammar.field_id_for_name(name);
        let mut by_kind = vec![Role::default(); grammar.node_kind_count()];
        for (id, role) in (0..).zip(by_kind.iter_mut()) {
            let Some(kind) = grammar.node_kind_for_id(id) else {
                continue;
            };
            if !grammar.node_kind_is_named(id) {
                continue;
            }
            role.part = if table.functions.contains(&kind) {
                Part::Function
            } else if table.comments.contains(&kind) {
                Part::Comment
            } else if matches!(table.else_if, Some(ElseIf::Inside(node)) if node == kind) {
                Part::ElseClause
            } else {
                Part::Code
            };
            role.decision = table.decisions.contains(&kind);
            role.operators = table
                .operators
                .iter()
                .find(|(node, _)| *node == kind)
                .map(|(_, operators)| *operators);
            role.with_field = table
                .with_field
                .iter()
                .find(|(node, _)| *node == kind)
                .and_then(|(_, name)| field(name));
            role.token = table
                .tokens
                .iter()
                .find(|(node, _)| *node == kind)
                .map(|(_, token)| *token);
            role.arms = table
                .arms
                .as_ref()
                .filter(|arms| arms.node == kind)
                .map(|arms| (arms.field.and_then(field), arms.arm));
            role.nesting = table.nesting.contains(&kind);
        }
        Roles {
            by_kind,
            operator: field("operator"),
            alternative: field("alternative"),
            else_if_alternative: matches!(table.else_if, Some(ElseIf::Alternative)),
        }
    }

    fn of(&self, kind: u16) -> Role {
        self.by_kind
            .get(usize::from(kind))
            .copied()
            .unwrap_or_default()
    }
}

/// A function being walked.
#[derive(Default)]
struct Frame {
    decisions: u64,
    depth: u64,
    deepest: u64,
}

/// What leaving a node undoes.
#[derive(Clone, Copy)]
enum Exit {
    Nothing,
    Function,
    Unnest,
}

/// Counts `tree`, parsed from `source`. Decisions outside any function count nowhere.
pub(super) fn count(tree: &Tree, source: &[u8], roles: &Roles) -> Counted {
    let mut counted = Counted::default();
    let mut rows = Rows::new(source);
    let mut frames: Vec<Frame> = Vec::new();
    // The kinds of the open ancestors, and what leaving each undoes.
    let mut open: Vec<(u16, Exit)> = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let role = roles.of(node.kind_id());
        if role.part == Part::Comment {
            markers(&source[node.byte_range()], &mut counted);
        } else {
            let parent = open.last().map(|(kind, _)| roles.of(*kind));
            let exit = enter(node, role, parent, &cursor, roles, &mut frames);
            if cursor.goto_first_child() {
                open.push((node.kind_id(), exit));
                continue;
            }
            rows.mark(node, source);
            leave(exit, &mut frames, &mut counted);
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                counted.lines = rows.count();
                return counted;
            }
            if let Some((_, exit)) = open.pop() {
                leave(exit, &mut frames, &mut counted);
            }
        }
    }
}

fn enter(
    node: Node<'_>,
    role: Role,
    parent: Option<Role>,
    cursor: &TreeCursor<'_>,
    roles: &Roles,
    frames: &mut Vec<Frame>,
) -> Exit {
    if role.part == Part::Function {
        frames.push(Frame::default());
        return Exit::Function;
    }
    let Some(frame) = frames.last_mut() else {
        return Exit::Nothing;
    };
    frame.decisions += decisions(node, role, roles);
    let chained = parent.is_some_and(|parent| parent.part == Part::ElseClause)
        || (roles.else_if_alternative
            && cursor.field_id().is_some()
            && cursor.field_id() == roles.alternative);
    if role.nesting && !chained {
        frame.depth += 1;
        frame.deepest = frame.deepest.max(frame.depth);
        return Exit::Unnest;
    }
    Exit::Nothing
}

/// The decisions `node` itself makes.
fn decisions(node: Node<'_>, role: Role, roles: &Roles) -> u64 {
    let mut made = u64::from(role.decision);
    if let (Some(operators), Some(field)) = (role.operators, roles.operator) {
        let operator = node.child_by_field_id(field.get());
        made += u64::from(operator.is_some_and(|op| operators.contains(&op.kind())));
    }
    if let Some(field) = role.with_field {
        made += u64::from(node.child_by_field_id(field.get()).is_some());
    }
    if let Some(token) = role.token {
        let mut walk = node.walk();
        made += count_u64(
            node.children(&mut walk)
                .filter(|child| !child.is_named() && child.kind() == token),
        );
    }
    if let Some((field, arm)) = role.arms {
        let holder = match field {
            Some(field) => node.child_by_field_id(field.get()),
            None => Some(node),
        };
        if let Some(holder) = holder {
            let mut walk = holder.walk();
            let arms = count_u64(
                holder
                    .named_children(&mut walk)
                    .filter(|child| child.kind() == arm),
            );
            made += arms.saturating_sub(1);
        }
    }
    made
}

fn leave(exit: Exit, frames: &mut Vec<Frame>, counted: &mut Counted) {
    match exit {
        Exit::Nothing => {}
        Exit::Unnest => {
            if let Some(frame) = frames.last_mut() {
                frame.depth = frame.depth.saturating_sub(1);
            }
        }
        Exit::Function => {
            if let Some(frame) = frames.pop() {
                counted.functions += 1;
                counted.decision_points += frame.decisions;
                counted.max_cyclomatic = counted.max_cyclomatic.max(1 + frame.decisions);
                counted.max_nesting = counted.max_nesting.max(frame.deepest);
                counted.nesting_total += frame.deepest;
            }
        }
    }
}

fn count_u64<T>(items: impl Iterator<Item = T>) -> u64 {
    items.fold(0, |n, _| n + 1)
}

/// `TODO`, `FIXME` and `HACK` in a comment's text: upper case, as whole words.
fn markers(text: &[u8], counted: &mut Counted) {
    counted.todo += words(text, b"TODO");
    counted.fixme += words(text, b"FIXME");
    counted.hack += words(text, b"HACK");
}

/// How often `word` stands in `text` with no letter, digit or underscore against either end.
pub(super) fn words(text: &[u8], word: &[u8]) -> u64 {
    let part_of_word = |b: Option<&u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
    let mut found = 0;
    let mut at = 0;
    while let Some(offset) = text
        .get(at..)
        .and_then(|rest| rest.windows(word.len()).position(|w| w == word))
    {
        let start = at + offset;
        let end = start + word.len();
        let before = start.checked_sub(1).and_then(|i| text.get(i));
        if !part_of_word(before) && !part_of_word(text.get(end)) {
            found += 1;
        }
        at = end;
    }
    found
}

/// The source's lines, marked as code is found on them.
struct Rows(Vec<u64>);

impl Rows {
    fn new(source: &[u8]) -> Rows {
        let lines = source.split(|&b| b == b'\n').count();
        Rows(vec![0; lines.div_ceil(64)])
    }

    /// Marks the lines on which a token holds something other than whitespace, so markup's
    /// whitespace between elements (JSX text) never makes a blank line code, and an empty token
    /// (one the parser supplied as missing) marks nothing.
    fn mark(&mut self, node: Node<'_>, source: &[u8]) {
        let mut row = node.start_position().row;
        for &byte in source.get(node.byte_range()).unwrap_or_default() {
            if byte == b'\n' {
                row += 1;
            } else if !byte.is_ascii_whitespace()
                && let Some(word) = self.0.get_mut(row / 64)
            {
                *word |= 1 << (row % 64);
            }
        }
    }

    fn count(&self) -> u64 {
        self.0.iter().map(|word| u64::from(word.count_ones())).sum()
    }
}
