//! The catalog, assembled from a biome's checked manifests and sequences. Rects and the atlas
//! size stay zero until the packer fills them, so the variant check (6) can run on the
//! catalog before any pixels are placed.

use std::cell::RefCell;
use std::collections::BTreeMap;

use terrarium_compose::catalog::{
    Animation, AnimationId, Catalog, EventGroup, Font, FrameMotion, Next, Rect, Scorch, Size,
    Sprite, SpriteId, Ui,
};

use super::animation::{AnimationDraft, NextDraft};
use super::manifest::{Biome, FontDraft, SpriteDraft};

/// The catalog, and every reference that did not resolve. A reference that fails to resolve
/// has already failed check 2 or 5, so the list only matters once those pass.
pub fn catalog(biome: &Biome, drafts: &[AnimationDraft]) -> (Catalog, Vec<String>) {
    let ids = Resolver {
        sprites: biome
            .sprites
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((s.id.as_str(), SpriteId::from_index(i)?)))
            .collect(),
        animations: drafts
            .iter()
            .enumerate()
            .filter_map(|(i, d)| Some((d.name.as_str(), AnimationId::from_index(i)?)))
            .collect(),
        unresolved: RefCell::new(Vec::new()),
    };
    let catalog = Catalog {
        biome: biome.name.clone(),
        atlas: Size::default(),
        palette: biome.palette,
        sprites: biome
            .sprites
            .iter()
            .map(|draft| convert(draft, &ids))
            .collect(),
        animations: drafts.iter().map(|draft| animation(draft, &ids)).collect(),
        fonts: biome
            .fonts
            .iter()
            .map(|font| self::font(font, biome, &ids))
            .collect(),
        ui: Ui {
            hover_plate: biome.hover_plate,
            nine_slice: biome.nine_slice,
        },
        residents: biome.residents.clone(),
        events: biome
            .groups
            .iter()
            .map(|(name, members)| EventGroup {
                name: name.clone(),
                sprites: members
                    .iter()
                    .filter_map(|id| ids.sprite(id, name))
                    .collect(),
            })
            .collect(),
    };
    (catalog, ids.unresolved.into_inner())
}

/// Names to indices, noting each name that has none.
struct Resolver<'a> {
    sprites: BTreeMap<&'a str, SpriteId>,
    animations: BTreeMap<&'a str, AnimationId>,
    unresolved: RefCell<Vec<String>>,
}

impl Resolver<'_> {
    fn sprite(&self, id: &str, from: &str) -> Option<SpriteId> {
        let found = self.sprites.get(id).copied();
        if found.is_none() {
            self.unresolved
                .borrow_mut()
                .push(format!("{from} names the sprite {id}"));
        }
        found
    }

    fn animation(&self, name: &str, from: &str) -> Option<AnimationId> {
        let found = self.animations.get(name).copied();
        if found.is_none() {
            self.unresolved
                .borrow_mut()
                .push(format!("{from} names the animation {name}"));
        }
        found
    }
}

fn animation(draft: &AnimationDraft, ids: &Resolver) -> Animation {
    let from = draft.name.as_str();
    let next = match &draft.next {
        NextDraft::Handoff(name) => ids.animation(name, from).map_or(Next::Hold, Next::Handoff),
        NextDraft::Hold => Next::Hold,
        NextDraft::Clear => Next::Clear,
        NextDraft::Repeat => Next::Repeat,
    };
    Animation {
        name: draft.name.clone(),
        keys: draft.keys.clone(),
        frames: draft
            .frames
            .iter()
            .filter_map(|id| ids.sprite(id, from))
            .collect(),
        durations_ms: draft.durations_ms.clone(),
        looping: draft.looping,
        play_count: draft.play_count,
        next,
        transition_ms: draft.transition_ms,
        cycle_displacement: draft.cycle_displacement,
        sync: draft
            .sync
            .as_deref()
            .and_then(|name| ids.animation(name, from)),
    }
}

fn font(font: &FontDraft, biome: &Biome, ids: &Resolver) -> Font {
    let mut glyphs: Vec<(u32, SpriteId)> = biome
        .sprites
        .iter()
        .filter_map(|s| match &s.glyph {
            Some((name, codepoint)) if *name == font.name => {
                Some((*codepoint, ids.sprite(&s.id, &font.name)?))
            }
            _ => None,
        })
        .collect();
    glyphs.sort_unstable();
    Font {
        name: font.name.clone(),
        cell: font.cell,
        advance: font.advance,
        ascent: font.ascent,
        descent: font.descent,
        line_height: font.line_height,
        fallback: font.fallback,
        glyphs,
    }
}

fn convert(draft: &SpriteDraft, ids: &Resolver) -> Sprite {
    let from = draft.id.as_str();
    let sprite = |id: &str, from: &str| ids.sprite(id, from);
    let scorch = draft.scorch.as_ref().and_then(|s| {
        Some(Scorch {
            eligible_stages: s.eligible_stages.clone(),
            eligible_states: s.eligible_states.clone(),
            replaces: sprite(&s.replaces, from)?,
            shared_stem: sprite(&s.shared_stem, from)?,
        })
    });
    Sprite {
        id: draft.id.clone(),
        rect: Rect {
            w: draft.rect.w,
            h: draft.rect.h,
            ..Rect::default()
        },
        anchor: draft.anchor,
        layer: draft.layer,
        blend: draft.blend,
        opacity: draft.opacity,
        flags: draft.flags,
        keys: draft.keys.clone(),
        attachment_point: draft.attachment_point,
        shear_pivot: draft.shear_pivot,
        foliage_attachment: draft.foliage_attachment,
        motion: draft.root_delta.map(|root_delta| FrameMotion {
            root_delta,
            ground_contacts: draft.ground_contacts.clone().unwrap_or_default(),
        }),
        paired_with: draft.paired_with.as_deref().and_then(|id| sprite(id, from)),
        attachment: draft.attachment.as_deref().and_then(|id| sprite(id, from)),
        targets: draft
            .targets
            .iter()
            .filter_map(|id| sprite(id, from))
            .collect(),
        transition_ms: draft.transition_ms,
        envelope: draft.envelope,
        scorch,
        burnoff: draft.burnoff,
        text_inset: draft.text_inset,
    }
}
