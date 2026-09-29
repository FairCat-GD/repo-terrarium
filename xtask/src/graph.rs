//! The crate-graph check: the dependency graph and crate roots that make the invariants compile
//! errors. The policy is data, checked by a pure function over a small model, so every rule can
//! be falsified by a unit test with a synthetic graph.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use cargo_metadata::semver::Version;
use cargo_metadata::{DependencyKind, Metadata, MetadataCommand};

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
];

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
}

#[derive(Debug, Clone)]
pub struct TargetRoot {
    /// For example `lib`, `bin`, `test` or `custom-build`.
    pub kind: String,
    pub path: String,
    /// Trimmed lines of the root file that start with `#![`.
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
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub rust_version: Option<Version>,
    /// Indices into `Workspace::packages`, every dependency kind.
    pub deps: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub members: Vec<Member>,
    /// The resolved graph: every package, workspace members included.
    pub packages: Vec<Package>,
    /// `rust-version` from `[workspace.package]`.
    pub msrv: Version,
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
        }
    }
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
        let lib = member.targets.iter().find(|t| t.kind == "lib");
        if !lib.is_some_and(|t| t.inner_attributes.contains(*attribute)) {
            out.push(format!("{} lib root lacks `{attribute}`", member.name));
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
    let metadata = MetadataCommand::new()
        .current_dir(root)
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
    Ok(Workspace {
        members,
        packages: resolved_packages(&metadata)?,
        msrv,
    })
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
                path: t.src_path.to_string(),
                inner_attributes: text
                    .lines()
                    .map(str::trim)
                    .filter(|l| l.starts_with("#!["))
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(Member {
        name: package.name.to_string(),
        deps,
        targets,
        inherited,
        lints_from_workspace,
    })
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
        let mut root = vec![FORBID_UNSAFE];
        match name {
            PLATFORM => root = vec!["#![allow(unsafe_code)]", REQUIRED_ROOT_ATTRIBUTES[0].1],
            TRUTH => root.push(REQUIRED_ROOT_ATTRIBUTES[1].1),
            _ => {}
        }
        Member {
            name: name.to_owned(),
            deps: deps
                .iter()
                .map(|(n, k)| Dep {
                    name: (*n).to_owned(),
                    kind: *k,
                })
                .collect(),
            targets: vec![TargetRoot {
                kind: "lib".to_owned(),
                path: format!("crates/{name}/src/lib.rs"),
                inner_attributes: attrs(&root),
            }],
            inherited: INHERITED_FIELDS.iter().map(|f| (*f).to_owned()).collect(),
            lints_from_workspace: true,
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
        });
        fails_with(&ws, "terrarium-truth must not depend on terrarium-compose");
    }

    #[test]
    fn compose_depending_on_store_fails() {
        let mut ws = workspace();
        member_mut(&mut ws, COMPOSE).deps.push(Dep {
            name: STORE.into(),
            kind: Kind::Normal,
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
            },
            Dep {
                name: "proptest".into(),
                kind: Kind::Dev,
            },
        ]);
        assert_eq!(check(&ws), Vec::<String>::new());
        member_mut(&mut ws, TRUTH).deps.push(Dep {
            name: "rand".into(),
            kind: Kind::Normal,
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
            "terrarium-platform lib root lacks `#![deny(clippy::undocumented",
        );
    }

    #[test]
    fn truth_needs_its_float_lint() {
        let mut ws = workspace();
        member_mut(&mut ws, TRUTH).targets[0].inner_attributes = attrs(&[FORBID_UNSAFE]);
        fails_with(
            &ws,
            "terrarium-truth lib root lacks `#![deny(clippy::float_arithmetic)]`",
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
