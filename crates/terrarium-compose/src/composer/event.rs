//! The foreground event's choreography in the sky: the storm front and the clearing, lightning,
//! dusk and sunburst, rain after drought and the windstorm. The ground's events are their own.
//!
//! Truth puts an event in the first scene that carries its effects, and the engine keeps it in
//! the scenes after, so an instance, its kind and its tick, plays once: from the update that
//! first carries it, or, while another still plays, not at all, its state arriving through the
//! ordinary tweens. One plays at a time.
//!
//! - **Storm and clearing** wipe the sky from the event's side. The storm front's curtain
//!   crosses at a pixel a frame at most, the new sky behind it and the old ahead, its opaque
//!   columns toward the storm: a storm rolls in, and a clearing carries it on across and out
//!   the far side. A wipe plays only where the weather crosses its way, and while it plays
//!   every other change of the sky waits for it to pass.
//! - **Lightning** plays its two frames above the struck plant, or strikes the ground where no
//!   struck plant is drawn. The scorch that follows is the plant's.
//! - **Dusk and sunburst** are the light's own crossfade.
//! - **Rain after drought** falls across the viewport, fading in, holding and fading out.
//! - **A windstorm** carries leaves across from its side.
//!
//! Each plays at its own pace in dim mode too. Residents and sway answer an event as every
//! change of motion does, from the next turn: storm weather sends residents into hiding and
//! clearing brings them out, and while the scene carries a storm the plants sway a step
//! harder, in a windstorm at a gale.

use terrarium_truth::Tick;
use terrarium_truth::scene::{
    ActiveEvent, EventKind, PlantView, SceneInput, Side, Strength, Weather, Wind,
};

use super::{Composer, Depicted, Earliest, Entry, FRAME_HEIGHT, Lit, Look, Piece, SHOWN, standing};
use crate::anim;
use crate::catalog::{AnimationId, Art, SpriteId};
use crate::frame::{Area, Draw, PLANT_GROUND_Y};
use crate::motion::motion_hash;
use crate::order::{KeyedDraw, Place, Subject};
use crate::salt::Purpose;
use crate::schedule::MIN_FRAME_MS;
use crate::tween::durations::SKY_MS;
use crate::tween::{Millis, Tween};
use crate::variants::{ArtTable, PlantArt, SkyEventArt};
use crate::world;

/// The storm front's curtain is opaque on every row from its storm side's first column through
/// this one, so the line between the new sky and the old always lies under it.
const SEAM: i32 = 8;
/// Rain after drought fades in and out over this long, holding between for longer the longer
/// the drought: by strength, low to high.
const RAIN_FADE_MS: u32 = 3000;
const RAIN_HOLD_MS: [u32; 3] = [12_000, 24_000, 36_000];
/// A windstorm's leaves, by strength. One sets off every so often, and each crosses at a pace
/// of its own, a pixel in so many milliseconds.
const LEAVES: [u8; 3] = [6, 9, 12];
const LEAF_EVERY_MS: u32 = 900;
const LEAF_MS_PER_PX: [u64; 2] = [16, 34];
/// The rows a leaf flies along, below the label and clear of the ground.
const LEAF_TOP: i32 = 14;
const LEAF_ROWS: u64 = 40;
/// A leaf bobs a pixel at a time, up two and down two.
const WOBBLE: [i32; 8] = [0, 1, 2, 1, 0, -1, -2, -1];
const WOBBLE_STEP_MS: u64 = 250;
/// The salted index of the ground lightning strikes without a plant, apart from any leaf's.
const STRIKE: u64 = 0xff;

/// One instance of an event: its kind and the tick it started in.
pub(super) type Instance = (EventKind, Tick);

/// Whether an event plays in the sky here.
const fn in_the_sky(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Storm
            | EventKind::Clearing
            | EventKind::Lightning
            | EventKind::Dusk
            | EventKind::Sunburst
            | EventKind::RainAfterDrought
            | EventKind::Windstorm
    )
}

const fn by_strength<T: Copy>(values: [T; 3], strength: Strength) -> T {
    match strength {
        Strength::Low => values[0],
        Strength::Mid => values[1],
        Strength::High => values[2],
    }
}

/// The event playing: what it is, how it plays, and when it is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Show {
    event: ActiveEvent,
    play: Play,
    end: Millis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Play {
    Wipe(Wipe),
    /// Lightning from `start`, above the struck plant as it stood in its cell when the strike
    /// began, so no later update moves the bolt.
    Bolt {
        start: Millis,
        struck: Option<(u8, PlantView)>,
    },
    /// The light's crossfade, which its own look plays.
    Light,
    Rain {
        start: Millis,
    },
    /// The flurry, laid out across the viewport as wide as it was when it began.
    Leaves {
        start: Millis,
        width: i32,
    },
}

impl Show {
    /// Whether anything of it shows or moves at `now`.
    fn playing(&self, now: Millis) -> bool {
        let from = match self.play {
            Play::Wipe(wipe) => wipe.begin,
            Play::Bolt { start, .. } | Play::Rain { start } | Play::Leaves { start, .. } => start,
            Play::Light => return false,
        };
        from <= now && now < self.end
    }
}

/// The storm front crossing the sky: carrying a storm in, or carrying one off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Wipe {
    storm: bool,
    side: Side,
    begin: Millis,
    /// Fixed at its begin: a pixel a frame across the viewport and the curtain as they were.
    ms: u64,
}

/// The curtain as delivered: its width, and how many columns on its left and right are empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Curtain {
    width: i32,
    open: (i32, i32),
}

impl Curtain {
    fn of(art: &Art, front: SpriteId) -> Curtain {
        Curtain {
            width: i32::from(art.catalog.sprite(front).rect.w),
            open: world::blank_columns(art, front),
        }
    }
}

/// Where a wipe stands: the curtain's left column and facing, and the line between the skies,
/// the new one on the event's side of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placing {
    x: i32,
    flip_x: bool,
    line: i32,
    new_on_left: bool,
}

impl Placing {
    /// The columns of the new sky, and of the old.
    fn columns(self) -> [(i32, i32); 2] {
        let (left, right) = ((i32::MIN, self.line), (self.line, i32::MAX));
        if self.new_on_left {
            [left, right]
        } else {
            [right, left]
        }
    }
}

impl Wipe {
    fn new(storm: bool, side: Side, begin: Millis, width: i32, front_width: i32) -> Wipe {
        let span = u64::try_from(width + front_width).unwrap_or(0);
        Wipe {
            storm,
            side,
            begin,
            ms: span * MIN_FRAME_MS,
        }
    }

    pub(super) fn end(self) -> Millis {
        Millis(self.begin.0 + self.ms)
    }

    /// The curtain's visible columns as it is placed moving right, and how far it travels to
    /// cross a viewport `width` wide, from just out of sight on one side to the other.
    fn path(self, width: i32, curtain: Curtain) -> (i32, i32) {
        // Toward a storm it faces as delivered, its storm columns behind it; carrying one off,
        // mirrored, its storm columns ahead.
        let (first, last) = if self.storm {
            (curtain.open.0, curtain.width - curtain.open.1)
        } else {
            (curtain.open.1, curtain.width - curtain.open.0)
        };
        (-last, width + last - first)
    }

    /// How many columns of `span` it has crossed at `now`.
    fn crossed(self, now: Millis, span: i32) -> i32 {
        let elapsed = now.0.saturating_sub(self.begin.0).min(self.ms);
        let span = u64::try_from(span).unwrap_or(0);
        i32::try_from(elapsed * span / self.ms.max(1)).unwrap_or(i32::MAX)
    }

    fn placing(self, now: Millis, width: i32, curtain: Curtain) -> Placing {
        let (from, span) = self.path(width, curtain);
        let x = from + self.crossed(now, span);
        let line = if self.storm {
            x + SEAM
        } else {
            x + curtain.width - SEAM
        };
        match self.side {
            Side::Left => Placing {
                x,
                flip_x: !self.storm,
                line,
                new_on_left: true,
            },
            Side::Right => Placing {
                x: width - x - curtain.width,
                flip_x: self.storm,
                line: width - line,
                new_on_left: false,
            },
        }
    }

