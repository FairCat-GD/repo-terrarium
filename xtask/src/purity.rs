//! The source scan behind truth's float wall (I-1).
//!
//! Clippy cannot see every float. `float_arithmetic` skips `const` and `static` bodies, and a
//! float that is only compared or passed to a method (`2.0_f64.sqrt() > 1.0`) names no type
//! path and does no arithmetic. A float can only enter truth through a float literal or a
//! float type's name, so this scan rejects both. It also rejects the ways a file could step
//! around the scan or the crate-level lint walls: a local `allow` of a wall lint, `include!`,
//! and `#[path]`.
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
                if name == "include"
                    && matches!(after, Some(TokenTree::Punct(p)) if p.as_char() == '!')
                {
                    findings.push(Finding {
                        line,
                        what: "`include!`, which hides code from the scan".to_owned(),
                    });
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
    fn unlexable_source_is_an_error() {
        assert!(scan("fn f() { \"unterminated }").is_err());
    }
}
