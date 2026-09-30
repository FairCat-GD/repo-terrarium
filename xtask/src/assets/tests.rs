//! The pipeline on the delivered package, and on copies of it broken one way at a time. Each
//! broken copy fails exactly the check its break aims at, and produces no output.
//!
//! A copy lives in memory. The package is read from `art-assets/` (never `references/`), and
//! nothing is written. An edit rehashes the file in the copy's `PACKAGE-CONTENTS.json`, so
//! check 1 stays out of the way of the checks after it.

use serde_json::Value;

use super::*;

const ACTIVE: &str = "biome-temperate/active-manifests.json";
const SKY: &str = "biome-temperate/sky/exports/batch-02/sky-batch-02.json";
const SKY_ATLAS: &str = "biome-temperate/sky/exports/batch-02/sky-batch-02.png";
const SUN: &str = "biome-temperate/sky/exports/batch-02/individual/tmp.sky.sun.png";
const WEEDS: &str =
    "biome-temperate/weeds-litter-fungus/exports/batch-03/weeds-litter-fungus-batch-03.json";
const PLANTS: &str = "biome-temperate/plants/exports/batch-01/plants-batch-01.json";
const CREATURES_06: &str = "biome-temperate/creatures/exports/batch-06/creatures-batch-06.json";
const CREATURES_07: &str = "biome-temperate/creatures/exports/batch-07/creatures-batch-07.json";
const PALETTE: &str = "biome-temperate/palettes/exports/palette-temperate.json";

fn delivered() -> Package {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(ART_DIR);
    Package::read(&root).expect("the delivered package reads")
}

/// Asserts that `check` is the only one that failed and nothing was generated, and returns the
/// failure messages.
fn only(report: &Report, check: Check) -> Vec<String> {
    let failed: BTreeSet<Check> = report.failures.iter().map(|f| f.check).collect();
    assert_eq!(
        failed,
        BTreeSet::from([check]),
        "failures: {:#?}",
        report.failures
    );
    assert!(
        report.output.is_none(),
        "a failing package generated output"
    );
    assert!(report.skipped.contains_key(&Check::Generated));
    report.failures.iter().map(|f| f.message.clone()).collect()
}

fn edit_json(package: &mut Package, path: &str, edit: impl FnOnce(&mut Value)) {
    let mut value: Value =
        serde_json::from_slice(package.file(path).expect("the file exists")).unwrap();
    edit(&mut value);
    package.replace(path, serde_json::to_vec_pretty(&value).unwrap());
}

fn sprite<'a>(manifest: &'a mut Value, id: &str) -> &'a mut Value {
    let sprites = manifest["sprites"].as_array_mut().expect("a sprite list");
    sprites
        .iter_mut()
        .find(|s| s["name"] == id)
        .unwrap_or_else(|| panic!("{id} is listed"))
}

fn animation<'a>(manifest: &'a mut Value, name: &str) -> &'a mut Value {
    let animations = manifest["animations"]
        .as_array_mut()
        .expect("an animation list");
    animations
        .iter_mut()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("{name} is listed"))
}

fn edit_png(
    package: &mut Package,
    path: &str,
    edit: impl FnOnce(&mut Indexed, &mut Vec<u8>, &mut Vec<u8>),
) {
    let bytes = pixels::testing::edit(package.file(path).expect("the PNG exists"), edit);
    package.replace(path, bytes);
}

/// Where the sun sits in the delivered sky atlas.
fn sun_in_atlas(package: &Package) -> (u32, u32) {
    let mut sky: Value = serde_json::from_slice(package.file(SKY).unwrap()).unwrap();
    let rect = &sprite(&mut sky, "tmp.sky.sun")["rect"];
    let coordinate = |key: &str| u32::try_from(rect[key].as_u64().unwrap()).unwrap();
    (coordinate("x"), coordinate("y"))
}

