//! The art catalog: every sprite the package delivers, under its semantic ID, with the metadata
//! the composer draws by.
//!
//! `cargo xtask assets` builds it from the delivered manifests and packs every sprite's pixels
//! into one atlas. Each atlas byte is the sprite's raw PNG index: 0 is transparent and 1..=32
//! is logical slot + 1. Both files are committed under `assets/generated/` and embedded here.
//! This module is the format's only home, so xtask encodes with the same types the app decodes.

use std::fmt;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Opaque palette slots; PNG index 0 is transparency, so indices run to this value.
pub const PALETTE_SLOTS: usize = 32;
/// The largest atlas side the packer may produce.
pub const MAX_ATLAS_SIDE: u16 = 2048;
/// Bumped whenever the encoded layout changes, so a stale file is refused, not misread.
pub const FORMAT: u32 = 1;
const MAGIC: [u8; 4] = *b"TRCA";

const EMBEDDED_CATALOG: &[u8] = include_bytes!("../assets/generated/temperate/catalog.bin");
const EMBEDDED_ATLAS: &[u8] = include_bytes!("../assets/generated/temperate/atlas.bin");

/// A sprite, by its place in the catalog's ID-sorted list. Valid only for the catalog it came
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SpriteId(u16);

impl SpriteId {
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    pub fn from_index(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(SpriteId)
    }
}

/// An animation, by its place in the catalog's name-sorted list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AnimationId(u16);

impl AnimationId {
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    pub fn from_index(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(AnimationId)
    }
}

/// A point in a sprite's own pixel space. Anchors and pivots may lie outside the opaque
/// pixels, and a foliage attachment can sit above the stem's rect, so both axes are signed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: i16,
    pub y: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Size {
    pub w: u16,
    pub h: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

pub type Rgb = [u8; 3];

/// Each logical slot's day and night colour. Night swaps slots before compositing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub day: [Rgb; PALETTE_SLOTS],
    pub night: [Rgb; PALETTE_SLOTS],
}

/// The depth layers, back to front. `SoilPanel` has no sprite of its own; the UI sprites and
/// glyphs, which carry no layer, are drawn there or on the label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Layer {
    Sky,
    FarPlants,
    Monuments,
    MidPlants,
    WeedsFungus,
    PlantGlow,
    NearPlants,
    ResidentsGround,
    ResidentsAir,
    GroundLitter,
    Soil,
    Ground,
    EventOverlays,
    Glass,
    Label,
    SoilPanel,
}

impl Layer {
    /// Every layer, back to front.
    pub const ALL: [Layer; 16] = [
        Layer::Sky,
        Layer::FarPlants,
        Layer::Monuments,
        Layer::MidPlants,
        Layer::WeedsFungus,
        Layer::PlantGlow,
        Layer::NearPlants,
        Layer::ResidentsGround,
        Layer::ResidentsAir,
        Layer::GroundLitter,
        Layer::Soil,
        Layer::Ground,
        Layer::EventOverlays,
        Layer::Glass,
        Layer::Label,
        Layer::SoilPanel,
    ];

    /// The manifests' name for the layer.
    pub const fn art_name(self) -> &'static str {
        match self {
            Layer::Sky => "sky",
            Layer::FarPlants => "far-plants",
            Layer::Monuments => "monuments",
            Layer::MidPlants => "mid-plants",
            Layer::WeedsFungus => "weeds-fungus",
            Layer::PlantGlow => "plant-glow",
            Layer::NearPlants => "near-plants",
            Layer::ResidentsGround => "residents-ground",
            Layer::ResidentsAir => "residents-air",
            Layer::GroundLitter => "ground-litter",
            Layer::Soil => "soil",
            Layer::Ground => "ground",
            Layer::EventOverlays => "event-overlays",
            Layer::Glass => "glass",
            Layer::Label => "label",
            Layer::SoilPanel => "soil-panel",
        }
    }

    pub fn from_art_name(name: &str) -> Option<Layer> {
        Layer::ALL
            .into_iter()
            .find(|layer| layer.art_name() == name)
    }
}

