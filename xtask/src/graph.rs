//! The crate-graph check: the dependency graph and crate roots that make the invariants compile
//! errors, plus the scan of truth's sources behind its float wall (I-1). The policy is data,
//! checked by a pure function over a small model, so every rule can be falsified by a unit test
//! with a synthetic graph.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::purity;

use cargo_metadata::semver::Version;
use cargo_metadata::{CargoOpt, DependencyKind, Metadata, MetadataCommand, TargetKind};

const TRUTH: &str = "terrarium-truth";
const ADAPTERS: &str = "terrarium-adapters";
const STORE: &str = "terrarium-store";
const ENGINE: &str = "terrarium-engine";
const COMPOSE: &str = "terrarium-compose";
const PLATFORM: &str = "terrarium-platform";
const APP: &str = "terrarium-app";
const XTASK: &str = "xtask";

/// The workspace crates each member may depend on, for every dependency kind. `None` means any.
/// A member missing from this table fails the check, so a new crate cannot join the workspace
/// without a decision about its place in the graph.
const ALLOWED_EDGES: &[(&str, Option<&[&str]>)] = &[
    (TRUTH, Some(&[])),
    (ADAPTERS, Some(&[TRUTH])),
    (STORE, Some(&[TRUTH])),
    (ENGINE, Some(&[TRUTH, ADAPTERS, STORE])),
    (COMPOSE, Some(&[TRUTH])),
    (PLATFORM, Some(&[])),
    (
        APP,
        Some(&[TRUTH, ADAPTERS, STORE, ENGINE, COMPOSE, PLATFORM]),
    ),
    (XTASK, None),
];

/// Truth's only external dependencies, for normal and build dependencies.
const TRUTH_EXTERNAL: &[&str] = &["serde", "postcard", "indexmap", "sha2"];

/// The HTTP client, reachable only through its owner (I-6).
const HTTP_CLIENT: &str = "ureq";
const HTTP_OWNER: &str = ADAPTERS;

const FORBID_UNSAFE: &str = "#![forbid(unsafe_code)]";
/// The one crate allowed `unsafe` (I-9).
const UNSAFE_CRATE: &str = PLATFORM;

/// Inner attributes a crate's library root must carry.
const REQUIRED_ROOT_ATTRIBUTES: &[(&str, &str)] = &[
    (PLATFORM, "#![deny(clippy::undocumented_unsafe_blocks)]"),
    (TRUTH, "#![deny(clippy::float_arithmetic)]"),
    (TRUTH, "#![deny(clippy::as_conversions)]"),
    (TRUTH, "#![deny(clippy::disallowed_types)]"),
    (TRUTH, "#![deny(clippy::disallowed_methods)]"),
];

/// The feature that builds the debug window, which never ships in release builds: no member may
/// enable it by default, directly or through another feature, and no member may turn it on in a
/// workspace crate it depends on.
const DEV_FEATURE: &str = "dev";

