//! `cargo xtask assets`: validates the delivered art package, then packs the runtime atlas and
//! catalog that `terrarium-compose` embeds.
//!
//! The package is read into memory once. The checks run on that copy, so tests mutate it and
//! never write to disk.
//! - Checks 1 to 6 all run, so a delivery shows every problem at once. The exception is a
//!   biome whose manifests fail check 2: checks 3 to 6 need its sprite table, so they are
//!   skipped for it.
//! - Packing and check 7 run only once 1 to 6 pass. So a broken package fails at its own
//!   check, and never writes output.
//! - `--check` compares the regenerated files with the committed ones byte for byte. Plain
//!   `assets` rewrites only the files whose bytes differ, so an unchanged run rebuilds
//!   nothing.

mod animation;
mod build;
mod manifest;
mod pack;
mod package;
mod pixels;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::process::ExitCode;

use terrarium_compose::catalog::Catalog;
use terrarium_compose::variants;

use pixels::Indexed;

pub use package::Package;

/// The art package, relative to the workspace root.
pub const ART_DIR: &str = "art-assets";
/// Where the generated files are committed, relative to the workspace root.
pub const GENERATED_DIR: &str = "crates/terrarium-compose/assets/generated";

/// The asset checks, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Check {
    /// 1: every file matches `PACKAGE-CONTENTS.json`.
    Package,
    /// 2: the active manifests, their counts and revisions, unique IDs.
    Manifests,
    /// 3: rects inside their atlas, atlas pixels equal to the individual PNGs.
    Pixels,
    /// 4: every shared index at most 32 and every shared file carrying the biome palette; art
    /// with its own colours carrying its listed ones; the slots fitting an atlas byte.
    Palette,
    /// 5: every sequence has frames and durations, and every terminal resolves.
    Animations,
    /// 6: every `SceneInput` variant has art.
    Variants,
    /// 7: the generated files equal the committed ones.
    Generated,
}

impl Check {
    pub const ALL: [Check; 7] = [
        Check::Package,
        Check::Manifests,
        Check::Pixels,
        Check::Palette,
        Check::Animations,
        Check::Variants,
        Check::Generated,
    ];

    pub fn number(self) -> usize {
        Check::ALL
            .iter()
            .position(|&c| c == self)
            .map_or(0, |i| i + 1)
    }