/// How a sprite meets what is below it; a missing `blend` is normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Add,
}

impl Blend {
    pub fn from_art_name(name: &str) -> Option<Blend> {
        match name {
            "normal" => Some(Blend::Normal),
            "multiply" => Some(Blend::Multiply),
            "add" => Some(Blend::Add),
            _ => None,
        }
    }
}

/// A sprite's yes-or-no placement rules, as one bit set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Flags(u8);

impl Flags {
    /// Repeats horizontally across the scene.
    pub const TILE_X: Flags = Flags(1);
    pub const DAY_ONLY: Flags = Flags(1 << 1);
    pub const NIGHT_ONLY: Flags = Flags(1 << 2);
    /// The final frame of a one-shot that persists: the settled crack and sinkhole.
    pub const SETTLED: Flags = Flags(1 << 3);

    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn with(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }
}

/// The structured fields the art is looked up by: never split an ID to find them.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ArtKeys {
    pub species: Option<String>,
    pub stage: Option<String>,
    pub state: Option<String>,
    pub part: Option<String>,
    pub life_stage: Option<String>,
    pub event: Option<String>,
}

/// A creature frame's world movement, applied after its hold, and its ground contacts.
/// Airborne frames have none.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameMotion {
    pub root_delta: Point,
    pub ground_contacts: Vec<Point>,
}

/// A finite overlay's envelope, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Envelope {
    pub enter_ms: u16,
    pub hold_ms: u16,
    pub exit_ms: u16,
}

/// A lightning scorch: a temporary replacement for a mature plant's foliage over its shared
/// stem, never a health state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scorch {
    pub eligible_stages: Vec<String>,
    pub eligible_states: Vec<String>,
    /// The foliage it stands in for.
    pub replaces: SpriteId,
    pub shared_stem: SpriteId,
}

/// Burn-off's opacity keys at the start of this frame, in thousandths, exactly as delivered
/// (`0.35` is 350).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BurnoffKeys {
    pub weed_permille: u16,
    pub fresh_soil_permille: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sprite {
    /// The semantic ID, `tmp.` prefix kept.
    pub id: String,
    /// Where the pixels sit in the runtime atlas.
    pub rect: Rect,
    /// Draw at `(world − anchor)`.
    pub anchor: Point,
    pub layer: Option<Layer>,
    pub blend: Blend,
    /// A missing `opacity` is 255.
    pub opacity: u8,
    pub flags: Flags,
    pub keys: ArtKeys,
    pub attachment_point: Option<Point>,
    /// Foliage shears about this point, never about its rect's centre.
    pub shear_pivot: Option<Point>,
    /// On a stem: where its foliage attaches.
    pub foliage_attachment: Option<Point>,
    pub motion: Option<FrameMotion>,
    /// A night glow's host sprite.
    pub paired_with: Option<SpriteId>,
    /// An attached effect's host frame (the firefly lantern's signal frame).
    pub attachment: Option<SpriteId>,
    pub targets: Vec<SpriteId>,
    /// A light overlay's crossfade.
    pub transition_ms: Option<u16>,
    pub envelope: Option<Envelope>,
    pub scorch: Option<Scorch>,
    pub burnoff: Option<BurnoffKeys>,
    /// A plate's text area.
    pub text_inset: Option<Rect>,
}