/// Package fields every member inherits from `[workspace.package]`.
const INHERITED_FIELDS: &[&str] = &[
    "version",
    "edition",
    "rust-version",
    "license",
    "repository",
    "publish",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Normal,
    Build,
    Dev,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: Kind,
    /// The features the member turns on in it.
    pub features: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TargetRoot {
    /// For example `lib`, `bin`, `test` or `custom-build`.
    pub kind: String,
    /// Any library kind: `lib`, `rlib`, `cdylib`, `proc-macro` and the rest.
    pub library: bool,
    pub path: String,
    /// The root file's inner attributes (see `inner_attributes`).
    pub inner_attributes: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    pub deps: Vec<Dep>,
    pub targets: Vec<TargetRoot>,
    /// Fields of `INHERITED_FIELDS` the manifest sets with `.workspace = true`.
    pub inherited: BTreeSet<String>,
    pub lints_from_workspace: bool,
    /// The `[features]` table: each feature and what it turns on.
    pub features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub rust_version: Option<Version>,
    /// Indices into `Workspace::packages`, every dependency kind.
    pub deps: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Relative to the workspace root.
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub members: Vec<Member>,
    /// The resolved graph: every package, workspace members included.
    pub packages: Vec<Package>,
    /// `rust-version` from `[workspace.package]`.
    pub msrv: Version,
    /// Every `.rs` file in truth's package directory.
    pub truth_sources: Vec<SourceFile>,
}

/// Every rule the workspace breaks, one line each. Empty means the check passes.
pub fn check(ws: &Workspace) -> Vec<String> {
    let mut violations = Vec::new();
    let names: BTreeSet<&str> = ws.members.iter().map(|m| m.name.as_str()).collect();

    for member in &ws.members {
        check_edges(member, &names, &mut violations);
        check_manifest(member, &mut violations);
        check_roots(member, &mut violations);
        if member.name == TRUTH {
            check_truth_externals(member, &names, &mut violations);
            check_truth_targets(member, &mut violations);
        }
    }
    check_truth_sources(&ws.truth_sources, &mut violations);
    check_http_client(ws, &mut violations);
    check_msrv(ws, &mut violations);
    violations
}

fn check_edges(member: &Member, names: &BTreeSet<&str>, out: &mut Vec<String>) {
    let Some((_, allowed)) = ALLOWED_EDGES.iter().find(|(name, _)| *name == member.name) else {
        out.push(format!("{} is not in the crate-graph policy", member.name));
        return;
    };
    let Some(allowed) = allowed else { return };
    for dep in member
        .deps
        .iter()
        .filter(|d| names.contains(d.name.as_str()))
    {
        if !allowed.contains(&dep.name.as_str()) {
            out.push(format!(
                "{} must not depend on {} ({:?} dependency)",
                member.name, dep.name, dep.kind
            ));
        }
    }
}

fn check_manifest(member: &Member, out: &mut Vec<String>) {
    for field in INHERITED_FIELDS {
        if !member.inherited.contains(*field) {
            out.push(format!(
                "{} must set `{field}.workspace = true`",
                member.name
            ));
        }
    }
    if !member.lints_from_workspace {
        out.push(format!(
            "{} must set `[lints] workspace = true`",
            member.name
        ));
    }
    let enables_dev = |entry: &String| {
        entry == DEV_FEATURE
            || entry
                .rsplit_once('/')
                .is_some_and(|(_, feature)| feature == DEV_FEATURE)
    };
    if default_features(&member.features).iter().any(enables_dev) {
        out.push(format!(
            "{} must not enable `{DEV_FEATURE}` by default: the debug window never ships in \
             release builds",
            member.name
        ));
    }
    for dep in &member.deps {
        if dep.features.iter().any(enables_dev) {
            out.push(format!(
                "{} must not turn on `{DEV_FEATURE}` in {}: the debug window never ships in \
                 release builds",
                member.name, dep.name
            ));
        }
    }
}

/// Every feature `default` turns on, followed through the features it names.
fn default_features(features: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut on = BTreeSet::new();
    let mut next = vec!["default".to_owned()];
    while let Some(feature) = next.pop() {
        if let Some(enables) = features.get(&feature) {
            next.extend(enables.iter().filter(|f| !on.contains(*f)).cloned());
        }
        on.insert(feature);
    }
    on
}

fn check_roots(member: &Member, out: &mut Vec<String>) {
    if member.name != UNSAFE_CRATE {
        for target in &member.targets {
            if !target.inner_attributes.contains(FORBID_UNSAFE) {
                out.push(format!(
                    "{} {} root {} lacks `{FORBID_UNSAFE}` (I-9)",
                    member.name, target.kind, target.path
                ));
            }
        }
    }
    for (crate_name, attribute) in REQUIRED_ROOT_ATTRIBUTES {
        if member.name != *crate_name {
            continue;
        }
        let lib = member.targets.iter().find(|t| t.library);
        if !lib.is_some_and(|t| t.inner_attributes.contains(*attribute)) {
            out.push(format!("{} library root lacks `{attribute}`", member.name));
        }
    }
}

fn check_truth_externals(member: &Member, names: &BTreeSet<&str>, out: &mut Vec<String>) {
    for dep in &member.deps {
        let linked = matches!(dep.kind, Kind::Normal | Kind::Build);
        let external = !names.contains(dep.name.as_str());
        if linked && external && !TRUTH_EXTERNAL.contains(&dep.name.as_str()) {
            out.push(format!(
                "{TRUTH} must not depend on {} (allowed: {})",
                dep.name,
                TRUTH_EXTERNAL.join(", ")
            ));
        }
    }
}

/// Truth is a single library, so its crate-level lint walls cover all of its code: a test,
/// bench, example or build-script target would be a crate of its own without them.
fn check_truth_targets(member: &Member, out: &mut Vec<String>) {
    for target in member.targets.iter().filter(|t| !t.library) {
        out.push(format!(
            "{TRUTH} has a {} target at {}; truth is one library, so its lint walls reach all of \
             it (I-1)",
            target.kind, target.path
        ));
    }
}

fn check_truth_sources(sources: &[SourceFile], out: &mut Vec<String>) {
    for source in sources {
        match purity::scan(&source.text) {
            Ok(findings) => out.extend(findings.into_iter().map(|finding| {
                format!(
                    "{TRUTH} {}:{} has {} (I-1)",
                    source.path, finding.line, finding.what
                )
            })),
            Err(err) => out.push(format!("{TRUTH} {} does not lex: {err}", source.path)),
        }
    }
}

/// Walks the resolved graph from every member except the owner, without entering the owner.
/// Reaching the client means some other path links it.
fn check_http_client(ws: &Workspace, out: &mut Vec<String>) {
    for member in ws.members.iter().filter(|m| m.name != HTTP_OWNER) {
        let Some(start) = ws.packages.iter().position(|p| p.name == member.name) else {
            continue;
        };
        let mut seen = BTreeSet::from([start]);
        let mut stack = vec![start];
        while let Some(index) = stack.pop() {
            for &next in &ws.packages[index].deps {
                let package = &ws.packages[next];
                if package.name == HTTP_OWNER || !seen.insert(next) {
                    continue;
                }
                if package.name == HTTP_CLIENT {
                    out.push(format!(
                        "{} reaches {HTTP_CLIENT} other than through {HTTP_OWNER} (I-6)",
                        member.name
                    ));
                    stack.clear();
                    break;
                }
                stack.push(next);
            }
        }
    }
}

fn check_msrv(ws: &Workspace, out: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for package in &ws.packages {
        if let Some(needed) = &package.rust_version
            && *needed > ws.msrv
            && seen.insert((&package.name, needed))
        {
            out.push(format!(
                "{} needs Rust {needed}, above the workspace rust-version {}",
                package.name, ws.msrv
            ));
        }
    }
}

/// Builds the model from `cargo metadata` and the files it names.
pub fn load(root: &Path) -> Result<Workspace, String> {
    // The graph is resolved for the shipping targets only, the same list cargo-deny checks, so
    // a crate that never ships (Linux window-system crates) cannot fail a rule.
    let deny = read_toml(&root.join("deny.toml"))?;
    let targets = deny
        .get("graph")
        .and_then(|g| g.get("targets"))
        .and_then(toml::Value::as_array)
        .ok_or("deny.toml has no [graph] targets")?;
    let mut filter = Vec::new();
    for target in targets {
        let triple = target
            .as_str()
            .ok_or("deny.toml [graph] targets must be strings")?;
        filter.extend(["--filter-platform".to_owned(), triple.to_owned()]);
    }
    // Every feature, so a dependency only a dev build links is held to the same rules.
    let metadata = MetadataCommand::new()
        .current_dir(root)
        .features(CargoOpt::AllFeatures)
        .other_options(filter)
        .exec()
        .map_err(|e| format!("cargo metadata failed: {e}"))?;
    let root_manifest = read_toml(&root.join("Cargo.toml"))?;
    let msrv = root_manifest
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("rust-version"))
        .and_then(toml::Value::as_str)
        .ok_or("the root Cargo.toml has no [workspace.package] rust-version")?;
    let msrv = parse_rust_version(msrv)?;

    let members = metadata
        .workspace_packages()
        .into_iter()
        .map(member_from)
        .collect::<Result<Vec<_>, _>>()?;
    let truth_dir = metadata
        .workspace_packages()
        .into_iter()
        .find(|p| p.name.as_str() == TRUTH)
        .and_then(|p| p.manifest_path.parent())
        .ok_or("cargo metadata has no terrarium-truth")?
        .as_std_path()
        .to_path_buf();
    let mut truth_sources = Vec::new();
    collect_sources(root, &truth_dir, &mut truth_sources)?;
    Ok(Workspace {
        members,
        packages: resolved_packages(&metadata)?,
        msrv,
        truth_sources,
    })
}

