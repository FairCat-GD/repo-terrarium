//! The source scan behind truth's float and IO walls (I-1).
//!
//! Clippy cannot see every float. `float_arithmetic` skips `const` and `static` bodies, and a
//! float that is only compared or passed to a method (`2.0_f64.sqrt() > 1.0`) names no type
//! path and does no arithmetic. A float can only enter truth through a float literal or a
//! float type's name, so this scan rejects both. It also rejects the ways a file could step
//! around the scan or the crate-level lint walls: a local `allow` of a wall lint, `include!`,
//! and `#[path]`.
//!
//! Truth does no IO either. The scan refuses every path through the standard library's modules
//! that reach outside the process, whatever the path is named: a `use` group's items count, and
//! an alias of `std` itself is refused, since the paths under it would pass unseen. A module
//! alias (`use std::env as e`) is refused at its `use`. The printing macros write to the
//! terminal without naming `std::io`, so they are refused by name.
//!
//! It works on tokens, not text, so comments, strings and char literals never trigger it.

use std::str::FromStr;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

/// The lints that make up truth's walls, the clippy groups that contain them, and `warnings`,
/// which silences any of them still at warn level.
const WALL_LINTS: &[&str] = &[
    "float_arithmetic",
    "as_conversions",
    "disallowed_types",
    "disallowed_methods",
    "all",
    "style",
    "restriction",
    "warnings",
];

const FLOAT_TYPES: &[&str] = &["f16", "f32", "f64", "f128"];

/// The standard library's modules that do IO, read the environment or start threads.
const IO_MODULES: &[&str] = &["fs", "env", "net", "process", "thread", "io"];

const PRINT_MACROS: &[&str] = &["print", "println", "eprint", "eprintln", "dbg"];

/// One place a file breaks the wall, at a 1-based line.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    pub what: String,
}

/// Every finding in `source`, or why it does not lex as Rust.
pub fn scan(source: &str) -> Result<Vec<Finding>, String> {
    let tokens = TokenStream::from_str(source).map_err(|e| e.to_string())?;
    let mut findings = Vec::new();
    scan_stream(tokens, &mut findings);
    Ok(findings)
}

fn scan_stream(tokens: TokenStream, findings: &mut Vec<Finding>) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        let before = |back: usize| index.checked_sub(back).map(|i| &tokens[i]);
        let after = tokens.get(index + 1);
        let line = token.span().start().line;
        match token {
            TokenTree::Literal(literal) => {
                let text = literal.to_string();
                if is_float_literal(&text) && !is_tuple_index(&text, before(1), before(2)) {
                    findings.push(Finding {
                        line,
                        what: format!("the float literal `{text}`"),
                    });
                }
            }
            TokenTree::Ident(ident) => {
                let text = ident.to_string();
                // A raw identifier (`r#f64`) names the same thing as the plain one.
                let name = text.strip_prefix("r#").unwrap_or(&text);
                if FLOAT_TYPES.contains(&name) {
                    findings.push(Finding {
                        line,
                        what: format!("the float type `{name}`"),
                    });
                }
                if matches!(name, "allow" | "expect" | "warn")
                    && let Some(TokenTree::Group(group)) = after
                    && group.delimiter() == Delimiter::Parenthesis
                    && let Some(lint) = names_a_wall_lint(group.stream())
                {
                    findings.push(Finding {
                        line,
                        what: format!("`{name}` of the wall lint `{lint}`"),
                    });
                }
                let bang = matches!(after, Some(TokenTree::Punct(p)) if p.as_char() == '!');
                if name == "include" && bang {
                    findings.push(Finding {
                        line,
                        what: "`include!`, which hides code from the scan".to_owned(),
                    });
                }
                if PRINT_MACROS.contains(&name) && bang {
                    findings.push(Finding {
                        line,
                        what: format!("`{name}!`, which writes to the terminal"),
                    });
                }
                if name == "std" {
                    findings.extend(
                        through_std(&tokens[index + 1..])
                            .into_iter()
                            .map(|what| Finding { line, what }),
                    );
                }
            }
            TokenTree::Punct(punct) if punct.as_char() == '#' => {
                if let Some(TokenTree::Group(group)) = after
                    && group.delimiter() == Delimiter::Bracket
                    && matches!(
                        group.stream().into_iter().next(),
                        Some(TokenTree::Ident(ident)) if ident == "path"
                    )
                {
                    findings.push(Finding {
                        line,
                        what: "`#[path]`, which hides a file from the scan".to_owned(),
                    });
                }
            }
            TokenTree::Group(group) => scan_stream(group.stream(), findings),
            TokenTree::Punct(_) => {}
        }
    }
}