    fn title(self) -> &'static str {
        match self {
            Check::Package => "package contents",
            Check::Manifests => "manifests",
            Check::Pixels => "pixels",
            Check::Palette => "palette",
            Check::Animations => "animations",
            Check::Variants => "variants",
            Check::Generated => "generated files",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub check: Check,
    pub message: String,
}

impl Failure {
    pub fn new(check: Check, message: impl Into<String>) -> Failure {
        Failure {
            check,
            message: message.into(),
        }
    }
}

/// What a run found, and the generated files when checks 1 to 6 passed.
#[derive(Debug, Default)]
pub struct Report {
    pub failures: Vec<Failure>,
    /// Checks that could not run because an earlier one failed, with what they skipped: a
    /// biome's name, or `None` for the whole package.
    pub skipped: BTreeMap<Check, BTreeSet<Option<String>>>,
    /// Paths relative to the generated folder, such as `temperate/atlas.bin`.
    pub output: Option<BTreeMap<String, Vec<u8>>>,
    /// One line per biome: its counts and atlas size.
    pub notes: Vec<String>,
}

impl Report {
    fn skip(&mut self, checks: impl IntoIterator<Item = Check>, biome: Option<&str>) {
        for check in checks {
            self.skipped
                .entry(check)
                .or_default()
                .insert(biome.map(str::to_owned));
        }
    }
}

/// A biome that got through check 2, with what checks 3 to 6 produced for it.
struct Stage {
    name: String,
    images: BTreeMap<String, Indexed>,
    catalog: Catalog,
    unresolved: Vec<String>,
}

/// Runs checks 1 to 6 and, when they pass, packs and encodes every biome.
pub fn build(package: &Package) -> Report {
    let mut report = Report::default();
    let (contents, failures) = package::check(package);
    report.failures.extend(failures);
    let Some(contents) = contents else {
        report.skip(Check::ALL.into_iter().skip(1), None);
        return report;
    };

    let mut biomes = Vec::new();
    let mut all_loaded = true;
    for name in &contents.biomes {
        match manifest::load(package, name) {
            Ok(biome) => biomes.push(biome),
            Err(failures) => {
                all_loaded = false;
                report.failures.extend(failures);
                report.skip(
                    [
                        Check::Pixels,
                        Check::Palette,
                        Check::Animations,
                        Check::Variants,
                    ],
                    Some(name),
                );
            }
        }
    }
    if all_loaded {
        report
            .failures
            .extend(manifest::check_totals(&contents, &biomes));
    }

    let mut stages = Vec::new();
    for biome in &biomes {
        let (images, palette, failures) = pixels::check(package, biome);
        report.failures.extend(failures);
        let (drafts, failures) = animation::build(biome);
        report.failures.extend(failures);
        let (catalog, unresolved) = build::catalog(biome, &drafts, palette);
        report.failures.extend(
            variants::missing(&catalog)
                .into_iter()
                .map(|gap| Failure::new(Check::Variants, format!("{}: {gap}", biome.name))),
        );
        stages.push(Stage {
            name: biome.name.clone(),
            images,
            catalog,
            unresolved,
        });
    }

    if !report.failures.is_empty() {
        report.skip([Check::Generated], None);
        return report;
    }
    let mut output = BTreeMap::new();
    for stage in stages {
        match emit(stage, &mut report.notes) {
            Ok(files) => output.extend(files),
            Err(failures) => report.failures.extend(failures),
        }
    }
    if report.failures.is_empty() {
        report.output = Some(output);
    }
    report
}

/// Packs a biome's pixels, fills in the rects, and encodes the pair. The packed atlas is read
/// back against every sprite's own PNG, and the encoded catalog must decode to itself.
fn emit(stage: Stage, notes: &mut Vec<String>) -> Result<BTreeMap<String, Vec<u8>>, Vec<Failure>> {
    let fail = |message: String| {
        vec![Failure::new(
            Check::Generated,
            format!("{}: {message}", stage.name),
        )]
    };
    if !stage.unresolved.is_empty() {
        return Err(stage
            .unresolved
            .iter()
            .map(|u| Failure::new(Check::Generated, u.clone()))
            .collect());
    }
    let mut catalog = stage.catalog;
    let images: Option<Vec<&Indexed>> = catalog
        .sprites
        .iter()
        .map(|s| stage.images.get(&s.id))
        .collect();
    let Some(images) = images else {
        return Err(fail("a sprite has no pixels".into()));
    };
    let packed = pack::pack(&images).map_err(fail)?;
    for (sprite, rect) in catalog.sprites.iter_mut().zip(&packed.rects) {
        sprite.rect = *rect;
    }
    catalog.atlas = packed.size;
    let atlas = Indexed {
        width: u32::from(packed.size.w),
        height: u32::from(packed.size.h),
        pixels: packed.atlas,
    };
    let misplaced: Vec<Failure> = catalog
        .sprites
        .iter()
        .zip(&images)
        .filter(|(sprite, image)| atlas.region(sprite.rect).as_ref() != Some(&image.pixels))
        .map(|(sprite, _)| {
            Failure::new(
                Check::Pixels,
                format!("the packed atlas misplaces {}", sprite.id),
            )
        })
        .collect();
    if !misplaced.is_empty() {
        return Err(misplaced);
    }
    let bytes = catalog.encode();
    match Catalog::decode(&bytes, &atlas.pixels) {
        Ok(decoded) if decoded == catalog => {}
        Ok(_) => return Err(fail("the catalog does not decode to itself".into())),
        Err(err) => return Err(fail(format!("the catalog does not decode: {err}"))),
    }
    notes.push(format!(
        "{}: {} sprites, {} distinct images in a {}×{} atlas, {} animations",
        stage.name,
        catalog.sprites.len(),
        packed.unique,
        packed.size.w,
        packed.size.h,
        catalog.animations.len()
    ));
    Ok(BTreeMap::from([
        (format!("{}/catalog.bin", stage.name), bytes),
        (format!("{}/atlas.bin", stage.name), atlas.pixels),
    ]))
}

/// Check 7: the generated files against the committed ones, byte for byte.
pub fn compare(
    output: &BTreeMap<String, Vec<u8>>,
    committed: &BTreeMap<String, Vec<u8>>,
) -> Vec<Failure> {
    let fix = "run `cargo xtask assets`";
    let mut failures = Vec::new();
    for (path, bytes) in output {
        match committed.get(path) {
            None => failures.push(Failure::new(
                Check::Generated,
                format!("{path} is not committed; {fix}"),
            )),
            Some(old) if old != bytes => {
                let at = old
                    .iter()
                    .zip(bytes)
                    .position(|(a, b)| a != b)
                    .unwrap_or(old.len().min(bytes.len()));
                failures.push(Failure::new(
                    Check::Generated,
                    format!(
                        "{path} differs from the regenerated file at byte {at} (committed {} bytes, regenerated {}); {fix}",
                        old.len(),
                        bytes.len()
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    for path in committed.keys().filter(|path| !output.contains_key(*path)) {
        failures.push(Failure::new(
            Check::Generated,
            format!("{path} is committed but no longer generated; {fix}"),
        ));
    }
    failures
}

/// The committed generated files, by path relative to their folder.
fn read_generated(dir: &Path) -> io::Result<BTreeMap<String, Vec<u8>>> {
    if !dir.exists() {
        return Ok(BTreeMap::new());
    }
    package::read_tree(dir, |path| path.rsplit('/').next() == Some(".DS_Store"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Regenerate and write what changed.
    Write,
    /// Regenerate and compare, writing nothing.
    Check,
}

pub fn run(root: &Path, mode: Mode) -> ExitCode {
    if validate(root, mode) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The verify stage: `assets --check`.
pub fn check(root: &Path) -> bool {
    validate(root, Mode::Check)
}

fn validate(root: &Path, mode: Mode) -> bool {
    let package = match Package::read(&root.join(ART_DIR)) {
        Ok(package) => package,
        Err(err) => {
            println!("assets: cannot read {ART_DIR}: {err}");
            return false;
        }
    };
    let generated = root.join(GENERATED_DIR);
    let committed = match read_generated(&generated) {
        Ok(committed) => committed,
        Err(err) => {
            println!("assets: cannot read {GENERATED_DIR}: {err}");
            return false;
        }
    };
    let mut report = build(&package);
    let mut written = Vec::new();
    if let Some(output) = &report.output {
        match mode {
            Mode::Check => report.failures.extend(compare(output, &committed)),
            Mode::Write => match write(&generated, output, &committed) {
                Ok(changed) => written = changed,
                Err(err) => report.failures.push(Failure::new(
                    Check::Generated,
                    format!("writing {GENERATED_DIR}: {err}"),
                )),
            },
        }
    }
    print(&report);
    if mode == Mode::Write && report.failures.is_empty() {
        if written.is_empty() {
            println!("assets: {GENERATED_DIR} is up to date");
        }
        for line in &written {
            println!("assets: {line}");
        }
    }
    report.failures.is_empty()
}

/// Writes the files whose bytes changed and removes the ones no longer generated. Returns
/// what it did.
fn write(
    dir: &Path,
    output: &BTreeMap<String, Vec<u8>>,
    committed: &BTreeMap<String, Vec<u8>>,
) -> io::Result<Vec<String>> {
    let mut done = Vec::new();
    for (path, bytes) in output {
        if committed.get(path) != Some(bytes) {
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, bytes)?;
            done.push(format!("wrote {path} ({} bytes)", bytes.len()));
        }
    }
    for path in committed.keys().filter(|path| !output.contains_key(*path)) {
        std::fs::remove_file(dir.join(path))?;
        done.push(format!("removed {path}"));
    }
    Ok(done)
}

fn print(report: &Report) {
    println!("assets: {ART_DIR} -> {GENERATED_DIR}");
    for check in Check::ALL {
        let failures: Vec<&Failure> = report
            .failures
            .iter()
            .filter(|f| f.check == check)
            .collect();
        let verdict = if !failures.is_empty() {
            format!("FAILED ({})", failures.len())
        } else if let Some(skipped) = report.skipped.get(&check) {
            let biomes: Vec<&str> = skipped.iter().flatten().map(String::as_str).collect();
            if skipped.contains(&None) {
                "skipped".to_owned()
            } else {
                format!("skipped for {}", biomes.join(", "))
            }
        } else {
            "ok".to_owned()
        };
        println!("  {} {:<17} {verdict}", check.number(), check.title());
        for failure in failures {
            println!("      - {}", failure.message);
        }
    }
    for note in &report.notes {
        println!("  {note}");
    }
}