fn set_pixel(image: &mut Indexed, x: u32, y: u32, index: u8) {
    let at = usize::try_from(y * image.width + x).unwrap();
    image.pixels[at] = index;
}

#[test]
fn the_delivered_package_passes_and_packs_the_same_bytes_twice() {
    let package = delivered();
    let first = build(&package);
    assert!(first.failures.is_empty(), "failures: {:#?}", first.failures);
    assert!(first.skipped.is_empty());
    let output = first.output.expect("the package generates");
    let names: Vec<&str> = output.keys().map(String::as_str).collect();
    assert_eq!(names, ["temperate/atlas.bin", "temperate/catalog.bin"]);
    assert_eq!(build(&package).output, Some(output.clone()));
    assert!(compare(&output, &output).is_empty());

    let catalog = Catalog::decode(
        &output["temperate/catalog.bin"],
        &output["temperate/atlas.bin"],
    )
    .unwrap();
    assert_eq!(catalog.sprites.len(), 778, "every ID keeps its own entry");
    assert!(
        first.notes[0].contains("661 distinct images"),
        "{}",
        first.notes[0]
    );
}

#[test]
fn check_1_fails_on_a_corrupted_byte() {
    let mut package = delivered();
    let mut bytes = package.file("START-HERE.md").unwrap().to_vec();
    bytes[0] ^= 1;
    package.put("START-HERE.md", bytes);
    let messages = only(&build(&package), Check::Package);
    assert_eq!(
        messages,
        ["START-HERE.md does not match its SHA-256 in PACKAGE-CONTENTS.json"]
    );
}

#[test]
fn check_1_fails_on_an_unlisted_file_and_a_missing_one() {
    let mut package = delivered();
    package.put("biome-temperate/notes.txt", b"fictional".to_vec());
    package.remove("biome-temperate/INTEGRATION.md");
    let messages = only(&build(&package), Check::Package);
    assert_eq!(
        messages,
        [
            "biome-temperate/INTEGRATION.md is listed in PACKAGE-CONTENTS.json but missing",
            "biome-temperate/notes.txt is not listed in PACKAGE-CONTENTS.json",
        ]
    );
}

#[test]
fn check_2_fails_on_a_duplicate_id() {
    let mut package = delivered();
    edit_json(&mut package, SKY, |sky| {
        sprite(sky, "tmp.sky.star.3")["name"] = "tmp.sky.star.2".into();
    });
    let messages = only(&build(&package), Check::Manifests);
    assert!(
        messages
            .iter()
            .any(|m| m.ends_with("tmp.sky.star.2 is not unique")),
        "{messages:?}"
    );
}

#[test]
fn check_2_fails_when_a_manifest_is_dropped_or_its_revision_moves() {
    let mut package = delivered();
    edit_json(&mut package, ACTIVE, |list| {
        let manifests = list["manifests"].as_array_mut().unwrap();
        manifests.retain(|m| m["manifest"] != "ui/exports/batch-02/ui-batch-02.json");
        manifests[0]["revision"] = 7.into();
    });
    let messages = only(&build(&package), Check::Manifests);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("778 assets declared, 777 loaded")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("revision 6, but the list says 7")),
        "{messages:?}"
    );
}

#[test]
fn check_2_fails_on_a_dangling_reference() {
    let mut package = delivered();
    edit_json(&mut package, WEEDS, |weeds| {
        sprite(weeds, "tmp.fungus.bracket.01.glow")["pairedWith"] = "tmp.fungus.nowhere".into();
    });
    let messages = only(&build(&package), Check::Manifests);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("names tmp.fungus.nowhere, which does not exist")),
        "{messages:?}"
    );
}

#[test]
fn check_2_fails_on_a_key_the_parser_does_not_know() {
    let mut package = delivered();
    edit_json(&mut package, SKY, |sky| {
        sprite(sky, "tmp.sky.sun")["sparkle"] = true.into();
    });
    let messages = only(&build(&package), Check::Manifests);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("sprite tmp.sky.sun: unknown field `sparkle`")),
        "{messages:?}"
    );
}