/// A numeric literal is a float if its leading digits are followed by a point or an exponent,
/// or if it carries a float suffix. Radix-prefixed literals are integers (`0x1f32` included).
fn is_float_literal(text: &str) -> bool {
    let Some(first) = text.chars().next() else {
        return false;
    };
    if !first.is_ascii_digit() || ["0x", "0o", "0b"].iter().any(|p| text.starts_with(p)) {
        return false;
    }
    let rest = text.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_');
    rest.starts_with(['.', 'e', 'E']) || FLOAT_TYPES.contains(&rest)
}

/// The lexer reads `pair.0.1` as `pair`, `.`, `0.1`, as rustc does, so a literal of digits
/// right after a lone `.` is a tuple index. After `..` it is a range bound and still counts.
fn is_tuple_index(
    text: &str,
    previous: Option<&TokenTree>,
    before_that: Option<&TokenTree>,
) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let shaped = match text.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(text),
    };
    let after_lone_dot = matches!(previous, Some(TokenTree::Punct(p)) if p.as_char() == '.' && p.spacing() == Spacing::Alone)
        && !matches!(before_that, Some(TokenTree::Punct(p)) if p.as_char() == '.');
    shaped && after_lone_dot
}

/// What a path through `std` reaches, from the tokens after `std`: an IO module after `::`,
/// each IO module among a `{…}` group's items, or an alias of `std` itself.
fn through_std(rest: &[TokenTree]) -> Vec<String> {
    let path = |module: &str| format!("a path through `std::{module}`, which does IO");
    match rest {
        [TokenTree::Ident(word), ..] if word == "as" => vec![alias_of_std()],
        [TokenTree::Punct(a), TokenTree::Punct(b), next, ..]
            if a.as_char() == ':' && a.spacing() == Spacing::Joint && b.as_char() == ':' =>
        {
            match next {
                TokenTree::Ident(module) => io_module(module).map(path).into_iter().collect(),
                TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => {
                    group_items(group.stream())
                        .iter()
                        .filter_map(|item| match item.as_slice() {
                            [TokenTree::Ident(word), TokenTree::Ident(alias), ..]
                                if word == "self" && alias == "as" =>
                            {
                                Some(alias_of_std())
                            }
                            [TokenTree::Ident(module), ..] => io_module(module).map(path),
                            _ => None,
                        })
                        .collect()
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

fn alias_of_std() -> String {
    "an alias of `std`, which hides the paths under it from the scan".to_owned()
}

fn io_module(ident: &proc_macro2::Ident) -> Option<&'static str> {
    let text = ident.to_string();
    let name = text.strip_prefix("r#").unwrap_or(&text);
    IO_MODULES.iter().copied().find(|module| *module == name)
}

/// A `use` group's items, split at its top-level commas.
fn group_items(tokens: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut items = vec![Vec::new()];
    for token in tokens {
        match &token {
            TokenTree::Punct(p) if p.as_char() == ',' => items.push(Vec::new()),
            _ => items.last_mut().expect("never empty").push(token),
        }
    }
    items
}

fn names_a_wall_lint(tokens: TokenStream) -> Option<String> {
    tokens.into_iter().find_map(|token| match token {
        TokenTree::Ident(ident) => {
            let text = ident.to_string();
            let name = text.strip_prefix("r#").unwrap_or(&text);
            WALL_LINTS.contains(&name).then(|| name.to_owned())
        }
        TokenTree::Group(group) => names_a_wall_lint(group.stream()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str) -> Vec<String> {
        scan(source)
            .expect("lexes")
            .into_iter()
            .map(|f| format!("{}: {}", f.line, f.what))
            .collect()
    }

    #[test]
    fn floats_clippy_misses_are_found() {
        assert_eq!(
            found("fn f() -> bool {\n    2.0_f64.sqrt() > 1.0\n}"),
            [
                "2: the float literal `2.0_f64`",
                "2: the float literal `1.0`"
            ]
        );
        assert_eq!(
            found("const X: bool = 1.5 > 2.;"),
            ["1: the float literal `1.5`", "1: the float literal `2.`"]
        );
        assert_eq!(found("let x = 1e3;"), ["1: the float literal `1e3`"]);
        assert_eq!(found("let x = 1_f32;"), ["1: the float literal `1_f32`"]);
        assert_eq!(found("let r = a..0.5;"), ["1: the float literal `0.5`"]);
        assert_eq!(
            found("let e = core::f64::consts::E;"),
            ["1: the float type `f64`"]
        );
        assert_eq!(found("fn g(x: f32) {}"), ["1: the float type `f32`"]);
        assert_eq!(
            found("let bits = core::r#f64::consts::E.ln().to_bits();"),
            ["1: the float type `f64`"]
        );
    }

    #[test]
    fn integers_and_text_are_not_floats() {
        let clean = r##"
            //! 1.5 in a doc comment, and f64
            /// 2.0 too
            // 3.0 in a line comment
            /* 4.0 in a /* nested */ block comment */
            fn f(pair: ((u8, u8), u8)) -> u8 {
                let _ = ("5.0", b"6.0", r#"7.0"#, c"8.0", '9', b'1');
                let _ = (0x1f32, 0b1, 0o7, 1usize, 10_000_u64, 1..2, 1..=2);
                let _ = 1.max(2);
                let _: &'static str = "";
                pair.0.1 + pair.1
            }
        "##;
        assert_eq!(found(clean), Vec::<String>::new());
    }

    #[test]
    fn stepping_around_the_walls_is_found() {
        assert_eq!(
            found("#[allow(clippy::float_arithmetic)]\nfn f() {}"),
            ["1: `allow` of the wall lint `float_arithmetic`"]
        );
        assert_eq!(
            found("#[cfg_attr(test, expect(clippy::as_conversions))]\nfn f() {}"),
            ["1: `expect` of the wall lint `as_conversions`"]
        );
        assert_eq!(
            found("#![warn(clippy::restriction)]"),
            ["1: `warn` of the wall lint `restriction`"]
        );
        assert_eq!(
            found("#[allow(warnings)]\nfn f() {}"),
            ["1: `allow` of the wall lint `warnings`"]
        );
        assert_eq!(
            found("include!(\"other.rs\");"),
            ["1: `include!`, which hides code from the scan"]
        );
        assert_eq!(
            found("#[path = \"elsewhere.rs\"]\nmod m;"),
            ["1: `#[path]`, which hides a file from the scan"]
        );
        assert_eq!(
            found("#[allow(dead_code)]\nfn f() { include_str!(\"x\"); }"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn io_is_found_however_it_is_named() {
        assert_eq!(
            found("fn f() {\n    let _ = std::fs::read_dir(\".\");\n}"),
            ["2: a path through `std::fs`, which does IO"]
        );
        assert_eq!(
            found("use std::env as e;\nfn f() { let _ = e::current_dir(); }"),
            ["1: a path through `std::env`, which does IO"]
        );
        assert_eq!(
            found("use std::{collections::BTreeMap, io::{self, Write}, r#thread};"),
            [
                "1: a path through `std::io`, which does IO",
                "1: a path through `std::thread`, which does IO"
            ]
        );
        assert_eq!(
            found("extern crate std as s;\nuse ::std::net::TcpStream;\nuse std::{self as t};"),
            [
                "1: an alias of `std`, which hides the paths under it from the scan",
                "2: a path through `std::net`, which does IO",
                "3: an alias of `std`, which hides the paths under it from the scan"
            ]
        );
        assert_eq!(
            found("fn f() { println!(\"x\"); dbg!(1); }"),
            [
                "1: `println!`, which writes to the terminal",
                "1: `dbg!`, which writes to the terminal"
            ]
        );
        let clean = "use std::collections::BTreeMap;\nuse std::fmt::Write as _;\n\
                     fn f(fs: u8, io: u8, print: u8) -> u8 { fs + io + print }";
        assert_eq!(found(clean), Vec::<String>::new());
    }

    #[test]
    fn unlexable_source_is_an_error() {
        assert!(scan("fn f() { \"unterminated }").is_err());
    }
}
