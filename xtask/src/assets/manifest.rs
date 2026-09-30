//! Check 2: the manifests. Only the manifests `active-manifests.json` lists are loaded, never
//! every JSON file, and their counts, revisions and IDs must agree with what the package
//! declares.
//!
//! Schema shapes vary between batches, so each level (manifest, sprite, animation) is one
//! struct holding every key any batch uses. Every key is either read, or named as prose or
//! provenance and ignored. `deny_unknown_fields` then turns a key a future delivery adds into
//! a failure that names it, instead of a field silently dropped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use serde::de::IgnoredAny;
use terrarium_compose::catalog::{
    ArtKeys, Blend, BurnoffKeys, Envelope, Flags, HoverPlate, Layer, NineSlice, PALETTE_SLOTS,
    Palette, Point, Rect, Residents, Rgb, Size,
};

use super::package::{Contents, Package};
use super::{Check, Failure};

/// The only schema this parser is written for.
const SCHEMA_VERSION: u32 = 1;
/// A glyph cell's `part`.
const GLYPH: &str = "glyph";

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ActiveListJson {
    schema_version: u32,
    biome: String,
    assets: u32,
    non_glyph_assets: u32,
    glyphs: u32,
    manifests: Vec<ActiveEntryJson>,
    style: Option<IgnoredAny>,
    status: Option<IgnoredAny>,
    historical: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveEntryJson {
    manifest: String,
    revision: u32,
    pieces: u32,
    category: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ManifestJson {
    schema_version: u32,
    revision: u32,
    atlas: String,
    /// Read one by one, so a failure can name its sprite.
    sprites: Vec<serde_json::Value>,
    canvas: Option<SizeJson>,
    width: Option<u32>,
    height: Option<u32>,
    palette: Option<String>,
    opaque_palette_slots: Option<u32>,
    transparent_index: Option<u32>,
    animation: Option<TopAnimationJson>,
    animations: Option<Vec<AnimationJson>>,
    effect_animation: Option<EffectJson>,
    motion_budget: Option<MotionBudgetJson>,
    transitions: Option<TransitionsJson>,
    reused_active_ids: Option<BTreeMap<String, Vec<String>>>,
    hover_plate: Option<HoverPlateJson>,
    nine_slice: Option<NineSliceJson>,
    fonts: Option<Vec<FontJson>>,
    glyph_count: Option<u32>,
    unique_characters: Option<u32>,
    // Prose, provenance, and restatements of what the sprites carry.
    arbitration: Option<IgnoredAny>,
    assembly: Option<IgnoredAny>,
    batch: Option<IgnoredAny>,
    biome: Option<IgnoredAny>,
    category: Option<IgnoredAny>,
    coordinates: Option<IgnoredAny>,
    exceptions: Option<IgnoredAny>,
    ink_slot: Option<IgnoredAny>,
    notes: Option<IgnoredAny>,
    opacity_contract: Option<IgnoredAny>,
    origin: Option<IgnoredAny>,
    provenance: Option<IgnoredAny>,
    registration: Option<IgnoredAny>,
    source_license: Option<IgnoredAny>,
    status: Option<IgnoredAny>,
    style: Option<IgnoredAny>,
    tint_policy: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SpriteJson {
    name: String,
    file: String,
    rect: RectJson,
    anchor: PointJson,
    layer: Option<String>,
    blend: Option<String>,
    opacity: Option<u32>,
    tile_x: Option<bool>,
    day_only: Option<bool>,
    night_only: Option<bool>,
    settled: Option<bool>,
    species: Option<String>,
    stage: Option<String>,
    state: Option<String>,
    part: Option<String>,
    life_stage: Option<String>,
    event: Option<String>,
    attachment_point: Option<PointJson>,
    shear_pivot: Option<PointJson>,
    foliage_attachment: Option<PointJson>,
    root_delta: Option<PointJson>,
    ground_contacts: Option<Vec<PointJson>>,
    animation: Option<String>,
    frame: Option<u32>,
    duration_ms: Option<u32>,
    #[serde(rename = "loop")]
    looping: Option<bool>,
    play_count: Option<u32>,
    paired_with: Option<String>,
    attachment: Option<String>,
    targets: Option<Vec<String>>,
    replaces: Option<String>,
    reference_foliage: Option<String>,
    shared_stem: Option<String>,
    eligible_stages: Option<Vec<String>>,
    eligible_health_states: Option<Vec<String>>,
    transition: Option<EnvelopeJson>,
    transition_ms: Option<u32>,
    weed_opacity_at_frame_start: Option<f64>,
    fresh_soil_opacity_at_frame_start: Option<f64>,
    text_inset: Option<RectJson>,
    font: Option<String>,
    codepoint: Option<u32>,
    character: Option<String>,
    // Prose, provenance, and restatements of what the fields above carry.
    advance: Option<IgnoredAny>,
    bloom_policy: Option<IgnoredAny>,
    canvas: Option<IgnoredAny>,
    category: Option<IgnoredAny>,
    clear_after_playback: Option<IgnoredAny>,
    component_intent: Option<IgnoredAny>,
    condition: Option<IgnoredAny>,
    direction: Option<IgnoredAny>,
    equivalent_variant: Option<IgnoredAny>,
    event_only: Option<IgnoredAny>,
    facing: Option<IgnoredAny>,
    health_state: Option<IgnoredAny>,
    minimum_dead_days: Option<IgnoredAny>,
    opaque_pixels: Option<IgnoredAny>,
    palette_group: Option<IgnoredAny>,
    pivot_space: Option<IgnoredAny>,
    placement: Option<IgnoredAny>,
    placement_mode: Option<IgnoredAny>,
    replaces_foliage_family: Option<IgnoredAny>,
    replaces_individual_weeds: Option<IgnoredAny>,
    signal: Option<IgnoredAny>,
    stem_policy: Option<IgnoredAny>,
    trim_offset: Option<IgnoredAny>,
}

/// A creature routine from a manifest's `animations[]`.
#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnimationJson {
    pub name: String,
    pub species: String,
    pub life_stage: String,
    pub state: String,
    pub tag: String,
    pub terminal: String,
    pub frames: Vec<String>,
    pub durations_ms: Vec<u32>,
    #[serde(rename = "loop")]
    pub looping: bool,
    pub play_count: u32,
    pub cycle_displacement: i32,
    pub transition_ms: u32,
    native_frame_range: Option<IgnoredAny>,
    root_motion: Option<IgnoredAny>,
    source: Option<IgnoredAny>,
}

/// The events manifest's top-level sequence: burn-off.
#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TopAnimationJson {
    pub name: String,
    /// A count here, not a list.
    pub frames: u32,
    pub duration_ms: u32,
    #[serde(rename = "loop")]
    pub looping: bool,
    pub play_count: u32,
    pub remove_final_frame: bool,
    source: Option<IgnoredAny>,
}

/// An attached effect synchronised to a routine: the firefly lantern.
#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EffectJson {
    pub tag: String,
    pub sync: String,
    /// Prose; an effect always ends by being removed after its opacity-0 last frame.
    terminal: Option<IgnoredAny>,
    /// A count here, not a list.
    pub frames: u32,
    #[serde(rename = "loop")]
    pub looping: bool,
    pub play_count: u32,
    opacity_location: Option<IgnoredAny>,
    source: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MotionBudgetJson {
    moving_residents: u32,
    flying_or_signalling: u32,
    render_cap_fps: u32,
    phase_offsets_ms: BTreeMap<String, u32>,
    move_frame_ms: Option<IgnoredAny>,
    global_cap: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TransitionsJson {
    duration_ms: u32,
    policy: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct HoverPlateJson {
    size: SizeJson,
    safe_text: RectJson,
    max_body_characters: u32,
    policy: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NineSliceJson {
    inset: u32,
    tile_size: u32,
    minimum_size: SizeJson,
    center: Option<IgnoredAny>,
    edges: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FontJson {
    name: String,
    cell: SizeJson,
    advance: u32,
    ascent: u32,
    descent: u32,
    line_height: u32,
    fallback: u32,
    codepoints: Vec<u32>,
    case_policy: Option<IgnoredAny>,
    license: Option<IgnoredAny>,
    source: Option<IgnoredAny>,
    source_sha256: Option<IgnoredAny>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeJson {
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RectJson {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct PointJson {
    x: i32,
    y: i32,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EnvelopeJson {
    enter_ms: u32,
    hold_ms: u32,
    exit_ms: u32,
    #[serde(rename = "loop")]
    looping: Option<IgnoredAny>,
    exit_to: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PaletteJson {
    opaque_slots: u32,
    transparent_index: u32,
    slots: Vec<SlotJson>,
    index_convention: Option<IgnoredAny>,
    status: Option<IgnoredAny>,
    version: Option<IgnoredAny>,
}

#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SlotJson {
    slot: u32,
    aseprite_index: u32,
    day: String,
    night: String,
    name: Option<IgnoredAny>,
}

/// A delivered section atlas.
#[derive(Clone, Debug)]
pub struct Atlas {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

/// A per-sprite animation frame: the sky and soil sequences, burn-off, the lantern, and the
/// creatures' copies of their `animations[]`.
#[derive(Clone, Debug)]
pub struct FrameJson {
    pub animation: Option<String>,
    pub index: u32,
    pub duration_ms: u32,
    pub looping: Option<bool>,
    pub play_count: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ScorchDraft {
    pub eligible_stages: Vec<String>,
    pub eligible_states: Vec<String>,
    pub replaces: String,
    pub shared_stem: String,
}

/// A sprite as the manifests describe it, its values checked and typed.
#[derive(Clone, Debug)]
pub struct SpriteDraft {
    pub id: String,
    pub manifest: String,
    pub file: String,
    /// Index into [`Biome::atlases`].
    pub atlas: usize,
    /// Where the pixels sit in the delivered atlas.
    pub rect: Rect,
    pub anchor: Point,
    pub layer: Option<Layer>,
    pub blend: Blend,
    pub opacity: u8,
    pub flags: Flags,
    pub keys: ArtKeys,
    pub attachment_point: Option<Point>,
    pub shear_pivot: Option<Point>,
    pub foliage_attachment: Option<Point>,
    pub root_delta: Option<Point>,
    pub ground_contacts: Option<Vec<Point>>,
    pub frame: Option<FrameJson>,
    pub paired_with: Option<String>,
    pub attachment: Option<String>,
    pub targets: Vec<String>,
    pub scorch: Option<ScorchDraft>,
    pub transition_ms: Option<u16>,
    pub envelope: Option<Envelope>,
    pub burnoff: Option<BurnoffKeys>,
    pub text_inset: Option<Rect>,
    /// A glyph cell's font and codepoint.
    pub glyph: Option<(String, u32)>,
}

#[derive(Clone, Debug)]
pub struct FontDraft {
    pub name: String,
    pub cell: Size,
    pub advance: u8,
    pub ascent: u8,
    pub descent: u8,
    pub line_height: u8,
    pub fallback: u32,
}

/// One biome's manifests, loaded and cross-checked.
#[derive(Clone, Debug)]
pub struct Biome {
    pub name: String,
    pub palette_png: String,
    pub palette: Palette,
    pub atlases: Vec<Atlas>,
    /// Sorted by ID.
    pub sprites: Vec<SpriteDraft>,
    pub creature_animations: Vec<AnimationJson>,
    pub top_animations: Vec<TopAnimationJson>,
    pub effects: Vec<EffectJson>,
    pub residents: Residents,
    pub groups: BTreeMap<String, Vec<String>>,
    pub hover_plate: HoverPlate,
    pub nine_slice: NineSlice,
    pub fonts: Vec<FontDraft>,
    /// The number of manifests loaded.
    pub manifests: u32,
}

impl Biome {
    pub fn sprite(&self, id: &str) -> Option<&SpriteDraft> {
        self.sprites
            .binary_search_by(|sprite| sprite.id.as_str().cmp(id))
            .ok()
            .map(|i| &self.sprites[i])
    }
}

/// Loads one biome's manifests, or says every way they fail check 2.
pub fn load(package: &Package, biome: &str) -> Result<Biome, Vec<Failure>> {
    let mut out = Loader::default();
    let dir = format!("biome-{biome}");
    let list_path = format!("{dir}/active-manifests.json");
    let Some(list) = out.parse::<ActiveListJson>(package, &list_path) else {
        return Err(out.failures);
    };
    out.expect(list.schema_version == SCHEMA_VERSION, || {
        format!(
            "{list_path}: schema {} is not {SCHEMA_VERSION}",
            list.schema_version
        )
    });
    out.expect(list.biome == biome, || {
        format!("{list_path}: names biome {}", list.biome)
    });
    for entry in &list.manifests {
        let Some(path) = resolve(&dir, &entry.manifest) else {
            out.fail(format!(
                "{list_path}: {} leaves the package",
                entry.manifest
            ));
            continue;
        };
        out.manifest(package, &path, entry);
    }
    out.finish(package, &list_path, &list, biome)
}

#[derive(Default)]
struct Loader {
    failures: Vec<Failure>,
    atlases: Vec<Atlas>,
    sprites: Vec<SpriteDraft>,
    palettes: BTreeSet<String>,
    creature_animations: Vec<AnimationJson>,
    top_animations: Vec<TopAnimationJson>,
    effects: Vec<EffectJson>,
    budgets: Vec<MotionBudgetJson>,
    transitions: Vec<u32>,
    groups: BTreeMap<String, Vec<String>>,
    hover_plates: Vec<HoverPlateJson>,
    nine_slices: Vec<NineSliceJson>,
    fonts: Vec<FontJson>,
    manifests: u32,
}

impl Loader {
    fn fail(&mut self, message: String) {
        self.failures.push(Failure::new(Check::Manifests, message));
    }

    fn expect(&mut self, holds: bool, message: impl FnOnce() -> String) {
        if !holds {
            self.fail(message());
        }
    }

    fn parse<T: for<'de> Deserialize<'de>>(&mut self, package: &Package, path: &str) -> Option<T> {
        let Some(bytes) = package.file(path) else {
            self.fail(format!("{path} is missing"));
            return None;
        };
        match serde_json::from_slice(bytes) {
            Ok(value) => Some(value),
            Err(err) => {
                self.fail(format!("{path}: {err}"));
                None
            }
        }
    }

    fn manifest(&mut self, package: &Package, path: &str, entry: &ActiveEntryJson) {
        let Some(mut manifest) = self.parse::<ManifestJson>(package, path) else {
            return;
        };
        self.manifests += 1;
        self.header(path, entry, &manifest);
        let Some(atlas) = self.atlas(path, &manifest) else {
            return;
        };
        for value in std::mem::take(&mut manifest.sprites) {
            let name = value
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("?")
                .to_owned();
            let draft = serde_json::from_value::<SpriteJson>(value)
                .map_err(|err| err.to_string())
                .and_then(|json| sprite(path, atlas, json));
            match draft {
                Ok(draft) => self.sprites.push(draft),
                Err(err) => self.fail(format!("{path}: sprite {name}: {err}")),
            }
        }
        self.extras(path, manifest);
    }

    /// The manifest against its entry in the list, and the palette it names.
    fn header(&mut self, path: &str, entry: &ActiveEntryJson, manifest: &ManifestJson) {
        self.expect(manifest.schema_version == SCHEMA_VERSION, || {
            format!(
                "{path}: schema {} is not {SCHEMA_VERSION}",
                manifest.schema_version
            )
        });
        self.expect(manifest.revision == entry.revision, || {
            format!(
                "{path}: revision {}, but the list says {}",
                manifest.revision, entry.revision
            )
        });
        let pieces = manifest.sprites.len();
        self.expect(u32::try_from(pieces).ok() == Some(entry.pieces), || {
            format!(
                "{path}: {pieces} sprites, but the list says {}",
                entry.pieces
            )
        });
        self.expect(
            manifest.opaque_palette_slots.is_none_or(|n| n == 32),
            || format!("{path}: opaquePaletteSlots is not 32"),
        );
        self.expect(manifest.transparent_index.is_none_or(|n| n == 0), || {
            format!("{path}: transparentIndex is not 0")
        });
        // `palette` is a path in some batches and prose in others.
        let is_path = |p: &&str| {
            Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        };
        if let Some(palette) = manifest.palette.as_deref().filter(is_path) {
            match resolve(dir_of(path), palette) {
                Some(palette) => {
                    self.palettes.insert(palette);
                }
                None => self.fail(format!("{path}: palette {palette} leaves the package")),
            }
        }
    }

    /// Registers the manifest's atlas; returns its index.
    fn atlas(&mut self, path: &str, manifest: &ManifestJson) -> Option<usize> {
        let (width, height) = match (manifest.canvas, manifest.width, manifest.height) {
            (Some(SizeJson { w, h }), None, None) | (None, Some(w), Some(h)) => (w, h),
            _ => {
                self.fail(format!(
                    "{path}: the atlas size is neither `canvas` nor `width`/`height`"
                ));
                (0, 0)
            }
        };
        let Some(atlas_path) = resolve(dir_of(path), &manifest.atlas) else {
            self.fail(format!(
                "{path}: atlas {} leaves the package",
                manifest.atlas
            ));
            return None;
        };
        self.atlases.push(Atlas {
            path: atlas_path,
            width,
            height,
        });
        Some(self.atlases.len() - 1)
    }

    /// Everything a manifest holds besides its sprites.
    fn extras(&mut self, path: &str, manifest: ManifestJson) {
        if let Some(fonts) = manifest.fonts {
            let codepoints: Vec<u32> = self
                .sprites
                .iter()
                .filter(|s| s.manifest == path)
                .filter_map(|s| s.glyph.as_ref().map(|(_, c)| *c))
                .collect();
            let count = codepoints.len();
            let unique = codepoints.iter().collect::<BTreeSet<_>>().len();
            self.expect(
                manifest
                    .glyph_count
                    .is_none_or(|n| usize::try_from(n) == Ok(count)),
                || format!("{path}: glyphCount is not the {count} glyphs listed"),
            );
            self.expect(
                manifest
                    .unique_characters
                    .is_none_or(|n| usize::try_from(n) == Ok(unique)),
                || format!("{path}: uniqueCharacters is not the {unique} listed"),
            );
            self.fonts.extend(fonts);
        }
        self.creature_animations
            .extend(manifest.animations.unwrap_or_default());
        self.top_animations.extend(manifest.animation);
        self.effects.extend(manifest.effect_animation);
        self.budgets.extend(manifest.motion_budget);
        self.transitions
            .extend(manifest.transitions.map(|t| t.duration_ms));
        self.hover_plates.extend(manifest.hover_plate);
        self.nine_slices.extend(manifest.nine_slice);
        for (name, ids) in manifest.reused_active_ids.unwrap_or_default() {
            if self.groups.insert(name.clone(), ids).is_some() {
                self.fail(format!("{path}: the reused group {name} is defined twice"));
            }
        }
    }

    fn finish(
        mut self,
        package: &Package,
        list_path: &str,
        list: &ActiveListJson,
        biome: &str,
    ) -> Result<Biome, Vec<Failure>> {
        self.sprites.sort_by(|a, b| a.id.cmp(&b.id));
        let mut seen = BTreeSet::new();
        for sprite in &self.sprites {
            if !sprite.id.starts_with("tmp.") {
                self.failures.push(Failure::new(
                    Check::Manifests,
                    format!("{}: {} lacks the `tmp.` prefix", sprite.manifest, sprite.id),
                ));
            }
            if !seen.insert(sprite.id.as_str()) {
                self.failures.push(Failure::new(
                    Check::Manifests,
                    format!("{}: {} is not unique", sprite.manifest, sprite.id),
                ));
            }
        }
        self.counts(list_path, list);
        self.references();
        let palette = self.palette(package, list_path);
        let residents = self.residents(list_path);
        let fonts = self.fonts(list_path);
        let hover_plate = self.hover_plate(list_path);
        let nine_slice = self.nine_slice(list_path);
        if !self.failures.is_empty() {
            return Err(self.failures);
        }
        let (Some((palette_png, palette)), Some(residents), Some(hover_plate), Some(nine_slice)) =
            (palette, residents, hover_plate, nine_slice)
        else {
            unreachable!("each of these fails check 2 when it is missing");
        };
        Ok(Biome {
            name: biome.to_owned(),
            palette_png,
            palette,
            atlases: self.atlases,
            sprites: self.sprites,
            creature_animations: self.creature_animations,
            top_animations: self.top_animations,
            effects: self.effects,
            residents,
            groups: self.groups,
            hover_plate,
            nine_slice,
            fonts,
            manifests: self.manifests,
        })
    }

    fn counts(&mut self, list_path: &str, list: &ActiveListJson) {
        let total = self.sprites.len();
        let glyphs = self
            .sprites
            .iter()
            .filter(|s| s.keys.part.as_deref() == Some(GLYPH))
            .count();
        let declared = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
        self.expect(declared(list.assets) == total, || {
            format!(
                "{list_path}: {} assets declared, {total} loaded",
                list.assets
            )
        });
        self.expect(declared(list.glyphs) == glyphs, || {
            format!(
                "{list_path}: {} glyphs declared, {glyphs} loaded",
                list.glyphs
            )
        });
        self.expect(declared(list.non_glyph_assets) == total - glyphs, || {
            format!(
                "{list_path}: {} non-glyph assets declared, {} loaded",
                list.non_glyph_assets,
                total - glyphs
            )
        });
    }

    /// Every ID a sprite or a reused group names exists.
    fn references(&mut self) {
        let ids: BTreeSet<&str> = self.sprites.iter().map(|s| s.id.as_str()).collect();
        let mut dangling = Vec::new();
        for sprite in &self.sprites {
            let scorch = sprite
                .scorch
                .iter()
                .flat_map(|s| [&s.replaces, &s.shared_stem]);
            let named = sprite
                .paired_with
                .iter()
                .chain(&sprite.attachment)
                .chain(&sprite.targets);
            for id in named.chain(scorch) {
                if !ids.contains(id.as_str()) {
                    dangling.push(format!(
                        "{}: {} names {id}, which does not exist",
                        sprite.manifest, sprite.id
                    ));
                }
            }
        }
        for (name, members) in &self.groups {
            for id in members.iter().filter(|id| !ids.contains(id.as_str())) {
                dangling.push(format!(
                    "the reused group {name} names {id}, which does not exist"
                ));
            }
        }
        for message in dangling {
            self.fail(message);
        }
    }

    /// The biome's palette: the one file every manifest that names a palette path names, with
    /// its 32×2 PNG beside it.
    fn palette(&mut self, package: &Package, list_path: &str) -> Option<(String, Palette)> {
        if self.palettes.len() != 1 {
            let named = self.palettes.iter().cloned().collect::<Vec<_>>().join(", ");
            self.fail(format!(
                "{list_path}: the manifests name {} palettes ({named}), not one",
                self.palettes.len()
            ));
            return None;
        }
        let path = self.palettes.first()?.clone();
        let json = self.parse::<PaletteJson>(package, &path)?;
        let mut day = [[0; 3]; PALETTE_SLOTS];
        let mut night = [[0; 3]; PALETTE_SLOTS];
        let slots_ok = json.opaque_slots == 32
            && json.transparent_index == 0
            && json.slots.len() == PALETTE_SLOTS
            && json.slots.iter().enumerate().all(|(i, slot)| {
                usize::try_from(slot.slot) == Ok(i)
                    && usize::try_from(slot.aseprite_index) == Ok(i + 1)
            });
        if !slots_ok {
            self.fail(format!(
                "{path}: not 32 slots numbered 0..=31 at Aseprite indices 1..=32"
            ));
            return None;
        }
        for (i, slot) in json.slots.iter().enumerate() {
            let (Some(d), Some(n)) = (colour(&slot.day), colour(&slot.night)) else {
                self.fail(format!("{path}: slot {i} is not two #RRGGBB colours"));
                return None;
            };
            (day[i], night[i]) = (d, n);
        }
        let png = format!("{}.png", path.strip_suffix(".json").unwrap_or(&path));
        Some((png, Palette { day, night }))
    }

    fn residents(&mut self, list_path: &str) -> Option<Residents> {
        let Some(first) = self.budgets.first() else {
            self.fail(format!(
                "{list_path}: no manifest gives the residents' motion budget"
            ));
            return None;
        };
        let shared = (
            first.moving_residents,
            first.flying_or_signalling,
            first.render_cap_fps,
        );
        let agree = self
            .budgets
            .iter()
            .all(|b| (b.moving_residents, b.flying_or_signalling, b.render_cap_fps) == shared);
        let mut offsets = BTreeMap::new();
        let mut twice = Vec::new();
        for (species, ms) in self.budgets.iter().flat_map(|b| &b.phase_offsets_ms) {
            if offsets.insert(species.clone(), *ms).is_some() {
                twice.push(species.clone());
            }
        }
        let transition = self.transitions.first().copied();
        let transitions_agree = self.transitions.iter().all(|t| Some(*t) == transition);
        self.expect(agree, || {
            format!("{list_path}: the creature manifests' motion budgets disagree")
        });
        self.expect(twice.is_empty(), || {
            format!(
                "{list_path}: phase offsets given twice for {}",
                twice.join(", ")
            )
        });
        self.expect(transitions_agree && transition.is_some(), || {
            format!("{list_path}: the creature transitions are missing or disagree")
        });
        let small = |n: u32| u8::try_from(n).ok();
        let residents = (|| {
            Some(Residents {
                moving: small(shared.0)?,
                flying_or_signalling: small(shared.1)?,
                render_cap_fps: small(shared.2)?,
                transition_ms: u16::try_from(transition?).ok()?,
                phase_offsets_ms: offsets
                    .into_iter()
                    .map(|(species, ms)| Some((species, u16::try_from(ms).ok()?)))
                    .collect::<Option<Vec<_>>>()?,
            })
        })();
        self.expect(residents.is_some(), || {
            format!("{list_path}: the motion budget is out of range")
        });
        residents
    }

    /// Each font's metrics, with exactly one glyph sprite per listed codepoint.
    fn fonts(&mut self, list_path: &str) -> Vec<FontDraft> {
        let mut fonts = Vec::new();
        let mut failures = Vec::new();
        for font in &self.fonts {
            let listed: BTreeSet<u32> = font.codepoints.iter().copied().collect();
            let drawn: Vec<u32> = self
                .sprites
                .iter()
                .filter_map(|s| {
                    s.glyph
                        .as_ref()
                        .filter(|(f, _)| *f == font.name)
                        .map(|(_, c)| *c)
                })
                .collect();
            let unique: BTreeSet<u32> = drawn.iter().copied().collect();
            if unique != listed
                || unique.len() != drawn.len()
                || listed.len() != font.codepoints.len()
            {
                failures.push(format!(
                    "{list_path}: font {} does not have one glyph per codepoint",
                    font.name
                ));
            }
            if !listed.contains(&font.fallback) {
                failures.push(format!(
                    "{list_path}: font {}'s fallback has no glyph",
                    font.name
                ));
            }
            let metrics = [font.advance, font.ascent, font.descent, font.line_height]
                .map(|n| u8::try_from(n).ok());
            let cell = size(font.cell);
            match (metrics, cell) {
                (
                    [
                        Some(advance),
                        Some(ascent),
                        Some(descent),
                        Some(line_height),
                    ],
                    Some(cell),
                ) => {
                    fonts.push(FontDraft {
                        name: font.name.clone(),
                        cell,
                        advance,
                        ascent,
                        descent,
                        line_height,
                        fallback: font.fallback,
                    });
                }
                _ => failures.push(format!(
                    "{list_path}: font {}'s metrics are out of range",
                    font.name
                )),
            }
        }
        let orphans = self.sprites.iter().filter(|s| {
            s.glyph
                .as_ref()
                .is_some_and(|(f, _)| !self.fonts.iter().any(|font| font.name == *f))
        });
        failures.extend(orphans.map(|s| format!("{}: {} belongs to no font", s.manifest, s.id)));
        for message in failures {
            self.fail(message);
        }
        fonts.sort_by(|a, b| a.name.cmp(&b.name));
        fonts
    }

    fn hover_plate(&mut self, list_path: &str) -> Option<HoverPlate> {
        let [plate] = self.hover_plates.as_slice() else {
            self.fail(format!(
                "{list_path}: {} hover plates, not one",
                self.hover_plates.len()
            ));
            return None;
        };
        let plate = (|| {
            Some(HoverPlate {
                size: size(plate.size)?,
                safe_text: rect(plate.safe_text)?,
                max_body_characters: u8::try_from(plate.max_body_characters).ok()?,
            })
        })();
        self.expect(plate.is_some(), || {
            format!("{list_path}: the hover plate is out of range")
        });
        plate
    }

    fn nine_slice(&mut self, list_path: &str) -> Option<NineSlice> {
        let [slice] = self.nine_slices.as_slice() else {
            self.fail(format!(
                "{list_path}: {} nine-slice panels, not one",
                self.nine_slices.len()
            ));
            return None;
        };
        let slice = (|| {
            Some(NineSlice {
                inset: u8::try_from(slice.inset).ok()?,
                tile_size: u8::try_from(slice.tile_size).ok()?,
                minimum: size(slice.minimum_size)?,
            })
        })();
        self.expect(slice.is_some(), || {
            format!("{list_path}: the nine-slice panel is out of range")
        });
        slice
    }
}

/// Checks and types one sprite's values.
fn sprite(manifest: &str, atlas: usize, mut json: SpriteJson) -> Result<SpriteDraft, String> {
    let file = resolve(dir_of(manifest), &json.file)
        .ok_or_else(|| format!("file {} leaves the package", json.file))?;
    let bounds = rect(json.rect)
        .filter(|r| r.w > 0 && r.h > 0)
        .ok_or("the rect is empty or too large")?;
    let (layer, blend, opacity) = looks(&json)?;
    let text_inset = json
        .text_inset
        .map(|r| rect(r).ok_or("the text inset is too large"))
        .transpose()?;
    let ground_contacts = json
        .ground_contacts
        .take()
        .map(|points| {
            let points: Option<Vec<Point>> = points.into_iter().map(point).collect();
            points.ok_or("a ground contact is too far out")
        })
        .transpose()?;
    Ok(SpriteDraft {
        manifest: manifest.to_owned(),
        file,
        atlas,
        rect: bounds,
        anchor: point(json.anchor).ok_or("the anchor is too far out")?,
        layer,
        blend,
        opacity,
        flags: flags(&json),
        attachment_point: optional_point(json.attachment_point)?,
        shear_pivot: optional_point(json.shear_pivot)?,
        foliage_attachment: optional_point(json.foliage_attachment)?,
        root_delta: optional_point(json.root_delta)?,
        ground_contacts,
        frame: frame(&mut json)?,
        scorch: scorch(&mut json)?,
        transition_ms: json
            .transition_ms
            .map(|n| u16::try_from(n).map_err(|_| "transitionMs exceeds 65535"))
            .transpose()?,
        envelope: json.transition.as_ref().map(envelope).transpose()?,
        burnoff: burnoff(&json)?,
        text_inset,
        glyph: glyph(&mut json)?,
        paired_with: json.paired_with.take(),
        attachment: json.attachment.take(),
        targets: json.targets.take().unwrap_or_default(),
        keys: ArtKeys {
            species: json.species.take(),
            stage: json.stage.take(),
            state: json.state.take(),
            part: json.part.take(),
            life_stage: json.life_stage.take(),
            event: json.event.take(),
        },
        id: json.name,
    })
}

/// The layer, blend and opacity; a missing blend is normal and a missing opacity is 255.
fn looks(json: &SpriteJson) -> Result<(Option<Layer>, Blend, u8), String> {
    let layer = json
        .layer
        .as_deref()
        .map(|name| Layer::from_art_name(name).ok_or(format!("unknown layer {name}")))
        .transpose()?;
    let blend = json
        .blend
        .as_deref()
        .map(|name| Blend::from_art_name(name).ok_or(format!("unknown blend {name}")))
        .transpose()?
        .unwrap_or_default();
    let opacity = u8::try_from(json.opacity.unwrap_or(255)).map_err(|_| "opacity exceeds 255")?;
    Ok((layer, blend, opacity))
}

fn flags(json: &SpriteJson) -> Flags {
    [
        (json.tile_x, Flags::TILE_X),
        (json.day_only, Flags::DAY_ONLY),
        (json.night_only, Flags::NIGHT_ONLY),
        (json.settled, Flags::SETTLED),
    ]
    .into_iter()
    .filter(|(set, _)| *set == Some(true))
    .fold(Flags::default(), |flags, (_, flag)| flags.with(flag))
}

fn frame(json: &mut SpriteJson) -> Result<Option<FrameJson>, String> {
    match (json.frame, json.duration_ms) {
        (Some(index), Some(duration_ms)) => Ok(Some(FrameJson {
            animation: json.animation.take(),
            index,
            duration_ms,
            looping: json.looping,
            play_count: json.play_count,
        })),
        (None, None) if json.animation.is_none() => Ok(None),
        _ => Err("an animation frame needs both `frame` and `durationMs`".into()),
    }
}

fn scorch(json: &mut SpriteJson) -> Result<Option<ScorchDraft>, String> {
    let fields = (
        json.eligible_stages.take(),
        json.eligible_health_states.take(),
        json.replaces.take(),
        json.shared_stem.take(),
    );
    match fields {
        (Some(stages), Some(health_states), Some(replaces), Some(shared_stem)) => {
            if json
                .reference_foliage
                .as_ref()
                .is_some_and(|r| *r != replaces)
            {
                return Err("referenceFoliage and replaces differ".into());
            }
            Ok(Some(ScorchDraft {
                eligible_stages: stages,
                eligible_states: health_states,
                replaces,
                shared_stem,
            }))
        }
        (None, None, None, None) => Ok(None),
        _ => Err("a scorch needs its eligibility, `replaces` and `sharedStem`".into()),
    }
}

fn burnoff(json: &SpriteJson) -> Result<Option<BurnoffKeys>, String> {
    match (
        json.weed_opacity_at_frame_start,
        json.fresh_soil_opacity_at_frame_start,
    ) {
        (Some(weed), Some(fresh)) => Ok(Some(BurnoffKeys {
            weed_permille: permille(weed)?,
            fresh_soil_permille: permille(fresh)?,
        })),
        (None, None) => Ok(None),
        _ => Err("burn-off needs both opacity keys".into()),
    }
}

fn glyph(json: &mut SpriteJson) -> Result<Option<(String, u32)>, String> {
    match (json.font.take(), json.codepoint) {
        (Some(font), Some(codepoint)) => {
            let character = char::from_u32(codepoint).map(String::from);
            if json.character.is_some() && json.character != character {
                return Err("character is not its codepoint".into());
            }
            Ok(Some((font, codepoint)))
        }
        (None, None) => Ok(None),
        _ => Err("a glyph needs both `font` and `codepoint`".into()),
    }
}

fn envelope(t: &EnvelopeJson) -> Result<Envelope, String> {
    let ms = |n: u32| u16::try_from(n).map_err(|_| "a transition exceeds 65535 ms".to_owned());
    Ok(Envelope {
        enter_ms: ms(t.enter_ms)?,
        hold_ms: ms(t.hold_ms)?,
        exit_ms: ms(t.exit_ms)?,
    })
}

fn point(p: PointJson) -> Option<Point> {
    Some(Point {
        x: i16::try_from(p.x).ok()?,
        y: i16::try_from(p.y).ok()?,
    })
}

fn optional_point(p: Option<PointJson>) -> Result<Option<Point>, String> {
    p.map(|p| point(p).ok_or_else(|| "a point is too far out".to_owned()))
        .transpose()
}

fn size(s: SizeJson) -> Option<Size> {
    Some(Size {
        w: u16::try_from(s.w).ok()?,
        h: u16::try_from(s.h).ok()?,
    })
}

fn rect(r: RectJson) -> Option<Rect> {
    Some(Rect {
        x: u16::try_from(r.x).ok()?,
        y: u16::try_from(r.y).ok()?,
        w: u16::try_from(r.w).ok()?,
        h: u16::try_from(r.h).ok()?,
    })
}

/// A delivered 0..=1 opacity as exact thousandths: `0.35` is 350. A value with more places
/// than that is refused rather than rounded.
fn permille(value: f64) -> Result<u16, String> {
    let scaled = value * 1000.0;
    (0..=1000_u16)
        .find(|&n| (f64::from(n) - scaled).abs() < 1e-6)
        .ok_or_else(|| format!("opacity key {value} is not a 0..=1 value in thousandths"))
}

/// `#RRGGBB`.
fn colour(hex: &str) -> Option<Rgb> {
    let hex = hex.strip_prefix('#').filter(|h| h.len() == 6)?;
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The directory part of a package path.
pub fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `relative` resolved against a package directory, or `None` if it climbs out of the package.
pub fn resolve(dir: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// The manifests the package's index declares, against those every biome loaded.
pub fn check_totals(contents: &Contents, biomes: &[Biome]) -> Vec<Failure> {
    let mut failures = Vec::new();
    let ids: usize = biomes.iter().map(|b| b.sprites.len()).sum();
    let manifests: u32 = biomes.iter().map(|b| b.manifests).sum();
    if usize::try_from(contents.active_ids) != Ok(ids) {
        failures.push(Failure::new(
            Check::Manifests,
            format!(
                "PACKAGE-CONTENTS.json declares {} active IDs, {ids} loaded",
                contents.active_ids
            ),
        ));
    }
    if contents.manifests != manifests {
        failures.push(Failure::new(
            Check::Manifests,
            format!(
                "PACKAGE-CONTENTS.json declares {} manifests, {manifests} loaded",
                contents.manifests
            ),
        ));
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_within_the_package() {
        assert_eq!(
            resolve(
                "biome-temperate/plants/exports/batch-01",
                "../../../palettes/exports/p.json"
            ),
            Some("biome-temperate/palettes/exports/p.json".into())
        );
        assert_eq!(
            resolve("biome-temperate", "plants/a.json"),
            Some("biome-temperate/plants/a.json".into())
        );
        assert_eq!(resolve("biome-temperate", "../../outside.json"), None);
        assert_eq!(dir_of("biome-temperate/a.json"), "biome-temperate");
        assert_eq!(dir_of("a.json"), "");
    }

    #[test]
    fn opacity_keys_become_exact_thousandths() {
        assert_eq!(permille(0.35), Ok(350));
        assert_eq!(permille(0.8), Ok(800));
        assert_eq!(permille(1.0), Ok(1000));
        assert_eq!(permille(0.0), Ok(0));
        assert!(permille(0.3333).is_err());
        assert!(permille(1.5).is_err());
        assert!(permille(-0.1).is_err());
    }

    #[test]
    fn colours_are_hash_hex() {
        assert_eq!(colour("#292720"), Some([0x29, 0x27, 0x20]));
        assert_eq!(colour("292720"), None);
        assert_eq!(colour("#2927"), None);
        assert_eq!(colour("#29272G"), None);
    }
}