#[test]
fn check_3_fails_on_a_rect_outside_its_atlas() {
    let mut package = delivered();
    edit_json(&mut package, SKY, |sky| {
        sprite(sky, "tmp.sky.sun")["rect"]["x"] = 1020.into();
    });
    let messages = only(&build(&package), Check::Pixels);
    assert_eq!(
        messages,
        [
            "tmp.sky.sun: its rect lies outside biome-temperate/sky/exports/batch-02/sky-batch-02.png"
        ]
    );
}

#[test]
fn check_3_fails_when_an_individual_png_differs_from_its_atlas() {
    let mut package = delivered();
    edit_png(&mut package, SUN, |image, _, _| {
        let index = image.pixels[0] % 32 + 1;
        set_pixel(image, 0, 0, index);
    });
    let messages = only(&build(&package), Check::Pixels);
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].starts_with("tmp.sky.sun: ") && messages[0].contains("differs from its rect")
    );
}

#[test]
fn check_4_fails_on_index_33_even_where_atlas_and_individual_agree() {
    let mut package = delivered();
    let (x, y) = sun_in_atlas(&package);
    edit_png(&mut package, SUN, |image, _, _| set_pixel(image, 0, 0, 33));
    edit_png(&mut package, SKY_ATLAS, |image, _, _| {
        set_pixel(image, x, y, 33);
    });
    let messages = only(&build(&package), Check::Palette);
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        messages
            .iter()
            .all(|m| m.contains("1 pixels use an index above 32, the first 33")),
        "{messages:?}"
    );
}

#[test]
fn check_4_fails_on_a_colour_outside_the_palette() {
    let mut package = delivered();
    edit_png(&mut package, SUN, |_, plte, _| plte[3 * 5] ^= 1);
    let messages = only(&build(&package), Check::Palette);
    assert_eq!(
        messages,
        [format!(
            "{SUN}: its PLTE is not index 0 plus the 32 day colours"
        )]
    );
}

#[test]
fn check_5_fails_on_a_destination_that_does_not_exist() {
    let mut package = delivered();
    edit_json(&mut package, CREATURES_06, |creatures| {
        animation(creatures, "snail.adult.move")["terminal"] =
            "finish cycle, then adult.idel".into();
    });
    let messages = only(&build(&package), Check::Animations);
    assert_eq!(
        messages,
        ["snail.adult.move: its terminal names adult.idel, which snail lacks"]
    );
}

#[test]
fn check_5_fails_on_a_hand_off_that_lost_its_destination() {
    let mut package = delivered();
    edit_json(&mut package, CREATURES_06, |creatures| {
        animation(creatures, "snail.egg.hatch")["terminal"] = "juvenile idle; never replay".into();
    });
    let messages = only(&build(&package), Check::Animations);
    assert_eq!(
        messages,
        [
            "snail.egg.hatch: its terminal \"juvenile idle; never replay\" names no destination, and the routine neither loops nor holds"
        ]
    );
}

#[test]
fn check_5_fails_on_a_sequence_without_frames() {
    let mut package = delivered();
    edit_json(&mut package, CREATURES_06, |creatures| {
        let feed = animation(creatures, "snail.adult.feed");
        feed["frames"] = Value::Array(Vec::new());
        feed["durationsMs"] = Value::Array(Vec::new());
    });
    let messages = only(&build(&package), Check::Animations);
    assert!(
        messages.contains(&"snail.adult.feed: needs frames, and one duration per frame".to_owned()),
        "{messages:?}"
    );
}

#[test]
fn check_6_fails_on_a_variant_without_art() {
    let mut package = delivered();
    edit_json(&mut package, PLANTS, |plants| {
        sprite(plants, "tmp.plant.oak.foliage.young.pale")["state"] = "pallid".into();
    });
    let messages = only(&build(&package), Check::Variants);
    assert_eq!(
        messages,
        ["temperate: PlantSpecies::Oak × GrowthStage::Young × PlantState::Pale: a foliage"]
    );
}

