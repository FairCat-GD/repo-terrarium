//! The exported items of a TypeScript, TSX or JavaScript file (`dead_code`'s `exported_items`):
//! the distinct names its top-level `export` statements make, as Knip counts exports and
//! exported types.
//!
//! A declaration exports its name, a declarator each binding (destructuring included),
//! `export { a, b as c }` each name it gives (re-exports too), `export * as ns from` one, a
//! default export one, and TypeScript's `export =` one. `export * from` passes another file's
//! exports on and names none here. Names are distinct, so overloads and a declaration merged
//! with a namespace count once. Assignments to `module.exports` are not exports here.

use std::collections::BTreeSet;

use tree_sitter::{Node, Tree};

/// Names standing for exports that have no name of their own.
const DEFAULT: &str = "default";
const EXPORT_ASSIGNMENT: &str = "=";

/// How many distinct names `tree`'s top level exports.
pub(super) fn count(tree: &Tree, source: &[u8]) -> u64 {
    let mut names: BTreeSet<&[u8]> = BTreeSet::new();
    let root = tree.root_node();
    let mut walk = root.walk();
    for statement in root
        .named_children(&mut walk)
        .filter(|node| node.kind() == "export_statement")
    {
        statement_names(statement, source, &mut names);
    }
    u64::try_from(names.len()).unwrap_or(u64::MAX)
}

fn statement_names<'s>(statement: Node<'_>, source: &'s [u8], names: &mut BTreeSet<&'s [u8]>) {
    let mut walk = statement.walk();
    let children: Vec<Node<'_>> = statement.children(&mut walk).collect();
    let token = |kind: &str| children.iter().any(|c| !c.is_named() && c.kind() == kind);
    if token("default") {
        names.insert(DEFAULT.as_bytes());
        return;
    }
    if token("=") {
        names.insert(EXPORT_ASSIGNMENT.as_bytes());
        return;
    }
    // `export as namespace X` declares a global for script use, not an export.
    if token("namespace") {
        return;
    }
    if let Some(declaration) = statement.child_by_field_name("declaration") {
        declaration_names(declaration, source, names);
    }
    for child in &children {
        match child.kind() {
            "export_clause" => {
                let mut walk = child.walk();
                for specifier in child
                    .named_children(&mut walk)
                    .filter(|node| node.kind() == "export_specifier")
                {
                    if let Some(name) = specifier
                        .child_by_field_name("alias")
                        .or_else(|| specifier.child_by_field_name("name"))
                    {
                        names.insert(text(name, source));
                    }
                }
            }
            "namespace_export" => {
                let mut walk = child.walk();
                if let Some(name) = child.named_children(&mut walk).next() {
                    names.insert(text(name, source));
                }
            }
            _ => {}
        }
    }
}

fn declaration_names<'s>(declaration: Node<'_>, source: &'s [u8], names: &mut BTreeSet<&'s [u8]>) {
    match declaration.kind() {
        "lexical_declaration" | "variable_declaration" => {
            let mut walk = declaration.walk();
            for declarator in declaration
                .named_children(&mut walk)
                .filter(|node| node.kind() == "variable_declarator")
            {
                if let Some(name) = declarator.child_by_field_name("name") {
                    bindings(name, source, names);
                }
            }
        }
        // `export declare …`: the declaration inside.
        "ambient_declaration" => {
            let mut walk = declaration.walk();
            for inner in declaration.named_children(&mut walk) {
                declaration_names(inner, source, names);
            }
        }
        // `export import A = B.C`.
        "import_alias" => {
            let mut walk = declaration.walk();
            if let Some(name) = declaration
                .named_children(&mut walk)
                .find(|node| node.kind() == "identifier")
            {
                names.insert(text(name, source));
            }
        }
        _ => {
            if let Some(name) = declaration.child_by_field_name("name") {
                names.insert(text(name, source));
            }
        }
    }
}

/// The names a binding pattern binds: an identifier, or each one a destructuring holds.
fn bindings<'s>(pattern: Node<'_>, source: &'s [u8], names: &mut BTreeSet<&'s [u8]>) {
    match pattern.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            names.insert(text(pattern, source));
        }
        // `{ key: binding }`: the binding, never the key.
        "pair_pattern" => {
            if let Some(value) = pattern.child_by_field_name("value") {
                bindings(value, source, names);
            }
        }
        // `binding = default`: the binding, never the default.
        "object_assignment_pattern" | "assignment_pattern" => {
            if let Some(left) = pattern.child_by_field_name("left") {
                bindings(left, source, names);
            }
        }
        "object_pattern" | "array_pattern" | "rest_pattern" => {
            let mut walk = pattern.walk();
            for inner in pattern.named_children(&mut walk) {
                bindings(inner, source, names);
            }
        }
        _ => {}
    }
}

fn text<'s>(node: Node<'_>, source: &'s [u8]) -> &'s [u8] {
    &source[node.byte_range()]
}