/// What follows an animation's last frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Next {
    /// Hand off to this routine: when a finite routine ends, or when a looping one is
    /// deselected and has finished its cycle.
    Handoff(AnimationId),
    /// Hold the last frame until something else moves on: the creature holds (egg, pupa,
    /// death, hide) and the settled crack and sinkhole.
    Hold,
    /// Remove the overlay after its last frame: burn-off's ash, lightning, the lantern.
    Clear,
    /// A loop with no destination, repeated while selected.
    Repeat,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnimationKeys {
    pub species: Option<String>,
    pub life_stage: Option<String>,
    /// The routine, which the manifests call `state`: `move`, `hatch`, `death`…
    pub routine: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Animation {
    /// The manifests' name: `snail.adult.move`, `rain`, `burnoff`.
    pub name: String,
    pub keys: AnimationKeys,
    pub frames: Vec<SpriteId>,
    /// One per frame. Authored durations govern animation time, never the frame rate.
    pub durations_ms: Vec<u16>,
    pub looping: bool,
    /// 0 while looping.
    pub play_count: u8,
    pub next: Next,
    /// Visibility and placement tweens.
    pub transition_ms: Option<u16>,
    /// The sum of the frames' `root_delta.x`.
    pub cycle_displacement: i16,
    /// An attached effect plays in step with this routine.
    pub sync: Option<AnimationId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Font {
    pub name: String,
    pub cell: Size,
    pub advance: u8,
    pub ascent: u8,
    pub descent: u8,
    pub line_height: u8,
    /// The codepoint drawn for a character the font lacks.
    pub fallback: u32,
    /// Sorted by codepoint.
    pub glyphs: Vec<(u32, SpriteId)>,
}

impl Font {
    pub fn glyph(&self, codepoint: u32) -> Option<SpriteId> {
        self.glyphs
            .binary_search_by_key(&codepoint, |(c, _)| *c)
            .ok()
            .map(|i| self.glyphs[i].1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoverPlate {
    pub size: Size,
    pub safe_text: Rect,
    pub max_body_characters: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NineSlice {
    pub inset: u8,
    pub tile_size: u8,
    pub minimum: Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ui {
    pub hover_plate: HoverPlate,
    pub nine_slice: NineSlice,
}

/// The residents' shared budget, as the two creature manifests propose it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Residents {
    pub moving: u8,
    pub flying_or_signalling: u8,
    pub render_cap_fps: u8,
    pub transition_ms: u16,
    /// Sorted by species.
    pub phase_offsets_ms: Vec<(String, u16)>,
}

/// A group of existing sprites an event reuses, from the events manifest's `reusedActiveIds`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventGroup {
    pub name: String,
    pub sprites: Vec<SpriteId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub biome: String,
    pub atlas: Size,
    pub palette: Palette,
    /// Sorted by ID; a `SpriteId` is an index into it.
    pub sprites: Vec<Sprite>,
    /// Sorted by name; an `AnimationId` is an index into it.
    pub animations: Vec<Animation>,
    pub fonts: Vec<Font>,
    pub ui: Ui,
    pub residents: Residents,
    /// Sorted by name.
    pub events: Vec<EventGroup>,
}

impl Catalog {
    pub fn id(&self, id: &str) -> Option<SpriteId> {
        self.sprites
            .binary_search_by(|sprite| sprite.id.as_str().cmp(id))
            .ok()
            .and_then(SpriteId::from_index)
    }

    /// # Panics
    /// If `id` is not from this catalog.
    pub fn sprite(&self, id: SpriteId) -> &Sprite {
        &self.sprites[id.index()]
    }

    pub fn animation_id(&self, name: &str) -> Option<AnimationId> {
        self.animations
            .binary_search_by(|animation| animation.name.as_str().cmp(name))
            .ok()
            .and_then(AnimationId::from_index)
    }

    /// # Panics
    /// If `id` is not from this catalog.
    pub fn animation(&self, id: AnimationId) -> &Animation {
        &self.animations[id.index()]
    }

    pub fn font(&self, name: &str) -> Option<&Font> {
        self.fonts.iter().find(|font| font.name == name)
    }

    pub fn event_group(&self, name: &str) -> Option<&EventGroup> {
        self.events.iter().find(|group| group.name == name)
    }

    /// The catalog file: a magic value, the format version, then the catalog in postcard.
    ///
    /// # Panics
    /// Never in practice: postcard only fails to encode into a full fixed-size buffer.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(&(MAGIC, FORMAT, self)).expect("encoding into a Vec cannot fail")
    }

    /// Reads a catalog file and checks it against its atlas.
    pub fn decode(bytes: &[u8], atlas: &[u8]) -> Result<Catalog, DecodeError> {
        let (magic, rest): ([u8; 4], &[u8]) =
            postcard::take_from_bytes(bytes).map_err(|_| DecodeError::Magic)?;
        if magic != MAGIC {
            return Err(DecodeError::Magic);
        }
        let (format, rest): (u32, &[u8]) =
            postcard::take_from_bytes(rest).map_err(|_| DecodeError::Magic)?;
        if format != FORMAT {
            return Err(DecodeError::Format(format));
        }
        let (catalog, rest): (Catalog, &[u8]) =
            postcard::take_from_bytes(rest).map_err(|err| DecodeError::Body(err.to_string()))?;
        if !rest.is_empty() {
            return Err(DecodeError::Trailing(rest.len()));
        }
        catalog.check(atlas)?;
        Ok(catalog)
    }

    /// What the types cannot hold: the atlas matches its size, every rect fits, the lists are
    /// sorted, and every reference points inside the catalog.
    fn check(&self, atlas: &[u8]) -> Result<(), DecodeError> {
        let Size { w, h } = self.atlas;
        if w > MAX_ATLAS_SIDE
            || h > MAX_ATLAS_SIDE
            || atlas.len() != usize::from(w) * usize::from(h)
        {
            return Err(DecodeError::Atlas {
                size: self.atlas,
                bytes: atlas.len(),
            });
        }
        let invalid = |what: String| Err(DecodeError::Invalid(what));
        if !self.sprites.windows(2).all(|pair| pair[0].id < pair[1].id) {
            return invalid("sprites are not sorted by unique ID".into());
        }
        if !self
            .animations
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
        {
            return invalid("animations are not sorted by unique name".into());
        }
        let sprite_ok = |id: SpriteId| id.index() < self.sprites.len();
        let animation_ok = |id: AnimationId| id.index() < self.animations.len();
        for sprite in &self.sprites {
            let Rect { x, y, w: sw, h: sh } = sprite.rect;
            if u32::from(x) + u32::from(sw) > u32::from(w)
                || u32::from(y) + u32::from(sh) > u32::from(h)
            {
                return invalid(format!("{} lies outside the atlas", sprite.id));
            }
            let scorch = sprite
                .scorch
                .iter()
                .flat_map(|s| [s.replaces, s.shared_stem]);
            let linked = sprite.paired_with.iter().chain(&sprite.attachment).copied();
            if !linked
                .chain(sprite.targets.iter().copied())
                .chain(scorch)
                .all(sprite_ok)
            {
                return invalid(format!("{} refers past the sprites", sprite.id));
            }
        }
        for animation in &self.animations {
            let next = match animation.next {
                Next::Handoff(id) => Some(id),
                _ => None,
            };
            if !animation.frames.iter().copied().all(sprite_ok)
                || !next
                    .iter()
                    .chain(&animation.sync)
                    .copied()
                    .all(animation_ok)
                || animation.frames.len() != animation.durations_ms.len()
            {
                return invalid(format!("{} is malformed", animation.name));
            }
        }
        // `Font::glyph` binary-searches, so each font's codepoints must be sorted and unique.
        if let Some(font) = self
            .fonts
            .iter()
            .find(|font| !font.glyphs.windows(2).all(|pair| pair[0].0 < pair[1].0))
        {
            return invalid(format!(
                "{}'s glyphs are not sorted by unique codepoint",
                font.name
            ));
        }
        // Text steps by the advance and panels by the tile, so a zero would divide by zero or
        // never finish laying out.
        if let Some(font) = self.fonts.iter().find(|font| font.advance == 0) {
            return invalid(format!("{}'s advance is 0", font.name));
        }
        if self.ui.nine_slice.tile_size == 0 {
            return invalid("the nine-slice tile size is 0".into());
        }
        let glyphs = self
            .fonts
            .iter()
            .flat_map(|font| font.glyphs.iter().map(|(_, id)| *id));
        let grouped = self
            .events
            .iter()
            .flat_map(|group| group.sprites.iter().copied());
        if !glyphs.chain(grouped).all(sprite_ok) {
            return invalid("a font or event group refers past the sprites".into());
        }
        Ok(())
    }
}

/// Why a catalog file was refused. For developers only; the embedded one is checked by
/// `cargo xtask verify`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Magic,
    Format(u32),
    Body(String),
    Trailing(usize),
    Atlas { size: Size, bytes: usize },
    Invalid(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Magic => write!(f, "not a catalog file"),
            DecodeError::Format(found) => {
                write!(
                    f,
                    "catalog format {found}, expected {FORMAT}; run cargo xtask assets"
                )
            }
            DecodeError::Body(err) => write!(f, "catalog does not decode: {err}"),
            DecodeError::Trailing(n) => write!(f, "{n} bytes follow the catalog"),
            DecodeError::Atlas { size, bytes } => {
                write!(f, "a {}×{} atlas cannot be {bytes} bytes", size.w, size.h)
            }
            DecodeError::Invalid(what) => write!(f, "catalog is invalid: {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A catalog with the atlas its rects point into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Art<'a> {
    pub catalog: Catalog,
    pub atlas: &'a [u8],
}

impl<'a> Art<'a> {
    pub fn decode(catalog: &[u8], atlas: &'a [u8]) -> Result<Art<'a>, DecodeError> {
        Ok(Art {
            catalog: Catalog::decode(catalog, atlas)?,
            atlas,
        })
    }

    /// A fresh decode of the embedded art: the path startup takes, and what the bench times.
    pub fn load_embedded() -> Result<Art<'static>, DecodeError> {
        Art::decode(EMBEDDED_CATALOG, EMBEDDED_ATLAS)
    }

    /// The temperate biome's art, decoded once.
    ///
    /// # Panics
    /// If the committed files do not decode, which `cargo xtask verify` rules out.
    pub fn embedded() -> &'static Art<'static> {
        static ART: OnceLock<Art<'static>> = OnceLock::new();
        ART.get_or_init(|| {
            Art::load_embedded().unwrap_or_else(|err| panic!("the embedded art: {err}"))
        })
    }

    /// The sprite's pixel rows: raw PNG indices, 0 transparent.
    pub fn rows(&self, id: SpriteId) -> impl Iterator<Item = &'a [u8]> + use<'a> {
        let Rect { x, y, w, h } = self.catalog.sprite(id).rect;
        let stride = usize::from(self.catalog.atlas.w);
        let atlas = self.atlas;
        (usize::from(y)..usize::from(y) + usize::from(h)).map(move |row| {
            let start = row * stride + usize::from(x);
            &atlas[start..start + usize::from(w)]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_art_decodes() {
        let art = Art::embedded();
        assert_eq!(art.catalog.biome, "temperate");
        assert!(!art.catalog.sprites.is_empty());
    }

    #[test]
    fn the_embedded_catalog_re_encodes_to_its_own_bytes() {
        // A layout change without a FORMAT bump would decode differently, or not round-trip.
        assert_eq!(Art::embedded().catalog.encode(), EMBEDDED_CATALOG);
    }

    #[test]
    fn a_wrong_magic_or_format_is_refused() {
        let catalog = &Art::embedded().catalog;
        let mut bytes = catalog.encode();
        bytes[0] ^= 1;
        assert_eq!(
            Catalog::decode(&bytes, EMBEDDED_ATLAS),
            Err(DecodeError::Magic)
        );

        let stale = postcard::to_allocvec(&(MAGIC, FORMAT + 1, catalog)).unwrap();
        assert_eq!(
            Catalog::decode(&stale, EMBEDDED_ATLAS),
            Err(DecodeError::Format(FORMAT + 1))
        );

        let mut trailing = catalog.encode();
        trailing.push(0);
        assert_eq!(
            Catalog::decode(&trailing, EMBEDDED_ATLAS),
            Err(DecodeError::Trailing(1))
        );
    }

    #[test]
    fn unsorted_glyphs_are_refused() {
        let mut catalog = Art::embedded().catalog.clone();
        catalog.fonts[0].glyphs.swap(0, 1);
        assert!(matches!(
            Catalog::decode(&catalog.encode(), EMBEDDED_ATLAS),
            Err(DecodeError::Invalid(what)) if what.contains("glyphs are not sorted")
        ));
    }

    #[test]
    fn a_zero_advance_or_tile_size_is_refused() {
        let mut catalog = Art::embedded().catalog.clone();
        catalog.fonts[0].advance = 0;
        assert!(matches!(
            Catalog::decode(&catalog.encode(), EMBEDDED_ATLAS),
            Err(DecodeError::Invalid(what)) if what.contains("advance is 0")
        ));
        let mut catalog = Art::embedded().catalog.clone();
        catalog.ui.nine_slice.tile_size = 0;
        assert!(matches!(
            Catalog::decode(&catalog.encode(), EMBEDDED_ATLAS),
            Err(DecodeError::Invalid(what)) if what.contains("tile size is 0")
        ));
    }

    #[test]
    fn an_atlas_of_the_wrong_length_is_refused() {
        let short = &EMBEDDED_ATLAS[1..];
        assert!(matches!(
            Catalog::decode(EMBEDDED_CATALOG, short),
            Err(DecodeError::Atlas { .. })
        ));
    }

    #[test]
    fn every_rect_lies_inside_the_atlas_and_every_row_has_its_width() {
        let art = Art::embedded();
        for (index, sprite) in art.catalog.sprites.iter().enumerate() {
            let id = SpriteId::from_index(index).unwrap();
            let rows: Vec<&[u8]> = art.rows(id).collect();
            assert_eq!(rows.len(), usize::from(sprite.rect.h), "{}", sprite.id);
            assert!(
                rows.iter()
                    .all(|row| row.len() == usize::from(sprite.rect.w))
            );
        }
    }

    #[test]
    fn ids_and_names_are_found_by_search() {
        let catalog = &Art::embedded().catalog;
        let id = catalog.id("tmp.ui.label.plate").expect("the label plate");
        assert_eq!(catalog.sprite(id).id, "tmp.ui.label.plate");
        assert_eq!(catalog.id("tmp.nothing"), None);
        let walk = catalog
            .animation_id("snail.adult.move")
            .expect("the snail walks");
        assert_eq!(
            catalog.animation(walk).keys.routine.as_deref(),
            Some("move")
        );
        assert_eq!(catalog.animation_id("snail.adult.fly"), None);
    }

    #[test]
    fn every_pixel_is_transparent_or_a_palette_slot() {
        let max = u8::try_from(PALETTE_SLOTS).unwrap();
        assert!(Art::embedded().atlas.iter().all(|&index| index <= max));
    }

    #[test]
    fn fonts_map_codepoints_to_glyphs() {
        let catalog = &Art::embedded().catalog;
        let body = catalog.font("body-4x6").expect("the body font");
        let a = body.glyph(u32::from('A')).expect("an A");
        assert_eq!(catalog.sprite(a).keys.part.as_deref(), Some("glyph"));
        assert!(body.glyph(body.fallback).is_some());
        assert_eq!(body.glyph(0x2603), None);
    }

    #[test]
    fn layers_keep_the_depth_order_and_their_art_names() {
        assert!(Layer::ALL.windows(2).all(|pair| pair[0] < pair[1]));
        for layer in Layer::ALL {
            assert_eq!(Layer::from_art_name(layer.art_name()), Some(layer));
        }
        assert_eq!(Layer::from_art_name("sky"), Some(Layer::Sky));
        assert_eq!(Layer::from_art_name("attic"), None);
    }

    #[test]
    fn flags_combine() {
        let flags = Flags::default().with(Flags::TILE_X).with(Flags::NIGHT_ONLY);
        assert!(flags.contains(Flags::TILE_X) && flags.contains(Flags::NIGHT_ONLY));
        assert!(!flags.contains(Flags::DAY_ONLY));
    }
}