#[test]
fn check_6_fails_on_pupa_art_for_a_species_without_a_pupa() {
    let mut package = delivered();
    edit_json(&mut package, CREATURES_07, |creatures| {
        let hatch = animation(creatures, "woodlouse.brood.hatch");
        hatch["name"] = "woodlouse.pupa.hatch".into();
        hatch["tag"] = "pupa.hatch".into();
        hatch["lifeStage"] = "pupa".into();
        let frames: Vec<String> = hatch["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap().to_owned())
            .collect();
        for id in frames {
            let frame = sprite(creatures, &id);
            frame["lifeStage"] = "pupa".into();
            frame["animation"] = "woodlouse.pupa.hatch".into();
        }
    });
    let messages = only(&build(&package), Check::Variants);
    assert_eq!(
        messages,
        [
            "temperate: ResidentSpecies::Woodlouse × LifeStage::Pupa: no pupa art, since has_pupa is false"
        ]
    );
}

#[test]
fn check_2_fails_when_the_package_index_miscounts() {
    let mut package = delivered();
    let mut contents: Value =
        serde_json::from_slice(package.file(package::CONTENTS).unwrap()).unwrap();
    contents["activeIds"] = 777.into();
    contents["manifests"] = 15.into();
    package.put(
        package::CONTENTS,
        serde_json::to_vec_pretty(&contents).unwrap(),
    );
    let messages = only(&build(&package), Check::Manifests);
    assert_eq!(
        messages,
        [
            "PACKAGE-CONTENTS.json declares 777 active IDs, 778 loaded",
            "PACKAGE-CONTENTS.json declares 15 manifests, 14 loaded",
        ]
    );
}

#[test]
fn check_2_in_one_biome_skips_its_later_checks_only() {
    let mut package = delivered();
    let mut contents: Value =
        serde_json::from_slice(package.file(package::CONTENTS).unwrap()).unwrap();
    contents["biomes"] = serde_json::json!(["temperate", "cave"]);
    package.put(
        package::CONTENTS,
        serde_json::to_vec_pretty(&contents).unwrap(),
    );
    let report = build(&package);
    let messages = only(&report, Check::Manifests);
    assert_eq!(messages, ["biome-cave/active-manifests.json is missing"]);
    let cave = BTreeSet::from([Some("cave".to_owned())]);
    for check in [
        Check::Pixels,
        Check::Palette,
        Check::Animations,
        Check::Variants,
    ] {
        assert_eq!(report.skipped.get(&check), Some(&cave), "{check:?}");
    }
}

#[test]
fn check_4_fails_when_the_palette_png_leaves_the_json() {
    let mut package = delivered();
    edit_json(&mut package, PALETTE, |palette| {
        palette["slots"][31]["night"] = "#000000".into();
    });
    let messages = only(&build(&package), Check::Palette);
    assert_eq!(
        messages,
        [
            "biome-temperate/palettes/exports/palette-temperate.png does not match the palette JSON (row 0 day, row 1 night)"
        ]
    );
}

#[test]
fn check_4_fails_on_a_translucent_index() {
    let mut package = delivered();
    edit_png(&mut package, SUN, |_, _, trns| trns[5] = 128);
    let messages = only(&build(&package), Check::Palette);
    assert_eq!(
        messages,
        [format!(
            "{SUN}: its tRNS does not make index 0, and only index 0, transparent"
        )]
    );
}

#[test]
fn check_5_fails_on_a_creature_frame_no_routine_lists() {
    let mut package = delivered();
    edit_json(&mut package, CREATURES_06, |creatures| {
        sprite(creatures, "tmp.creature.snail.adult.move.f01")["animation"] =
            "snail.adult.mvoe".into();
    });
    let messages = only(&build(&package), Check::Animations);
    assert!(
        messages.contains(
            &"tmp.creature.snail.adult.move.f01: names snail.adult.mvoe, which no routine lists"
                .to_owned()
        ),
        "{messages:?}"
    );
}

#[test]
fn check_5_fails_on_an_effect_frame_hosted_outside_its_routine() {
    let mut package = delivered();
    edit_json(&mut package, CREATURES_06, |creatures| {
        sprite(creatures, "tmp.creature.firefly.effect.signal-glow.f04")["attachment"] =
            "tmp.creature.firefly.adult.idle.f01".into();
    });
    let messages = only(&build(&package), Check::Animations);
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("signal-glow-once: it declares 4 frames, and 3")),
        "{messages:?}"
    );
}