/// Every `.rs` file under `dir`, skipping build output, sorted so reports are stable.
fn collect_sources(root: &Path, dir: &Path, out: &mut Vec<SourceFile>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    let mut paths: Vec<_> = entries
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                collect_sources(root, &path, out)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let relative = path.strip_prefix(root).unwrap_or(&path);
            out.push(SourceFile {
                path: relative.display().to_string(),
                text,
            });
        }
    }
    Ok(())
}

fn member_from(package: &cargo_metadata::Package) -> Result<Member, String> {
    let manifest = read_toml(package.manifest_path.as_std_path())?;
    let package_table = manifest.get("package");
    let inherited = INHERITED_FIELDS
        .iter()
        .filter(|field| {
            package_table
                .and_then(|p| p.get(**field))
                .and_then(|f| f.get("workspace"))
                .and_then(toml::Value::as_bool)
                == Some(true)
        })
        .map(|field| (*field).to_owned())
        .collect();
    let lints_from_workspace = manifest
        .get("lints")
        .and_then(|l| l.get("workspace"))
        .and_then(toml::Value::as_bool)
        == Some(true);

    let deps = package
        .dependencies
        .iter()
        .map(|d| Dep {
            name: d.name.clone(),
            kind: match d.kind {
                DependencyKind::Development => Kind::Dev,
                DependencyKind::Build => Kind::Build,
                _ => Kind::Normal,
            },
            features: d.features.clone(),
        })
        .collect();

    let targets = package
        .targets
        .iter()
        .map(|t| {
            let text = fs::read_to_string(&t.src_path)
                .map_err(|e| format!("cannot read {}: {e}", t.src_path))?;
            Ok(TargetRoot {
                kind: t.kind.first().map_or_else(String::new, ToString::to_string),
                library: t.kind.iter().any(|k| {
                    matches!(
                        k,
                        TargetKind::Lib
                            | TargetKind::RLib
                            | TargetKind::DyLib
                            | TargetKind::CDyLib
                            | TargetKind::StaticLib
                            | TargetKind::ProcMacro
                    )
                }),
                path: t.src_path.to_string(),
                inner_attributes: inner_attributes(&text),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(Member {
        name: package.name.to_string(),
        deps,
        targets,
        inherited,
        lints_from_workspace,
        features: package
            .features
            .iter()
            .map(|(name, enables)| (name.clone(), enables.clone()))
            .collect(),
    })
}

/// The inner attributes at the top of a crate root, each trimmed and without a trailing line
/// comment. Only the leading run of attributes, comments and blank lines counts, so the text of
/// an attribute inside a string or a later block comment is not mistaken for one.
fn inner_attributes(text: &str) -> BTreeSet<String> {
    let mut attributes = BTreeSet::new();
    let mut in_block_comment = false;
    for line in text.lines().map(str::trim) {
        if in_block_comment || line.starts_with("/*") {
            in_block_comment = !line.contains("*/");
            continue;
        }
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if !line.starts_with("#![") {
            break;
        }
        let code = line.find("//").map_or(line, |comment| &line[..comment]);
        attributes.insert(code.trim_end().to_owned());
    }
    attributes
}

fn resolved_packages(metadata: &Metadata) -> Result<Vec<Package>, String> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or("cargo metadata returned no resolve graph")?;
    let index: BTreeMap<&str, usize> = metadata
        .packages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.repr.as_str(), i))
        .collect();
    let mut packages: Vec<Package> = metadata
        .packages
        .iter()
        .map(|p| Package {
            name: p.name.to_string(),
            rust_version: p.rust_version.clone(),
            deps: Vec::new(),
        })
        .collect();
    for node in &resolve.nodes {
        let Some(&from) = index.get(node.id.repr.as_str()) else {
            continue;
        };
        packages[from].deps = node
            .deps
            .iter()
            .filter_map(|d| index.get(d.pkg.repr.as_str()).copied())
            .collect();
    }
    Ok(packages)
}

fn read_toml(path: &Path) -> Result<toml::Table, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    text.parse()
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

/// `rust-version` may omit the patch (`1.95`); semver needs all three parts.
fn parse_rust_version(text: &str) -> Result<Version, String> {
    let padded = match text.matches('.').count() {
        0 => format!("{text}.0.0"),
        1 => format!("{text}.0"),
        _ => text.to_owned(),
    };
    Version::parse(&padded).map_err(|e| format!("bad rust-version {text:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(lines: &[&str]) -> BTreeSet<String> {
        lines.iter().map(|l| (*l).to_owned()).collect()
    }

    fn member(name: &str, deps: &[(&str, Kind)]) -> Member {
        let mut root = if name == PLATFORM {
            vec!["#![allow(unsafe_code)]"]
        } else {
            vec![FORBID_UNSAFE]
        };
        root.extend(
            REQUIRED_ROOT_ATTRIBUTES
                .iter()
                .filter(|(crate_name, _)| *crate_name == name)
                .map(|(_, attribute)| *attribute),
        );
        Member {
            name: name.to_owned(),
            deps: deps
                .iter()
                .map(|(n, k)| Dep {
                    name: (*n).to_owned(),
                    kind: *k,
                    features: Vec::new(),
                })
                .collect(),
            targets: vec![TargetRoot {
                kind: "lib".to_owned(),
                library: true,
                path: format!("crates/{name}/src/lib.rs"),
                inner_attributes: attrs(&root),
            }],
            inherited: INHERITED_FIELDS.iter().map(|f| (*f).to_owned()).collect(),
            lints_from_workspace: true,
            features: BTreeMap::new(),
        }
    }

    /// The policy's graph, which must pass.
    fn workspace() -> Workspace {
        use Kind::Normal;
        let members = vec![
            member(TRUTH, &[]),
            member(ADAPTERS, &[(TRUTH, Normal)]),
            member(STORE, &[(TRUTH, Normal)]),
            member(
                ENGINE,
                &[(TRUTH, Normal), (ADAPTERS, Normal), (STORE, Normal)],
            ),
            member(COMPOSE, &[(TRUTH, Normal)]),
            member(PLATFORM, &[]),
            member(
                APP,
                &[
                    (TRUTH, Normal),
                    (ADAPTERS, Normal),
                    (STORE, Normal),
                    (ENGINE, Normal),
                    (COMPOSE, Normal),
                    (PLATFORM, Normal),
                ],
            ),
            member(XTASK, &[("cargo_metadata", Normal)]),
        ];
        let mut ws = Workspace {
            members,
            packages: Vec::new(),
            msrv: Version::new(1, 95, 0),
            truth_sources: vec![SourceFile {
                path: "crates/terrarium-truth/src/lib.rs".into(),
                text: "#![deny(clippy::float_arithmetic)]\npub fn half(x: u32) -> u32 { x / 2 }\n"
                    .into(),
            }],
        };
        let edges: Vec<(String, Vec<String>)> = ws
            .members
            .iter()
            .map(|m| {
                (
                    m.name.clone(),
                    m.deps.iter().map(|d| d.name.clone()).collect(),
                )
            })
            .collect();
        for (name, deps) in edges {
            for dep in deps {
                add_edge(&mut ws, &name, &dep);
            }
        }
        ws
    }

    fn package(ws: &mut Workspace, name: &str) -> usize {
        if let Some(i) = ws.packages.iter().position(|p| p.name == name) {
            return i;
        }
        ws.packages.push(Package {
            name: name.to_owned(),
            rust_version: None,
            deps: Vec::new(),
        });
        ws.packages.len() - 1
    }

    fn add_edge(ws: &mut Workspace, from: &str, to: &str) {
        let (from, to) = (package(ws, from), package(ws, to));
        ws.packages[from].deps.push(to);
    }

    fn member_mut<'a>(ws: &'a mut Workspace, name: &str) -> &'a mut Member {
        ws.members.iter_mut().find(|m| m.name == name).unwrap()
    }

    fn fails_with(ws: &Workspace, needle: &str) {
        let violations = check(ws);
        assert!(
            violations.iter().any(|v| v.contains(needle)),
            "expected a violation containing {needle:?}, got {violations:#?}"
        );
    }

    #[test]
    fn the_policy_graph_passes() {
        assert_eq!(check(&workspace()), Vec::<String>::new());
    }

    #[test]
    fn truth_depending_on_compose_fails_even_as_a_dev_dependency() {
        let mut ws = workspace();
        member_mut(&mut ws, TRUTH).deps.push(Dep {
            name: COMPOSE.into(),
            kind: Kind::Dev,
            features: Vec::new(),
        });
        fails_with(&ws, "terrarium-truth must not depend on terrarium-compose");
    }

    #[test]
    fn compose_depending_on_store_fails() {
        let mut ws = workspace();
        member_mut(&mut ws, COMPOSE).deps.push(Dep {
            name: STORE.into(),
            kind: Kind::Normal,
            features: Vec::new(),
        });
        fails_with(&ws, "terrarium-compose must not depend on terrarium-store");
    }

    #[test]
    fn a_member_outside_the_policy_fails() {
        let mut ws = workspace();
        ws.members.push(member("terrarium-extra", &[]));
        fails_with(&ws, "terrarium-extra is not in the crate-graph policy");
    }

    #[test]
    fn truth_may_link_only_its_listed_externals() {
        let mut ws = workspace();
        member_mut(&mut ws, TRUTH).deps.extend([
            Dep {
                name: "serde".into(),
                kind: Kind::Normal,
                features: Vec::new(),
            },
            Dep {
                name: "proptest".into(),
                kind: Kind::Dev,
                features: Vec::new(),
            },
        ]);
        assert_eq!(check(&ws), Vec::<String>::new());
        member_mut(&mut ws, TRUTH).deps.push(Dep {
            name: "rand".into(),
            kind: Kind::Normal,
            features: Vec::new(),
        });
        fails_with(&ws, "terrarium-truth must not depend on rand");
    }

    #[test]
    fn the_http_client_is_reachable_only_through_adapters() {
        let mut ws = workspace();
        add_edge(&mut ws, ADAPTERS, HTTP_CLIENT);
        assert_eq!(
            check(&ws),
            Vec::<String>::new(),
            "engine and app reach it via adapters"
        );

        add_edge(&mut ws, STORE, HTTP_CLIENT);
        fails_with(&ws, "terrarium-store reaches ureq");
    }

    #[test]
    fn a_transitive_http_client_fails() {
        let mut ws = workspace();
        add_edge(&mut ws, COMPOSE, "some-helper");
        add_edge(&mut ws, "some-helper", HTTP_CLIENT);
        fails_with(&ws, "terrarium-compose reaches ureq");
    }

    #[test]
    fn every_target_root_outside_platform_forbids_unsafe() {
        let mut ws = workspace();
        member_mut(&mut ws, APP).targets.push(TargetRoot {
            kind: "test".into(),
            library: false,
            path: "crates/terrarium-app/tests/cli.rs".into(),
            inner_attributes: BTreeSet::new(),
        });
        fails_with(
            &ws,
            "terrarium-app test root crates/terrarium-app/tests/cli.rs lacks",
        );
    }

    #[test]
    fn platform_needs_its_unsafe_documentation_lint() {
        let mut ws = workspace();
        member_mut(&mut ws, PLATFORM).targets[0].inner_attributes =
            attrs(&["#![allow(unsafe_code)]"]);
        fails_with(
            &ws,
            "terrarium-platform library root lacks `#![deny(clippy::undocumented",
        );
    }

    #[test]
    fn truth_needs_its_float_lint() {
        let mut ws = workspace();
        member_mut(&mut ws, TRUTH).targets[0].inner_attributes = attrs(&[FORBID_UNSAFE]);
        fails_with(
            &ws,
            "terrarium-truth library root lacks `#![deny(clippy::float_arithmetic)]`",
        );
    }

    #[test]
    fn truth_needs_its_cast_lint() {
        let mut ws = workspace();
        member_mut(&mut ws, TRUTH).targets[0]
            .inner_attributes
            .remove("#![deny(clippy::as_conversions)]");
        fails_with(
            &ws,
            "terrarium-truth library root lacks `#![deny(clippy::as_conversions)]`",
        );
    }

    #[test]
    fn truth_denies_its_disallowed_lists() {
        for attribute in [
            "#![deny(clippy::disallowed_types)]",
            "#![deny(clippy::disallowed_methods)]",
        ] {
            let mut ws = workspace();
            member_mut(&mut ws, TRUTH).targets[0]
                .inner_attributes
                .remove(attribute);
            fails_with(
                &ws,
                &format!("terrarium-truth library root lacks `{attribute}`"),
            );
        }
    }

    #[test]
    fn truth_is_one_library() {
        let mut ws = workspace();
        member_mut(&mut ws, TRUTH).targets.push(TargetRoot {
            kind: "test".into(),
            library: false,
            path: "crates/terrarium-truth/tests/extra.rs".into(),
            inner_attributes: attrs(&[FORBID_UNSAFE]),
        });
        fails_with(
            &ws,
            "terrarium-truth has a test target at crates/terrarium-truth/tests/extra.rs",
        );
    }

    #[test]
    fn truth_sources_are_scanned() {
        let mut ws = workspace();
        ws.truth_sources.push(SourceFile {
            path: "crates/terrarium-truth/src/tuning.rs".into(),
            text: "pub const SPLIT: bool = 0.5_f64 < 1.0;\n".into(),
        });
        fails_with(
            &ws,
            "terrarium-truth crates/terrarium-truth/src/tuning.rs:1 has the float literal `0.5_f64`",
        );
        ws.truth_sources.push(SourceFile {
            path: "crates/terrarium-truth/src/broken.rs".into(),
            text: "fn f() { \"unterminated }".into(),
        });
        fails_with(
            &ws,
            "terrarium-truth crates/terrarium-truth/src/broken.rs does not lex",
        );
    }

    #[test]
    fn manifests_inherit_the_workspace_package_and_lints() {
        let mut ws = workspace();
        member_mut(&mut ws, STORE).inherited.remove("edition");
        member_mut(&mut ws, ENGINE).lints_from_workspace = false;
        fails_with(&ws, "terrarium-store must set `edition.workspace = true`");
        fails_with(&ws, "terrarium-engine must set `[lints] workspace = true`");
    }

    fn features(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(name, on)| {
                (
                    (*name).to_owned(),
                    on.iter().map(|f| (*f).to_owned()).collect(),
                )
            })
            .collect()
    }

    const DEV_BY_DEFAULT: &str = "terrarium-app must not enable `dev` by default: the debug window never ships in \
         release builds";

    #[test]
    fn no_member_builds_the_debug_window_by_default() {
        let mut ws = workspace();
        member_mut(&mut ws, APP).features = features(&[("default", &[]), ("dev", &["dep:egui"])]);
        assert_eq!(check(&ws), Vec::<String>::new());
        member_mut(&mut ws, APP).features = features(&[("default", &["dev"]), ("dev", &[])]);
        fails_with(&ws, DEV_BY_DEFAULT);
    }

    #[test]
    fn nor_through_another_feature_or_a_dependency() {
        let mut ws = workspace();
        member_mut(&mut ws, APP).features =
            features(&[("default", &["ui"]), ("ui", &["dev"]), ("dev", &[])]);
        fails_with(&ws, DEV_BY_DEFAULT);

        let mut ws = workspace();
        member_mut(&mut ws, APP).features = features(&[("default", &["terrarium-compose/dev"])]);
        fails_with(&ws, DEV_BY_DEFAULT);

        let mut ws = workspace();
        member_mut(&mut ws, XTASK).deps.push(Dep {
            name: APP.to_owned(),
            kind: Kind::Normal,
            features: vec!["dev".to_owned()],
        });
        fails_with(
            &ws,
            "xtask must not turn on `dev` in terrarium-app: the debug window never ships in \
             release builds",
        );
    }

    #[test]
    fn a_library_of_any_kind_carries_the_required_attributes() {
        let mut ws = workspace();
        member_mut(&mut ws, PLATFORM).targets[0].kind = "rlib".into();
        assert_eq!(check(&ws), Vec::<String>::new());
    }

    #[test]
    fn inner_attributes_are_read_from_the_top_of_the_root_only() {
        let root = "//! Crate docs.\n\
            /* a block\n   comment */\n\
            #![forbid(unsafe_code)] // I-9\n\
            #![doc = include_str!(\"../README.md\")]\n\
            \n\
            const S: &str = r\"\n\
            #![deny(clippy::float_arithmetic)]\n\
            \";\n";
        assert_eq!(
            inner_attributes(root),
            attrs(&[FORBID_UNSAFE, "#![doc = include_str!(\"../README.md\")]"])
        );
    }

    #[test]
    fn an_attribute_inside_a_leading_block_comment_does_not_count() {
        let root = "/*\n#![forbid(unsafe_code)]\n*/\npub fn f() {}\n";
        assert_eq!(inner_attributes(root), BTreeSet::new());
    }

    #[test]
    fn a_dependency_above_the_msrv_fails() {
        let mut ws = workspace();
        add_edge(&mut ws, APP, "new-crate");
        let index = package(&mut ws, "new-crate");
        ws.packages[index].rust_version = Some(Version::new(1, 95, 0));
        assert_eq!(check(&ws), Vec::<String>::new());
        ws.packages[index].rust_version = Some(Version::new(1, 96, 0));
        fails_with(&ws, "new-crate needs Rust 1.96.0");
    }

    #[test]
    fn rust_versions_without_a_patch_parse() {
        assert_eq!(parse_rust_version("1.95"), Ok(Version::new(1, 95, 0)));
        assert_eq!(parse_rust_version("1.95.2"), Ok(Version::new(1, 95, 2)));
    }
}