    /// The moment after `now` at which the curtain next moves a column; `None` once across.
    fn next_step(self, now: Millis, width: i32, curtain: Curtain) -> Option<Millis> {
        let span = self.path(width, curtain).1;
        let crossed = self.crossed(now, span);
        let span = u64::try_from(span).ok().filter(|&span| span > 0)?;
        let next = u64::try_from(crossed).ok()? + 1;
        (next <= span).then(|| Millis(self.begin.0 + (next * self.ms).div_ceil(span)))
    }
}

/// `draw` cut to the columns `from..to` of a viewport `width` wide, or `None` if nothing it
/// shows there lies between them. A cut that keeps all it shows is no cut.
fn cut(draw: Draw, sprite_width: i32, (from, to): (i32, i32), width: i32) -> Option<Draw> {
    let (left, right) = (draw.x.max(0), (draw.x + sprite_width).min(width));
    if left >= right {
        return Some(draw);
    }
    let (lo, hi) = (from.max(left), to.min(right));
    if lo >= hi {
        return None;
    }
    if (lo, hi) == (left, right) {
        return Some(draw);
    }
    let area = Area {
        x: lo,
        y: 0,
        w: hi - lo,
        h: FRAME_HEIGHT,
    };
    let clip = match draw.clip {
        Some(clip) => {
            let (x, y) = (clip.x.max(area.x), clip.y.max(area.y));
            let w = (clip.x + clip.w).min(area.x + area.w) - x;
            let h = (clip.y + clip.h).min(area.y + area.h) - y;
            if w <= 0 || h <= 0 {
                return None;
            }
            Area { x, y, w, h }
        }
        None => area,
    };
    Some(Draw {
        clip: Some(clip),
        ..draw
    })
}

/// One leaf of a flurry: when it sets off, its pace, where in its bob it starts, and which of
/// the two leaves it is. None of it reads the salt; the row it flies along does.
#[derive(Clone, Copy, Debug)]
struct Leaf {
    set_off: Millis,
    ms_per_px: u64,
    phase: u64,
    art: usize,
}

impl Leaf {
    fn of(event: &ActiveEvent, start: Millis, i: u8) -> Leaf {
        let hash = motion_hash(u64::from(i), u64::from(event.started_tick.hours()));
        let [slowest, fastest] = LEAF_MS_PER_PX;
        Leaf {
            set_off: start.plus(u32::from(i) * LEAF_EVERY_MS),
            ms_per_px: slowest + hash % (fastest - slowest),
            phase: (hash >> 32) % 8,
            art: usize::from(i % 2),
        }
    }

    /// When it has crossed a viewport `width` wide and gone out of sight.
    fn gone(self, width: i32, leaf_width: i32) -> Millis {
        let columns = u64::try_from(width + leaf_width).unwrap_or(0);
        Millis(self.set_off.0 + columns * self.ms_per_px)
    }
}

/// The wind the plants sway in: a step harder while the scene carries a storm, a gale while it
/// carries a windstorm.
pub(super) fn wind(input: &SceneInput) -> Wind {
    match (input.event.map(|event| event.kind), input.wind) {
        (Some(EventKind::Windstorm), _) | (Some(EventKind::Storm), Wind::Breezy | Wind::Gale) => {
            Wind::Gale
        }
        (Some(EventKind::Storm), Wind::Calm) => Wind::Breezy,
        (_, wind) => wind,
    }
}

impl Entry {
    /// Every look the subject has goes behind a wipe, frozen as it stands until the wipe has
    /// passed, then gone: the old sky ahead of the curtain holds still until it is covered.
    fn behind(&mut self, now: Millis, end: Millis) {
        for look in self.parts.iter_mut().flatten() {
            look.opacity = Tween::new(look.opacity.value(now), 0, Millis(end.0 - 1), 1);
            look.behind = true;
        }
    }

    /// The new sky's pieces, whole at once: the wipe uncovers them.
    fn ahead(&mut self, pieces: super::Pieces) {
        for (looks, piece) in self.parts.iter_mut().zip(pieces) {
            if let Some(piece) = piece {
                looks.push(Look {
                    piece,
                    opacity: Tween::at(SHOWN),
                    behind: false,
                });
            }
        }
    }
}

impl Composer {
    /// Begins the choreography of the instance the scene carries, unless it has played or
    /// another still plays; returns the wipe it begins, if it is one. It reads the sky as it
    /// stands, so it runs before the update moves any look.
    pub(super) fn cue(&mut self, input: &SceneInput, now: Millis) -> Option<Wipe> {
        let event = input.event?;
        let instance = (event.kind, event.started_tick);
        if !in_the_sky(event.kind) || self.played == Some(instance) {
            return None;
        }
        self.played = Some(instance);
        if self.show.is_some_and(|show| now < show.end) {
            return None;
        }
        // On the first update everything fades in: what the event brings comes after.
        let start = if self.clock.is_none() {
            now.plus(SKY_MS)
        } else {
            now
        };
        let width = self.viewport_width();
        let (play, end) = match event.kind {
            EventKind::Storm | EventKind::Clearing => {
                let storm = event.kind == EventKind::Storm;
                let before = self.weather_shown()?;
                let crosses = (before != Weather::Storm) == storm
                    && (input.weather == Weather::Storm) == storm;
                if !crosses {
                    self.show = None;
                    return None;
                }
                let wipe = Wipe::new(storm, event.side, now, width, self.sky.front_width);
                (Play::Wipe(wipe), wipe.end())
            }
            EventKind::Lightning => {
                let start = start.plus(1);
                let target = input.plants.iter().find(|plant| plant.scorched);
                let placed = target.and_then(|plant| self.placement.plants.get(&plant.id));
                let play = Play::Bolt {
                    start,
                    struck: placed.map(|placed| (placed.cell, placed.view)),
                };
                (play, Millis(start.0 + self.sky.lightning_ms))
            }
            EventKind::Dusk | EventKind::Sunburst => (Play::Light, now.plus(self.light_ms)),
            EventKind::RainAfterDrought => {
                let hold = by_strength(RAIN_HOLD_MS, event.strength);
                (Play::Rain { start }, start.plus(2 * RAIN_FADE_MS + hold))
            }
            _ => {
                let gone = (0..by_strength(LEAVES, event.strength))
                    .map(|i| Leaf::of(&event, start, i).gone(width, self.sky.leaf_width))
                    .max()
                    .unwrap_or(start);
                (Play::Leaves { start, width }, gone)
            }
        };
        self.show = Some(Show { event, play, end });
        match play {
            Play::Wipe(wipe) => Some(wipe),
            _ => None,
        }
    }

    /// The weather the sky is showing or becoming.
    fn weather_shown(&self) -> Option<Weather> {
        let looks = &self.entries.get(&Subject::Weather)?.parts[0];
        match looks
            .iter()
            .rev()
            .find(|look| look.opacity.target() == SHOWN)?
            .piece
        {
            Piece::Weather(weather) => Some(weather),
            _ => None,
        }
    }

    /// The sky's looks at a wipe's begin: those standing go behind it, the new ones ahead.
    pub(super) fn wipe_sky(
        &mut self,
        wanted: &std::collections::BTreeMap<Subject, super::Pieces>,
        now: Millis,
        end: Millis,
    ) {
        for (_, entry) in self.entries.iter_mut().filter(|(s, _)| super::sky(**s)) {
            entry.behind(now, end);
        }
        for (&subject, &pieces) in wanted.iter().filter(|(s, _)| super::sky(**s)) {
            self.entries.entry(subject).or_default().ahead(pieces);
        }
    }

    /// When a change of the sky an update brings at `now` may begin: once a wipe crossing has
    /// passed.
    pub(super) fn sky_from(&self, now: Millis) -> Millis {
        match self.show.map(|show| show.play) {
            Some(Play::Wipe(wipe)) if now < wipe.end() => wipe.end(),
            _ => now,
        }
    }

    /// The wipe crossing at `now`, if one is.
    pub(super) fn crossing(&self, now: Millis) -> Option<Wipe> {
        match self.show.map(|show| show.play) {
            Some(Play::Wipe(wipe)) if wipe.begin <= now && now < wipe.end() => Some(wipe),
            _ => None,
        }
    }