/// A directory the test owns under the system temp dir, removed when dropped.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "terrarium-assets-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp dir is writable");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn write_mode_rewrites_what_changed_and_removes_what_is_no_longer_generated() {
    let dir = TempDir::new("write");
    std::fs::create_dir_all(dir.0.join("old")).unwrap();
    std::fs::write(dir.0.join("old/atlas.bin"), [1]).unwrap();
    std::fs::write(dir.0.join(".DS_Store"), [0]).unwrap();
    let output = BTreeMap::from([
        ("temperate/atlas.bin".to_owned(), vec![0, 1, 2]),
        ("temperate/catalog.bin".to_owned(), vec![9]),
    ]);

    let committed = read_generated(&dir.0).unwrap();
    assert_eq!(
        committed.keys().collect::<Vec<_>>(),
        ["old/atlas.bin"],
        "Finder files are ignored"
    );
    let done = write(&dir.0, &output, &committed).unwrap();
    assert_eq!(
        done,
        [
            "wrote temperate/atlas.bin (3 bytes)",
            "wrote temperate/catalog.bin (1 bytes)",
            "removed old/atlas.bin"
        ]
    );
    assert_eq!(read_generated(&dir.0).unwrap(), output);

    let unchanged = read_generated(&dir.0).unwrap();
    assert!(write(&dir.0, &output, &unchanged).unwrap().is_empty());
    assert!(compare(&output, &unchanged).is_empty());
}

#[test]
fn check_7_compares_byte_for_byte_both_ways() {
    let output = BTreeMap::from([
        ("temperate/atlas.bin".to_owned(), vec![0, 1, 2, 3]),
        ("temperate/catalog.bin".to_owned(), vec![9, 9]),
    ]);
    assert!(compare(&output, &output).is_empty());

    let mut flipped = output.clone();
    flipped.get_mut("temperate/atlas.bin").unwrap()[2] ^= 1;
    let failures = compare(&output, &flipped);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].check, Check::Generated);
    assert!(
        failures[0]
            .message
            .starts_with("temperate/atlas.bin differs from the regenerated file at byte 2")
    );

    let mut missing = output.clone();
    missing.remove("temperate/catalog.bin");
    missing.insert("old/atlas.bin".to_owned(), vec![1]);
    let messages: Vec<String> = compare(&output, &missing)
        .into_iter()
        .map(|f| f.message)
        .collect();
    assert_eq!(
        messages,
        [
            "temperate/catalog.bin is not committed; run `cargo xtask assets`",
            "old/atlas.bin is committed but no longer generated; run `cargo xtask assets`",
        ]
    );
}

#[test]
fn git_keeps_the_generated_files_binary() {
    let attributes = include_str!("../../../.gitattributes");
    assert!(
        attributes
            .lines()
            .any(|line| line.trim() == format!("{GENERATED_DIR}/** binary")),
        "`.gitattributes` must mark {GENERATED_DIR} binary"
    );
}

#[test]
fn checks_are_numbered_in_their_order() {
    let numbers: Vec<usize> = Check::ALL.iter().map(|c| c.number()).collect();
    assert_eq!(numbers, [1, 2, 3, 4, 5, 6, 7]);
}