    /// Whether a crossing wipe at `at` cuts away all of a sky look's columns `from..to`, on
    /// the side it is on: behind the wipe or ahead of it.
    pub(super) fn cut_away(
        &self,
        art: &Art,
        at: Millis,
        behind: bool,
        (from, to): (i32, i32),
    ) -> bool {
        let Some(wipe) = self.crossing(at) else {
            return false;
        };
        let width = self.viewport_width();
        let curtain = Curtain::of(art, self.sky.front);
        let [new, old] = wipe.placing(at, width, curtain).columns();
        let (lo, hi) = if behind { old } else { new };
        from.max(0).max(lo) >= to.min(width).min(hi)
    }

    /// When the choreography playing is over.
    pub(super) fn show_end(&self) -> Option<Millis> {
        self.show.map(|show| show.end)
    }

    /// The choreography at `now`, as a depicted state: what plays, never where.
    pub(super) fn show_depicted(&self, now: Millis) -> Option<Depicted> {
        let show = self.show.filter(|show| show.playing(now))?;
        let subject = match show.play {
            Play::Wipe(_) => Subject::Front,
            Play::Bolt { .. } => Subject::Bolt,
            Play::Rain { .. } => Subject::Rain,
            Play::Leaves { .. } => Subject::Leaf(0),
            Play::Light => return None,
        };
        Some(Depicted {
            subject,
            part: 0,
            piece: Piece::Event(show.event.kind),
            opacity: SHOWN,
        })
    }

    /// The draws of the choreography at `now`, added to `keyed`, which already holds every
    /// other draw in viewport columns; `sky` names those of the sky's looks, and whether each
    /// is behind a wipe.
    pub(super) fn push_show(
        &self,
        art: &Art,
        table: &ArtTable,
        palette_mix: u8,
        now: Millis,
        keyed: &mut Vec<KeyedDraw>,
        sky: &[(usize, bool)],
    ) {
        let Some(show) = self.show.filter(|show| show.playing(now)) else {
            return;
        };
        let lit = Lit {
            value: SHOWN,
            palette_mix,
        };
        let width = self.viewport_width();
        let events = table.sky_events();
        match show.play {
            Play::Wipe(wipe) => {
                let curtain = Curtain::of(art, events.front);
                let placing = wipe.placing(now, width, curtain);
                divide(&art.catalog, keyed, sky, placing.columns(), width);
                keyed.extend(front(art, events.front, curtain, placing, width, lit));
            }
            Play::Bolt { start, struck } => {
                let strike = self.strike(art, table, &show.event, struck);
                keyed.extend(bolt(art, events.lightning, start, strike, lit, now));
            }
            Play::Rain { start } => {
                let lit = Lit {
                    value: rain_veil(start, show.end, now),
                    palette_mix,
                };
                keyed.extend(rain(art, events.rain, start, lit, now, width));
            }
            Play::Leaves { start, width } => {
                let flurry = (start, width);
                keyed.extend(self.leaves(art, events, &show.event, flurry, lit, now));
            }
            Play::Light => {}
        }
    }

    /// The flurry's leaves in flight at `now`, from `start` across a viewport `width` wide.
    fn leaves(
        &self,
        art: &Art,
        events: &SkyEventArt,
        event: &ActiveEvent,
        (start, width): (Millis, i32),
        lit: Lit,
        now: Millis,
    ) -> Vec<KeyedDraw> {
        let catalog = &art.catalog;
        let mut out = Vec::new();
        for i in 0..by_strength(LEAVES, event.strength) {
            let leaf = Leaf::of(event, start, i);
            if now < leaf.set_off || now >= leaf.gone(width, events.leaf_width) {
                continue;
            }
            let elapsed = now.0 - leaf.set_off.0;
            let anim = catalog.animation(events.leaves[leaf.art]);
            let frame = anim::pose(catalog, anim, elapsed).frame;
            let travelled = i32::try_from(elapsed / leaf.ms_per_px).unwrap_or(i32::MAX);
            let across = travelled - events.leaf_width;
            let index = (u64::from(event.started_tick.hours()) << 8) | u64::from(i);
            let row = self.salt.below(Purpose::EventPlace, index, LEAF_ROWS);
            let bob = (elapsed / WOBBLE_STEP_MS + leaf.phase) % 8;
            let bob = WOBBLE[usize::try_from(bob).unwrap_or(0)];
            let y = LEAF_TOP + i32::try_from(row).unwrap_or(0) + bob;
            let (x, flip_x) = match event.side {
                Side::Left => (across, false),
                Side::Right => (width - events.leaf_width - across, true),
            };
            // Out of sight on either side, it is not drawn.
            if x + events.leaf_width <= 0 || x >= width {
                continue;
            }
            let draw = Draw {
                flip_x,
                ..Draw::new(catalog, frame, x, y)
            };
            out.extend(
                lit.apply(art, draw)
                    .map(|draw| overlay(Subject::Leaf(i), 0, draw)),
            );
        }
        out
    }

    /// Where lightning strikes, in viewport columns, and the row its bolt reaches down to: the
    /// struck plant's crown, or, where no struck plant is drawn, the ground at a salted place.
    fn strike(
        &self,
        art: &Art,
        table: &ArtTable,
        event: &ActiveEvent,
        struck: Option<(u8, PlantView)>,
    ) -> (i32, i32) {
        let inset = self.viewport.inset();
        if let Some((cell, view)) = struck {
            let x = self.layout.slot(cell).x;
            let PlantView {
                species,
                stage,
                state,
                ..
            } = view;
            let foliage = match table.plant(species, stage, state) {
                PlantArt::Parts { foliage, .. } => foliage,
                PlantArt::Seedling(combined) => combined,
            };
            let piece = Piece::Foliage {
                species,
                stage,
                state,
            };
            let top = Draw::anchored(&art.catalog, foliage, x, standing(piece)).y;
            return (x + inset, top);
        }
        let index = (u64::from(event.started_tick.hours()) << 8) | STRIKE;
        let width = u64::try_from(super::FRAME_WIDTH - 2 * STRIKE_MARGIN).unwrap_or(0);
        let x = STRIKE_MARGIN
            + i32::try_from(self.salt.below(Purpose::EventPlace, index, width)).unwrap_or(0);
        (x + inset, PLANT_GROUND_Y)
    }

    /// The first moment after `now` at which the choreography's draws change.
    pub(super) fn show_next(&self, art: &Art, table: &ArtTable, now: Millis) -> Option<Millis> {
        let show = self.show.filter(|show| now < show.end)?;
        let catalog = &art.catalog;
        let events = table.sky_events();
        let mut next = Earliest::default();
        let frames = |anim, start: Millis| {
            anim::next_frame_at(catalog.animation(anim), now.0.saturating_sub(start.0))
                .map(|ms| Millis(start.0 + ms))
                .filter(|&moment| moment > now)
        };
        match show.play {
            Play::Wipe(wipe) => {
                let curtain = Curtain::of(art, events.front);
                next.take(wipe.next_step(now, self.viewport_width(), curtain));
            }
            Play::Bolt { start, .. } => {
                next.take(if now < start {
                    Some(start)
                } else {
                    frames(events.lightning, start)
                });
            }
            Play::Rain { start } => {
                let [rise, fall] = rain_tweens(start, show.end);
                let own = catalog
                    .sprite(catalog.animation(events.rain).frames[0])
                    .opacity;
                let seen = |value| super::scale(own, value);
                next.take(rise.next_step_by(now, seen));
                next.take(fall.next_step_by(now, seen));
                // A frame of streaks the veil hides changes nothing.
                let showing = |moment: &Millis| seen(rain_veil(start, show.end, *moment)) > 0;
                next.take(frames(events.rain, start).filter(showing));
            }
            Play::Leaves { start, width } => {
                for i in 0..by_strength(LEAVES, show.event.strength) {
                    let leaf = Leaf::of(&show.event, start, i);
                    let gone = leaf.gone(width, events.leaf_width);
                    if now >= gone {
                        continue;
                    }
                    // It shows from its first step in.
                    let first = leaf.set_off.0 + leaf.ms_per_px;
                    if now.0 < first {
                        next.take(Some(Millis(first)));
                        continue;
                    }
                    let elapsed = now.0 - leaf.set_off.0;
                    let step = |every: u64| Millis(leaf.set_off.0 + (elapsed / every + 1) * every);
                    next.take(Some(step(leaf.ms_per_px)));
                    next.take(Some(step(WOBBLE_STEP_MS)));
                    next.take(frames(events.leaves[leaf.art], leaf.set_off));
                }
            }
            Play::Light => {}
        }
        next.0
    }
}

/// An event's piece, keyed under the light, so dusk and sunburst lie over it.
fn overlay(subject: Subject, item: u16, draw: Draw) -> KeyedDraw {
    KeyedDraw::at(Place::new(subject, 0, 0).item(item), (0, -1), draw)
}

/// Cuts each of the sky's draws in `keyed` to its side of a wipe, the old sky's to `old` and
/// the new one's to `new`, and drops those with nothing left to show.
fn divide(
    catalog: &crate::catalog::Catalog,
    keyed: &mut Vec<KeyedDraw>,
    sky: &[(usize, bool)],
    [new, old]: [(i32, i32); 2],
    width: i32,
) {
    let mut gone = Vec::new();
    for &(i, behind) in sky {
        let draw = keyed[i].draw;
        let w = i32::from(catalog.sprite(draw.sprite).rect.w);
        match cut(draw, w, if behind { old } else { new }, width) {
            Some(draw) => keyed[i].draw = draw,
            None => gone.push(i),
        }
    }
    let mut index = 0;
    keyed.retain(|_| {
        index += 1;
        gone.binary_search(&(index - 1)).is_err()
    });
}

/// The storm front's curtain where a wipe has it, while any of it is in sight.
fn front(
    art: &Art,
    sprite: SpriteId,
    curtain: Curtain,
    placing: Placing,
    width: i32,
    lit: Lit,
) -> Option<KeyedDraw> {
    let (first, last) = if placing.flip_x {
        (curtain.open.1, curtain.width - curtain.open.0)
    } else {
        (curtain.open.0, curtain.width - curtain.open.1)
    };
    if placing.x + last <= 0 || placing.x + first >= width {
        return None;
    }
    let draw = Draw {
        flip_x: placing.flip_x,
        ..Draw::new(&art.catalog, sprite, placing.x, 0)
    };
    // Past the sky's far corner, so it paints over every cloud and star.
    let place = Place::new(Subject::Front, 0, 0);
    Some(KeyedDraw::at(
        place,
        (placing.x, FRAME_HEIGHT),
        lit.apply(art, draw)?,
    ))
}

/// Lightning's frame at `now`, from `start`, its bolt centred on `x` and reaching down to
/// `bottom`; nothing once its frames are over.
fn bolt(
    art: &Art,
    lightning: AnimationId,
    start: Millis,
    (x, bottom): (i32, i32),
    lit: Lit,
    now: Millis,
) -> Option<KeyedDraw> {
    let catalog = &art.catalog;
    let pose = anim::pose(catalog, catalog.animation(lightning), now.0 - start.0);
    if pose.done {
        return None;
    }
    let rect = catalog.sprite(pose.frame).rect;
    let left = x - i32::from(rect.w) / 2;
    let draw = Draw::new(catalog, pose.frame, left, bottom - i32::from(rect.h));
    Some(overlay(Subject::Bolt, 0, lit.apply(art, draw)?))
}

/// Rain's streaks at `now`, falling since `start`, across a viewport `width` wide.
fn rain(
    art: &Art,
    streaks: AnimationId,
    start: Millis,
    lit: Lit,
    now: Millis,
    width: i32,
) -> Vec<KeyedDraw> {
    let catalog = &art.catalog;
    let frame = anim::pose(catalog, catalog.animation(streaks), now.0 - start.0).frame;
    (0_u16..)
        .zip(world::tiled(catalog, frame, width))
        .filter_map(|(item, draw)| Some(overlay(Subject::Rain, item, lit.apply(art, draw)?)))
        .collect()
}

/// Lightning without a plant to strike keeps this far inside the scene's edges.
const STRIKE_MARGIN: i32 = 24;

/// Rain's veil rising from `start`, and falling to be gone at `end`.
fn rain_tweens(start: Millis, end: Millis) -> [Tween; 2] {
    let fall = Millis(end.0 - u64::from(RAIN_FADE_MS));
    [
        Tween::new(0, SHOWN, start, RAIN_FADE_MS),
        Tween::new(SHOWN, 0, fall, RAIN_FADE_MS),
    ]
}

/// How much of the rain shows at `now`.
fn rain_veil(start: Millis, end: Millis, now: Millis) -> u8 {
    let [rise, fall] = rain_tweens(start, end);
    rise.value(now).min(fall.value(now))
}

#[cfg(test)]
mod tests {
    use terrarium_truth::scene::{GrowthStage, Light, PlantState};

    use super::super::OverlayInput;
    use super::*;
    use crate::catalog::{Blend, Layer};
    use crate::clock::LocalClock;
    use crate::frame::{NATIVE_WIDTH, Viewport};
    use crate::preset;
    use crate::salt::InstallSalt;

    fn art() -> &'static Art<'static> {
        Art::embedded()
    }

    fn table() -> &'static ArtTable {
        ArtTable::embedded()
    }

    fn overlay(night: bool) -> OverlayInput {
        let clock = if night {
            LocalClock::midnight(preset::DATE)
        } else {
            LocalClock::noon(preset::DATE)
        };
        OverlayInput::new(Vec::new(), clock)
    }

    /// The thriving terrarium under `weather`: clouds, the sun by day, the moon and stars by
    /// night, when the sky is clear.
    fn scene(weather: Weather) -> SceneInput {
        SceneInput {
            weather,
            ..preset::scene("thriving").unwrap()
        }
    }

    fn event(kind: EventKind, tick: u32, side: Side) -> ActiveEvent {
        ActiveEvent {
            kind,
            started_tick: Tick::new(tick),
            strength: Strength::Mid,
            side,
            fossil: None,
        }
    }

    fn carrying(scene: SceneInput, event: ActiveEvent) -> SceneInput {
        SceneInput {
            event: Some(event),
            ..scene
        }
    }

    /// A composer settled on `before`, given `after` once settled; and when.
    fn played(before: &SceneInput, after: &SceneInput, salt: InstallSalt) -> (Composer, Millis) {
        let mut composer = Composer::new(table(), salt);
        composer.update(before, &overlay(false), Millis(0));
        let now = composer.settled_at();
        composer.update(after, &overlay(false), now);
        (composer, now)
    }

    fn draws(composer: &Composer, now: Millis) -> Vec<KeyedDraw> {
        composer.draws(art(), table(), 0, now)
    }

    fn of(composer: &Composer, now: Millis, wanted: fn(Subject) -> bool) -> Vec<KeyedDraw> {
        draws(composer, now)
            .into_iter()
            .filter(|keyed| wanted(keyed.key.subject))
            .collect()
    }

    fn front(composer: &Composer, now: Millis) -> Option<Draw> {
        of(composer, now, |s| s == Subject::Front)
            .first()
            .map(|keyed| keyed.draw)
    }

    fn storm_body() -> SpriteId {
        table().weather(Weather::Storm)[0]
    }

    fn wiped(side: Side) -> (Composer, Millis) {
        let storm = carrying(scene(Weather::Storm), event(EventKind::Storm, 7, side));
        played(&scene(Weather::Clear), &storm, preset::SALT)
    }

    /// The columns a draw shows in a viewport `width` wide, after its clip.
    fn columns(draw: &Draw, width: i32) -> (i32, i32) {
        let w = i32::from(art().catalog.sprite(draw.sprite).rect.w);
        let (mut from, mut to) = (draw.x.max(0), (draw.x + w).min(width));
        if let Some(clip) = draw.clip {
            (from, to) = (from.max(clip.x), to.min(clip.x + clip.w));
        }
        (from, to)
    }

    const WIDTH: i32 = NATIVE_WIDTH.cast_signed();

    #[test]
    fn the_seam_lies_under_the_front_s_opaque_columns() {
        let front = table().sky_events().front;
        let rows: Vec<&[u8]> = art().rows(front).collect();
        assert_eq!(rows.len(), usize::from(NATIVE_HEIGHT_ROWS));
        for (y, row) in rows.iter().enumerate() {
            let seam = usize::try_from(SEAM).unwrap();
            assert!(row[..=seam].iter().all(|&index| index != 0), "row {y}");
        }
        // Its open columns lie ahead of it, on its right, as it enters from the left.
        let (left, right) = world::blank_columns(art(), front);
        assert_eq!(left, 0);
        assert!(right > 0 && right < SEAM);
    }

    const NATIVE_HEIGHT_ROWS: u8 = 74;

    #[test]
    fn the_frame_at_a_wipe_s_update_is_the_frame_before() {
        for side in [Side::Left, Side::Right] {
            for kind in [EventKind::Storm, EventKind::Clearing] {
                let (from, to) = if kind == EventKind::Storm {
                    (Weather::Clear, Weather::Storm)
                } else {
                    (Weather::Storm, Weather::Clear)
                };
                let mut composer = Composer::new(table(), preset::SALT);
                composer.update(&scene(from), &overlay(false), Millis(0));
                let now = composer.settled_at();
                let mut updated = composer.clone();
                updated.update(
                    &carrying(scene(to), event(kind, 3, side)),
                    &overlay(false),
                    now,
                );
                assert!(
                    updated.crossing(now).is_some(),
                    "{kind:?} from the {side:?}"
                );
                for mix in [0, 255] {
                    assert_eq!(
                        composer.draws(art(), table(), mix, now),
                        updated.draws(art(), table(), mix, now),
                        "{kind:?} from the {side:?} at {mix}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_storm_wipes_in_from_its_side_and_mirrors_for_the_right() {
        let (left, now) = wiped(Side::Left);
        let (right, _) = wiped(Side::Right);
        let end = left.crossing(now).unwrap().end();
        // A pixel a frame across the scene and the curtain.
        assert_eq!(end.0 - now.0, (640 + 24) * MIN_FRAME_MS);
        let mut last = None;
        for at in [now.plus(200), now.plus(20_000), Millis(end.0 - 200)] {
            let (l, r) = (front(&left, at).unwrap(), front(&right, at).unwrap());
            assert!(!l.flip_x && r.flip_x, "the storm's columns behind it");
            assert_eq!(r.x, WIDTH - l.x - 24, "mirrored");
            assert!(last.is_none_or(|x| l.x > x), "moving right");
            last = Some(l.x);
            // The storm's backdrop behind the curtain, the old sky ahead of it.
            let body = |composer: &Composer| {
                of(composer, at, |s| s == Subject::Weather)
                    .into_iter()
                    .find(|keyed| keyed.draw.sprite == storm_body())
                    .map(|keyed| columns(&keyed.draw, WIDTH))
            };
            let line = l.x + SEAM;
            let shows = (line > 0).then_some(());
            assert_eq!(body(&left), shows.map(|()| (0, line.min(WIDTH))));
            assert_eq!(body(&right), shows.map(|()| ((WIDTH - line).max(0), WIDTH)));
        }
        // Its last column leaves at the end, with the old sky.
        assert_eq!(
            front(&left, Millis(end.0 - 1)).map(|d| d.x),
            Some(WIDTH - 1)
        );
        assert_eq!(front(&left, end), None);
    }

    #[test]
    fn a_clearing_carries_the_storm_out_the_other_side() {
        let clearing = carrying(
            scene(Weather::Clear),
            event(EventKind::Clearing, 9, Side::Left),
        );
        let (composer, now) = played(&scene(Weather::Storm), &clearing, preset::SALT);
        let at = now.plus(15_000);
        let curtain = front(&composer, at).unwrap();
        assert!(curtain.flip_x, "its storm columns ahead, toward the storm");
        let line = curtain.x + 24 - SEAM;
        let body = of(&composer, at, |s| s == Subject::Weather)
            .into_iter()
            .find(|keyed| keyed.draw.sprite == storm_body())
            .unwrap();
        // The storm lies ahead, to the right, and the clear sky follows from the left.
        assert_eq!(columns(&body.draw, WIDTH), (line, WIDTH));
        let clear = table().weather(Weather::Clear)[0];
        let sky = of(&composer, at, |s| s == Subject::Weather);
        let clear = sky.iter().find(|keyed| keyed.draw.sprite == clear).unwrap();
        assert_eq!(columns(&clear.draw, WIDTH), (0, line));
        let later = front(&composer, at.plus(5000)).unwrap();
        assert!(later.x > curtain.x, "on across");
    }

    #[test]
    fn a_wipe_begins_only_where_the_weather_crosses_its_way() {
        let cases = [
            // Already a storm, or none: no wipe.
            (Weather::Storm, Weather::Storm, EventKind::Storm),
            (Weather::Clear, Weather::Clear, EventKind::Storm),
            (Weather::Clear, Weather::Clear, EventKind::Clearing),
            (Weather::Storm, Weather::Storm, EventKind::Clearing),
            // A clearing never brings a storm, nor a storm takes one away.
            (Weather::Clear, Weather::Storm, EventKind::Clearing),
            (Weather::Storm, Weather::Haze, EventKind::Storm),
        ];
        for (from, to, kind) in cases {
            let after = carrying(scene(to), event(kind, 4, Side::Left));
            let (composer, now) = played(&scene(from), &after, preset::SALT);
            assert_eq!(composer.crossing(now.plus(100)), None, "{from:?} to {to:?}");
            assert_eq!(front(&composer, now.plus(5000)), None);
        }
        let after = carrying(
            scene(Weather::Storm),
            event(EventKind::Storm, 4, Side::Left),
        );
        let (composer, now) = played(&scene(Weather::Haze), &after, preset::SALT);
        assert!(composer.crossing(now.plus(100)).is_some(), "haze to storm");
    }

    #[test]
    fn the_front_names_each_of_its_steps() {
        for width in [640, 840] {
            let mut composer = Composer::new(table(), preset::SALT);
            composer.set_viewport(Viewport::new(width));
            composer.update(&scene(Weather::Clear), &overlay(false), Millis(0));
            let now = composer.settled_at();
            let storm = carrying(
                scene(Weather::Storm),
                event(EventKind::Storm, 1, Side::Right),
            );
            composer.update(&storm, &overlay(false), now);
            let wipe = composer.crossing(now).unwrap();
            let curtain = Curtain::of(art(), table().sky_events().front);
            let mut at = now;
            let mut steps = 0;
            let mut x = wipe.placing(at, i32::try_from(width).unwrap(), curtain).x;
            while let Some(next) = wipe.next_step(at, i32::try_from(width).unwrap(), curtain) {
                assert!(
                    next.0 - at.0 >= MIN_FRAME_MS || at == now,
                    "a pixel a frame at most"
                );
                let moved = wipe.placing(next, i32::try_from(width).unwrap(), curtain).x;
                assert_eq!((x - moved).abs(), 1, "one column at {next:?}");
                let before =
                    wipe.placing(Millis(next.0 - 1), i32::try_from(width).unwrap(), curtain);
                assert_eq!(before.x, x, "not before it is named");
                (at, x, steps) = (next, moved, steps + 1);
            }
            assert_eq!(at, wipe.end());
            assert_eq!(steps, i32::try_from(width).unwrap() + 20);
        }
    }

    #[test]
    fn the_front_paints_over_clouds_sun_moon_and_stars() {
        for (night, width) in [(false, 640), (true, 640), (false, 840), (true, 735)] {
            let mut composer = Composer::new(table(), preset::SALT);
            composer.set_viewport(Viewport::new(width));
            composer.update(&scene(Weather::Clear), &overlay(night), Millis(0));
            let now = composer.settled_at();
            let storm = carrying(
                scene(Weather::Storm),
                event(EventKind::Storm, 2, Side::Left),
            );
            composer.update(&storm, &overlay(night), now);
            let at = now.plus(30_000);
            let keyed = composer.draws(art(), table(), if night { 255 } else { 0 }, at);
            let front = keyed
                .iter()
                .position(|k| k.key.subject == Subject::Front)
                .unwrap();
            let sky = |s: Subject| {
                matches!(
                    s,
                    Subject::Weather
                        | Subject::Cloud(_)
                        | Subject::Sun
                        | Subject::Moon
                        | Subject::Star(_)
                )
            };
            assert!(keyed.iter().filter(|k| sky(k.key.subject)).count() > 2);
            assert!(keyed[front + 1..].iter().all(|k| k.key.layer != Layer::Sky));
            assert!(
                keyed[..front]
                    .iter()
                    .filter(|k| k.key.layer == Layer::Sky)
                    .all(|k| sky(k.key.subject))
            );
            // Every cut lies on the line under the curtain, in the viewport's columns, the
            // scene's sun, moon and stars as much as the backdrop.
            let line = keyed[front].draw.x + SEAM;
            let cut: Vec<_> = keyed.iter().filter_map(|k| k.draw.clip).collect();
            assert!(!cut.is_empty());
            assert!(
                cut.iter().all(|c| c.x == line || c.x + c.w == line),
                "{cut:?} at {line}"
            );
        }
    }

    /// The scene's own sky, its stars, stands in the viewport's middle: the curtain cuts it
    /// where it shows, not where it stands in the scene.
    #[test]
    fn a_wipe_cuts_the_scene_s_stars_where_they_show() {
        let mut composer = Composer::new(table(), preset::SALT);
        composer.set_viewport(Viewport::new(840));
        composer.update(&scene(Weather::Clear), &overlay(true), Millis(0));
        let now = composer.settled_at();
        let stars = |composer: &Composer, at: Millis| -> Vec<i32> {
            composer
                .draws(art(), table(), 255, at)
                .iter()
                .filter(|k| matches!(k.key.subject, Subject::Star(_)))
                .map(|k| k.draw.x)
                .collect()
        };
        let last = *stars(&composer, now).iter().max().unwrap();
        let storm = carrying(
            scene(Weather::Storm),
            event(EventKind::Storm, 2, Side::Left),
        );
        composer.update(&storm, &overlay(true), now);
        // The moment the line under the curtain comes within ten columns of the last star.
        let wipe = composer.crossing(now).unwrap();
        let curtain = Curtain::of(art(), table().sky_events().front);
        let mut at = now;
        while wipe.placing(at, 840, curtain).line < last - 10 {
            at = wipe.next_step(at, 840, curtain).unwrap();
        }
        let shown = stars(&composer, at);
        assert!(shown.contains(&last), "the star ahead of the curtain shows");
        assert!(shown.iter().all(|&x| x >= last - 10));
    }

    #[test]
    fn nothing_stands_behind_the_front_once_it_has_passed() {
        let (composer, now) = wiped(Side::Left);
        let end = composer.crossing(now).unwrap().end();
        for at in [end, end.plus(1), end.plus(60_000)] {
            let sky = of(&composer, at, |s| {
                matches!(s, Subject::Weather | Subject::Cloud(_) | Subject::Sun)
            });
            assert_eq!(sky.len(), 1, "{at:?}");
            assert_eq!(sky[0].draw.sprite, storm_body());
            assert_eq!(sky[0].draw.clip, None);
            assert_eq!(sky[0].draw.opacity, SHOWN);
        }
        assert_eq!(composer.settled_at(), end);
    }

    #[test]
    fn a_wipe_begun_mid_crossfade_keeps_both_old_looks() {
        // A storm passing to haze, and back to a storm while the haze still rises.
        let mut composer = Composer::new(table(), preset::SALT);
        composer.update(&scene(Weather::Storm), &overlay(false), Millis(0));
        let t = composer.settled_at();
        composer.update(&scene(Weather::Haze), &overlay(false), t);
        let begin = t.plus(1000);
        let storm = carrying(
            scene(Weather::Storm),
            event(EventKind::Storm, 5, Side::Left),
        );
        let before = of(&composer, begin, |s| s == Subject::Weather);
        composer.update(&storm, &overlay(false), begin);
        assert_eq!(before.len(), 2, "the old storm under the haze rising");
        let at = begin.plus(10_000);
        let after = of(&composer, at, |s| s == Subject::Weather);
        for old in &before {
            let held = after
                .iter()
                .filter(|k| k.draw.sprite == old.draw.sprite)
                .any(|k| {
                    k.draw.opacity == old.draw.opacity && k.draw.clip.is_some_and(|c| c.x > 0)
                });
            assert!(held, "{:?} held ahead of the curtain", old.draw.sprite);
        }
    }

    #[test]
    fn a_crossing_back_mid_wipe_waits_for_it_and_never_cuts() {
        let (mut composer, now) = wiped(Side::Left);
        let end = composer.crossing(now).unwrap().end();
        // The tests pass again within seconds: the sky clears once the storm is in.
        composer.update(&scene(Weather::Clear), &overlay(false), now.plus(5000));
        let clear = table().weather(Weather::Clear)[0];
        let shown = |at: Millis| -> Vec<(SpriteId, u8, (i32, i32))> {
            of(&composer, at, |s| s == Subject::Weather)
                .iter()
                .map(|k| (k.draw.sprite, k.draw.opacity, columns(&k.draw, WIDTH)))
                .collect()
        };
        // Still crossing: the new clear sky stays behind the old one, ahead of the curtain.
        let mid = shown(now.plus(30_000));
        let curtain = front(&composer, now.plus(30_000)).unwrap();
        for (sprite, _, (from, _)) in &mid {
            if *sprite == clear {
                assert!(
                    *from >= curtain.x,
                    "the clear sky only ahead of the curtain"
                );
            }
        }
        // Across: the storm stands whole, and the clear sky rises from nothing over it.
        assert_eq!(shown(end), vec![(storm_body(), SHOWN, (0, WIDTH))]);
        let rising = shown(end.plus(1500));
        let risen = rising.iter().find(|(sprite, ..)| *sprite == clear).unwrap();
        assert!(risen.1 > 0 && risen.1 < SHOWN, "{rising:?}");
    }

    #[test]
    fn a_viewport_change_mid_wipe_never_brings_back_a_look() {
        let (mut composer, now) = wiped(Side::Left);
        let end = composer.crossing(now).unwrap().end();
        composer.set_viewport(Viewport::new(840));
        for at in [now.plus(40_000), Millis(end.0 - 1), end] {
            let behind = of(&composer, at, |s| s == Subject::Weather)
                .into_iter()
                .filter(|k| k.draw.sprite != storm_body())
                .map(|k| columns(&k.draw, 840))
                .collect::<Vec<_>>();
            let curtain = front(&composer, at);
            for (from, to) in behind {
                assert!(from < to);
                assert!(
                    curtain.is_some_and(|c| from >= c.x),
                    "{at:?}: ({from}, {to})"
                );
            }
        }
        assert_eq!(front(&composer, end), None);
    }

    /// A scene with one mature, thriving plant struck by lightning, from the left.
    fn struck(salt: InstallSalt, target: bool) -> (Composer, Millis, Option<u32>) {
        let mut storm = scene(Weather::Storm);
        let id = storm
            .plants
            .iter_mut()
            .find(|p| p.stage == GrowthStage::Mature && p.state == PlantState::Thriving)
            .map(|plant| {
                plant.scorched = target;
                plant.id
            });
        let lightning = carrying(storm.clone(), event(EventKind::Lightning, 30, Side::Left));
        let (composer, now) = played(&storm, &lightning, salt);
        (composer, now, id.filter(|_| target))
    }

    #[test]
    fn lightning_plays_its_two_frames_once_above_the_struck_plant() {
        let (composer, now, id) = struck(preset::SALT, true);
        let bolt = |at: Millis| of(&composer, at, |s| s == Subject::Bolt);
        let frames = &art()
            .catalog
            .animation(table().sky_events().lightning)
            .frames;
        assert!(bolt(now).is_empty(), "not at the update");
        let first = bolt(now.plus(1));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].draw.sprite, frames[0]);
        assert_eq!(bolt(now.plus(1 + 166))[0].draw.sprite, frames[1]);
        assert!(bolt(now.plus(1 + 332)).is_empty(), "played once");
        assert!(bolt(now.plus(60_000)).is_empty());
        // Above the struck plant: centred on its slot, down to its crown.
        let placed = composer.placement.plants[&id.unwrap()];
        let x = composer.layout.slot(placed.cell).x;
        let draw = first[0].draw;
        let rect = art().catalog.sprite(draw.sprite).rect;
        assert_eq!(draw.x + i32::from(rect.w) / 2, x);
        let plant = of(&composer, now.plus(1), |s| {
            matches!(s, Subject::Plant { .. })
        })
        .into_iter()
        .filter(|k| {
            k.key.subject
                == Subject::Plant {
                    id: id.unwrap(),
                    cell: placed.cell,
                }
                && k.key.part == 1
        })
        .map(|k| k.draw.y)
        .min()
        .unwrap();
        assert_eq!(draw.y + i32::from(rect.h), plant);
        assert_eq!(
            composer.next_change(art(), table(), now).unwrap(),
            now.plus(1)
        );
        // An update mid-strike that takes the plant away moves nothing of the bolt.
        let mut later = composer.clone();
        let mut gone = scene(Weather::Storm);
        gone.plants.retain(|p| Some(p.id) != id);
        gone.event = Some(event(EventKind::Lightning, 30, Side::Left));
        later.update(&gone, &overlay(false), now.plus(100));
        let held = of(&later, now.plus(170), |s| s == Subject::Bolt)[0].draw;
        assert_eq!((held.x, held.y), (draw.x, draw.y));
    }

    #[test]
    fn lightning_without_a_drawn_target_strikes_a_salted_place() {
        let at = |salt| {
            let (composer, now, _) = struck(salt, false);
            let bolt = of(&composer, now.plus(1), |s| s == Subject::Bolt);
            let draw = bolt[0].draw;
            let rect = art().catalog.sprite(draw.sprite).rect;
            assert_eq!(draw.y + i32::from(rect.h), PLANT_GROUND_Y);
            draw.x
        };
        let places: Vec<i32> = (0..4_u8).map(|n| at(InstallSalt::new([n; 16]))).collect();
        assert!(places.iter().any(|&x| x != places[0]), "{places:?}");
        assert!(
            places
                .iter()
                .all(|&x| (STRIKE_MARGIN - 3..WIDTH - STRIKE_MARGIN).contains(&x))
        );
    }

    #[test]
    fn rain_tiles_the_viewport_by_its_art_and_fades_in_holds_and_out() {
        for width in [640_u32, 840] {
            let mut composer = Composer::new(table(), preset::SALT);
            composer.set_viewport(Viewport::new(width));
            composer.update(&scene(Weather::Clear), &overlay(false), Millis(0));
            let now = composer.settled_at();
            let rain = carrying(
                scene(Weather::Clear),
                event(EventKind::RainAfterDrought, 9, Side::Right),
            );
            composer.update(&rain, &overlay(false), now);
            let streaks = |at: Millis| of(&composer, at, |s| s == Subject::Rain);
            let frames = &art().catalog.animation(table().sky_events().rain).frames;
            let w = i32::from(art().catalog.sprite(frames[0]).rect.w);
            // Held, the streaks show at the art's own opacity.
            let own = art().catalog.sprite(frames[0]).opacity;
            assert!(streaks(now).is_empty());
            let held = streaks(now.plus(RAIN_FADE_MS + 1000));
            assert_eq!(
                held.len(),
                usize::try_from(width.div_ceil(u32::try_from(w).unwrap())).unwrap()
            );
            assert!(
                held.iter()
                    .all(|k| k.draw.opacity == own && k.draw.blend == Blend::Normal)
            );
            let xs: Vec<i32> = held.iter().map(|k| k.draw.x).collect();
            assert_eq!(
                xs,
                (0..)
                    .step_by(usize::try_from(w).unwrap())
                    .take(xs.len())
                    .collect::<Vec<_>>()
            );
            let rising = streaks(now.plus(RAIN_FADE_MS / 2))[0].draw.opacity;
            assert!(rising > 0 && rising < own);
            // Every frame of the loop shows, at the art's own pace.
            let seen: std::collections::BTreeSet<SpriteId> = (0..6)
                .map(|i| streaks(now.plus(5000 + i * 166))[0].draw.sprite)
                .collect();
            assert_eq!(seen.len(), frames.len());
            let end = now.plus(2 * RAIN_FADE_MS + RAIN_HOLD_MS[1]);
            let falling = streaks(Millis(end.0 - u64::from(RAIN_FADE_MS) / 2))[0]
                .draw
                .opacity;
            assert!(falling > 0 && falling < own);
            assert!(streaks(end).is_empty());
            assert_eq!(composer.settled_at(), end);
        }
    }

    /// Where a leaf is at one moment: its column, row, facing and frame.
    type Step = (i32, i32, bool, SpriteId);

    /// Each leaf's columns and rows over a windstorm, by leaf.
    fn flurry(side: Side, salt: InstallSalt) -> (Vec<Vec<Step>>, Millis, Millis) {
        let blow = carrying(scene(Weather::Clear), event(EventKind::Windstorm, 12, side));
        let (composer, now) = played(&scene(Weather::Clear), &blow, salt);
        let end = composer.settled_at();
        let mut leaves = vec![Vec::new(); 12];
        // Finer than any leaf's step, so each is seen at every column it takes.
        for at in (now.0..end.0 + 100).step_by(10) {
            for keyed in of(&composer, Millis(at), |s| matches!(s, Subject::Leaf(_))) {
                let Subject::Leaf(i) = keyed.key.subject else {
                    unreachable!()
                };
                leaves[usize::from(i)].push((
                    keyed.draw.x,
                    keyed.draw.y,
                    keyed.draw.flip_x,
                    keyed.draw.sprite,
                ));
            }
        }
        (leaves, now, end)
    }

    #[test]
    fn windstorm_leaves_cross_from_the_event_s_side_out_of_sight() {
        for side in [Side::Left, Side::Right] {
            let (leaves, ..) = flurry(side, preset::SALT);
            let flown: Vec<_> = leaves.iter().filter(|path| !path.is_empty()).collect();
            assert_eq!(flown.len(), usize::from(LEAVES[1]), "a middling flurry");
            for path in flown {
                let (first, last) = (path[0], path[path.len() - 1]);
                match side {
                    Side::Left => {
                        assert!(first.0 < 0 && last.0 > WIDTH - 4);
                        assert!(path.windows(2).all(|w| w[1].0 >= w[0].0));
                    }
                    Side::Right => {
                        assert!(first.0 > WIDTH - 4 && last.0 < 0);
                        assert!(path.windows(2).all(|w| w[1].0 <= w[0].0));
                    }
                }
                assert!(path.iter().all(|step| step.2 == (side == Side::Right)));
                assert!(
                    path.iter()
                        .all(|step| (LEAF_TOP - 2..LEAF_TOP + 42).contains(&step.1))
                );
                assert!(
                    path.windows(2).all(|w| (w[1].1 - w[0].1).abs() <= 1),
                    "a pixel a bob"
                );
            }
        }
    }

    #[test]
    fn leaf_timing_reads_no_salt() {
        let when = |leaves: &Vec<Vec<Step>>| -> Vec<Vec<(i32, SpriteId)>> {
            leaves
                .iter()
                .map(|path| path.iter().map(|s| (s.0, s.3)).collect())
                .collect()
        };
        let (first, ..) = flurry(Side::Left, preset::SALT);
        for n in [1, 9, 77, 200] {
            let (other, ..) = flurry(Side::Left, InstallSalt::new([n; 16]));
            assert_eq!(
                when(&first),
                when(&other),
                "the same paths across, at the same times"
            );
            assert_ne!(first, other, "along other rows");
        }
    }

    #[test]
    fn dusk_multiplies_and_sunburst_adds_over_their_crossfade() {
        for (kind, light, blend, opacity) in [
            (EventKind::Dusk, Light::Dusk, Blend::Multiply, 34),
            (EventKind::Sunburst, Light::Sunburst, Blend::Add, 26),
        ] {
            let after = carrying(
                SceneInput {
                    light,
                    ..scene(Weather::Clear)
                },
                event(kind, 2, Side::Left),
            );
            let (composer, now) = played(&scene(Weather::Clear), &after, preset::SALT);
            let lit = |at: Millis| of(&composer, at, |s| s == Subject::Light);
            assert!(lit(now).is_empty());
            let half = lit(now.plus(600))[0].draw;
            assert_eq!(half.blend, blend);
            assert!(half.opacity > 0 && half.opacity < opacity);
            assert_eq!(lit(now.plus(1200))[0].draw.opacity, opacity);
            // It holds the stage for its crossfade: a storm then is passed over.
            let mut later = composer.clone();
            let storm = carrying(
                SceneInput {
                    light,
                    ..scene(Weather::Storm)
                },
                event(EventKind::Storm, 3, Side::Left),
            );
            later.update(&storm, &overlay(false), now.plus(600));
            assert_eq!(later.crossing(now.plus(700)), None);
        }
    }

    #[test]
    fn each_instance_plays_once_however_many_scenes_carry_it() {
        let rain = carrying(
            scene(Weather::Clear),
            event(EventKind::RainAfterDrought, 40, Side::Left),
        );
        let (mut composer, now) = played(&scene(Weather::Clear), &rain, preset::SALT);
        let end = composer.settled_at();
        // The engine keeps the event in the scenes after, through the next hour.
        composer.update(&rain, &overlay(false), now.plus(5000));
        assert_eq!(composer.settled_at(), end, "not begun again");
        composer.update(&rain, &overlay(false), end.plus(1000));
        assert!(of(&composer, end.plus(5000), |s| s == Subject::Rain).is_empty());
        // Another instance of the kind plays.
        let again = carrying(
            scene(Weather::Clear),
            event(EventKind::RainAfterDrought, 41, Side::Left),
        );
        composer.update(&again, &overlay(false), end.plus(2000));
        assert!(!of(&composer, end.plus(6000), |s| s == Subject::Rain).is_empty());
    }

    #[test]
    fn an_instance_arriving_mid_play_is_dropped() {
        let rain = carrying(
            scene(Weather::Clear),
            event(EventKind::RainAfterDrought, 40, Side::Left),
        );
        let (mut composer, now) = played(&scene(Weather::Clear), &rain, preset::SALT);
        let end = composer.settled_at();
        let blow = carrying(
            scene(Weather::Clear),
            event(EventKind::Windstorm, 41, Side::Left),
        );
        composer.update(&blow, &overlay(false), now.plus(2000));
        // It is passed over for good, even once the rain is over.
        composer.update(&blow, &overlay(false), end.plus(1000));
        for at in (now.0..end.0 + 60_000).step_by(500) {
            assert!(of(&composer, Millis(at), |s| matches!(s, Subject::Leaf(_))).is_empty());
        }
        assert!(!of(&composer, now.plus(5000), |s| s == Subject::Rain).is_empty());
    }

    #[test]
    fn ground_events_are_left_to_their_own_session() {
        for kind in [
            EventKind::ClearingBurn,
            EventKind::Flowering,
            EventKind::Earthquake,
            EventKind::Sinkhole,
        ] {
            let after = carrying(scene(Weather::Clear), event(kind, 6, Side::Left));
            let (composer, _) = played(&scene(Weather::Clear), &after, preset::SALT);
            assert_eq!(composer.show, None, "{kind:?}");
            assert_eq!(composer.played, None);
        }
    }

    #[test]
    fn a_storm_or_windstorm_raises_the_sway_from_the_next_turn() {
        let calm = SceneInput {
            wind: Wind::Calm,
            ..scene(Weather::Clear)
        };
        let breezy = SceneInput {
            wind: Wind::Breezy,
            ..scene(Weather::Clear)
        };
        let cases = [
            (&calm, None, Wind::Calm),
            (&calm, Some(EventKind::Storm), Wind::Breezy),
            (&breezy, Some(EventKind::Storm), Wind::Gale),
            (&calm, Some(EventKind::Windstorm), Wind::Gale),
            (&breezy, Some(EventKind::Clearing), Wind::Breezy),
            (&breezy, Some(EventKind::RainAfterDrought), Wind::Breezy),
        ];
        for (scene, kind, expected) in cases {
            let input = SceneInput {
                event: kind.map(|kind| event(kind, 1, Side::Left)),
                ..scene.clone()
            };
            assert_eq!(wind(&input), expected, "{kind:?}");
        }
        // Through the turns: a calm terrarium leans only once the storm's turn comes.
        let storm = carrying(
            SceneInput {
                weather: Weather::Storm,
                ..calm.clone()
            },
            event(EventKind::Storm, 1, Side::Left),
        );
        let (composer, now) = played(&calm, &storm, preset::SALT);
        let leaning = |from: u64, to: u64| {
            (from..to).step_by(100).any(|at| {
                of(&composer, Millis(at), |s| {
                    matches!(s, Subject::Plant { .. })
                })
                .iter()
                .any(|k| k.draw.shear.slope != 0)
            })
        };
        let next_turn = crate::motion::turn_start(crate::motion::turn(now) + 1);
        assert!(
            !leaning(now.0, next_turn.0),
            "the turn running keeps its wind"
        );
        let later =
            crate::motion::turn_start(crate::motion::turn(now) + 1 + crate::sway::GUST_EVERY);
        assert!(leaning(next_turn.0, later.0));
    }

    #[test]
    fn rain_after_drought_wakes_residents_by_truth_s_state() {
        // Dormant, everyone sleeps; the first commit back ends dormancy, and the rain's scene
        // wakes them from the next turn.
        let dormant = preset::scene("dormant").unwrap();
        assert!(!dormant.residents.is_empty());
        let mut back = carrying(
            scene(Weather::Clear),
            event(EventKind::RainAfterDrought, 50, Side::Left),
        );
        back.residents.clone_from(&dormant.residents);
        let mut composer = Composer::new(table(), preset::SALT);
        composer.update(&dormant, &overlay(false), Millis(0));
        let now = composer.settled_at();
        let routines = |composer: &Composer, w: u64| -> Vec<String> {
            composer
                .turn_plans(art(), table(), w)
                .iter()
                .flat_map(|plan| &plan.segments)
                .filter_map(|s| art().catalog.animation(s.routine).keys.routine.clone())
                .collect()
        };
        let running = crate::motion::turn(now);
        assert!(
            routines(&composer, running)
                .iter()
                .all(|r| r == "sleep" || r == "idle")
        );
        composer.update(&back, &overlay(false), now);
        assert!(
            routines(&composer, running + 1)
                .iter()
                .any(|r| r != "sleep")
        );
    }

    #[test]
    fn every_choreography_ends_by_settled_at() {
        for (kind, from, to) in [
            (EventKind::Storm, Weather::Clear, Weather::Storm),
            (EventKind::Clearing, Weather::Storm, Weather::Clear),
            (EventKind::Lightning, Weather::Storm, Weather::Storm),
            (EventKind::Dusk, Weather::Clear, Weather::Clear),
            (EventKind::Sunburst, Weather::Clear, Weather::Clear),
            (EventKind::RainAfterDrought, Weather::Clear, Weather::Clear),
            (EventKind::Windstorm, Weather::Clear, Weather::Clear),
        ] {
            let after = carrying(scene(to), event(kind, 8, Side::Right));
            let (composer, _) = played(&scene(from), &after, preset::SALT);
            let settled = composer.settled_at();
            assert!(
                composer.show_end().is_some_and(|end| end <= settled),
                "{kind:?}"
            );
            let pieces = of(&composer, settled, |s| {
                matches!(
                    s,
                    Subject::Front | Subject::Bolt | Subject::Rain | Subject::Leaf(_)
                )
            });
            assert!(pieces.is_empty(), "{kind:?}");
            assert_eq!(composer.show_depicted(settled), None);
        }
    }
}
