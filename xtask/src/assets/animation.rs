//! Check 5: every animation sequence has frames and durations, and every terminal destination
//! parses and exists.
//!
//! Sequences come in four shapes:
//! - the creatures' `animations[]`, which are the authority, and whose sprites repeat them and
//!   must agree;
//! - per-sprite `animation`, `frame` and `durationMs` fields (sky and soil);
//! - the events manifest's top-level `animation` (burn-off), whose frames are the sprites that
//!   name it;
//! - an `effectAnimation` (the firefly lantern), whose frames are the attached-effect sprites.
//!
//! A creature terminal is prose. Its destination is the leftmost match of
//! `(adult|juvenile|egg|pupa|brood|cocoon)\.[a-z-]+`, and it must name a routine of the same
//! species. A terminal with no destination passes only if the routine loops or the text says it
//! holds (`hold…`): the egg, pupa, death and hide routines hold, and idle and sleep loop. Any
//! other terminal fails, so a hand-off such as a hatch cannot lose its destination silently.

use std::collections::{BTreeMap, BTreeSet};

use terrarium_compose::catalog::{AnimationKeys, Flags};

use super::manifest::{AnimationJson, Biome, SpriteDraft};
use super::{Check, Failure};

/// The life stages a destination may name, in the regex's alternation order.
const STAGES: [&str; 6] = ["adult", "juvenile", "egg", "pupa", "brood", "cocoon"];

/// What follows a sequence, by name until the catalog gives names their indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NextDraft {
    Handoff(String),
    Hold,
    Clear,
    Repeat,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationDraft {
    pub name: String,
    pub keys: AnimationKeys,
    /// Sprite IDs.
    pub frames: Vec<String>,
    pub durations_ms: Vec<u16>,
    pub looping: bool,
    pub play_count: u8,
    pub next: NextDraft,
    pub transition_ms: Option<u16>,
    pub cycle_displacement: i16,
    /// The routine an attached effect plays in step with.
    pub sync: Option<String>,
}

/// The destination a terminal names: the leftmost match of the terminal regex, written out so
/// the pipeline needs no regex engine. No stage word is a prefix of another, so at most one
/// alternative can match at a position, and leftmost-first order is the same as the regex's.
pub fn destination(terminal: &str) -> Option<&str> {
    let bytes = terminal.as_bytes();
    (0..bytes.len()).find_map(|start| {
        STAGES.iter().find_map(|stage| {
            let rest = &bytes[start..];
            let after = rest.strip_prefix(stage.as_bytes())?.strip_prefix(b".")?;
            let tail = after
                .iter()
                .take_while(|b| b.is_ascii_lowercase() || **b == b'-')
                .count();
            // Each byte of the match is ASCII, so both ends are character boundaries.
            (tail > 0).then(|| &terminal[start..start + stage.len() + 1 + tail])
        })
    })
}

/// Builds every sequence and reports every way one fails check 5. A sequence that fails is
/// still returned, with what could be read, so the variant check sees what the art offers.
pub fn build(biome: &Biome) -> (Vec<AnimationDraft>, Vec<Failure>) {
    let mut out = Builder {
        biome,
        failures: Vec::new(),
        drafts: Vec::new(),
    };
    out.creatures();
    out.per_sprite();
    out.effects();
    let mut names = BTreeSet::new();
    for draft in &out.drafts {
        if !names.insert(draft.name.clone()) {
            out.failures.push(Failure::new(
                Check::Animations,
                format!("{} is defined twice", draft.name),
            ));
        }
    }
    let mut drafts = out.drafts;
    drafts.sort_by(|a, b| a.name.cmp(&b.name));
    (drafts, out.failures)
}

struct Builder<'a> {
    biome: &'a Biome,
    failures: Vec<Failure>,
    drafts: Vec<AnimationDraft>,
}

impl<'a> Builder<'a> {
    fn fail(&mut self, name: &str, message: impl std::fmt::Display) {
        self.failures.push(Failure::new(
            Check::Animations,
            format!("{name}: {message}"),
        ));
    }

    fn ms(&mut self, name: &str, durations: &[u32]) -> Vec<u16> {
        let converted: Vec<u16> = durations
            .iter()
            .filter_map(|&d| u16::try_from(d).ok())
            .collect();
        if converted.len() != durations.len() || converted.contains(&0) {
            self.fail(name, "every frame duration is 1..=65535 ms");
        }
        converted
    }

    /// The creatures' routines, checked against the copies their sprites carry.
    fn creatures(&mut self) {
        let biome = self.biome;
        let routines: BTreeSet<(&str, &str, &str)> = biome
            .creature_animations
            .iter()
            .map(|a| (a.species.as_str(), a.life_stage.as_str(), a.state.as_str()))
            .collect();
        for animation in &biome.creature_animations {
            self.creature(animation, &routines);
        }
        let names: BTreeSet<&str> = biome
            .creature_animations
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        let claimed: BTreeSet<&str> = biome
            .creature_animations
            .iter()
            .flat_map(|a| a.frames.iter().map(String::as_str))
            .collect();
        for sprite in &biome.sprites {
            let routine = sprite.frame.as_ref().and_then(|f| f.animation.as_deref());
            if routine.is_some_and(|r| names.contains(r)) && !claimed.contains(sprite.id.as_str()) {
                self.fail(&sprite.id, "names a creature routine that does not list it");
            }
        }
    }

    fn creature(&mut self, animation: &AnimationJson, routines: &BTreeSet<(&str, &str, &str)>) {
        let name = animation.name.as_str();
        if animation.tag != format!("{}.{}", animation.life_stage, animation.state)
            || name != format!("{}.{}", animation.species, animation.tag)
        {
            self.fail(name, "its name is not species.lifeStage.state");
        }
        if animation.frames.is_empty() || animation.frames.len() != animation.durations_ms.len() {
            self.fail(name, "needs frames, and one duration per frame");
        }
        let durations = self.ms(name, &animation.durations_ms);
        let displacement = self.frames_repeat_their_routine(animation);
        if displacement != animation.cycle_displacement {
            self.fail(
                name,
                format!(
                    "its rootDelta sums to {displacement}, but cycleDisplacement is {}",
                    animation.cycle_displacement
                ),
            );
        }
        let next = self.terminal(animation, routines);
        let play_count = u8::try_from(animation.play_count).ok();
        let transition_ms = u16::try_from(animation.transition_ms).ok();
        let cycle_displacement = i16::try_from(animation.cycle_displacement).ok();
        if play_count.is_none() || transition_ms.is_none() || cycle_displacement.is_none() {
            self.fail(
                name,
                "playCount, transitionMs or cycleDisplacement is out of range",
            );
        }
        self.drafts.push(AnimationDraft {
            name: name.to_owned(),
            keys: AnimationKeys {
                species: Some(animation.species.clone()),
                life_stage: Some(animation.life_stage.clone()),
                routine: Some(animation.state.clone()),
            },
            frames: animation.frames.clone(),
            durations_ms: durations,
            looping: animation.looping,
            play_count: play_count.unwrap_or_default(),
            next,
            transition_ms,
            cycle_displacement: cycle_displacement.unwrap_or_default(),
            sync: None,
        });
    }

    /// Checks each frame's copy of its routine's fields; returns the frames' summed
    /// `rootDelta.x`.
    fn frames_repeat_their_routine(&mut self, animation: &AnimationJson) -> i32 {
        let name = animation.name.as_str();
        let mut displacement = 0_i32;
        for (index, id) in animation.frames.iter().enumerate() {
            let Some(sprite) = self.biome.sprite(id) else {
                self.fail(name, format!("frame {id} does not exist"));
                continue;
            };
            displacement += sprite.root_delta.map_or(0, |d| i32::from(d.x));
            let frame_agrees = sprite.frame.as_ref().is_some_and(|frame| {
                frame.animation.as_deref() == Some(name)
                    && usize::try_from(frame.index) == Ok(index)
                    && animation.durations_ms.get(index) == Some(&frame.duration_ms)
                    && frame.looping == Some(animation.looping)
                    && frame.play_count == Some(animation.play_count)
            });
            let keys = &sprite.keys;
            let keys_agree = keys.species.as_deref() == Some(animation.species.as_str())
                && keys.life_stage.as_deref() == Some(animation.life_stage.as_str())
                && keys.state.as_deref() == Some(animation.state.as_str());
            if !(frame_agrees && keys_agree && sprite.root_delta.is_some()) {
                self.fail(
                    name,
                    format!("frame {id} does not repeat the routine's own fields"),
                );
            }
        }
        displacement
    }

    /// The terminal rule: a destination that exists, a loop, or a hold.
    fn terminal(
        &mut self,
        animation: &AnimationJson,
        routines: &BTreeSet<(&str, &str, &str)>,
    ) -> NextDraft {
        let name = animation.name.as_str();
        match destination(&animation.terminal) {
            Some(dest) => {
                let (life_stage, routine) = dest.split_once('.').unwrap_or((dest, ""));
                if !routines.contains(&(animation.species.as_str(), life_stage, routine)) {
                    self.fail(
                        name,
                        format!(
                            "its terminal names {dest}, which {} lacks",
                            animation.species
                        ),
                    );
                }
                NextDraft::Handoff(format!("{}.{dest}", animation.species))
            }
            None if animation.looping => NextDraft::Repeat,
            None if animation.terminal.starts_with("hold") => NextDraft::Hold,
            None => {
                self.fail(
                    name,
                    format!(
                        "its terminal \"{}\" names no destination, and the routine neither \
                         loops nor holds",
                        animation.terminal
                    ),
                );
                NextDraft::Hold
            }
        }
    }

    /// The sprites that carry frame fields for a sequence of their own, by sequence name. A
    /// creature frame belongs to an `animations[]` routine, so one that names another is a
    /// mistake, never a sequence of its own.
    fn per_sprite_groups(&mut self) -> BTreeMap<&'a str, Vec<&'a SpriteDraft>> {
        let biome = self.biome;
        let creature_names: BTreeSet<&str> = biome
            .creature_animations
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        let mut groups: BTreeMap<&str, Vec<&SpriteDraft>> = BTreeMap::new();
        for sprite in &biome.sprites {
            let Some(name) = sprite.frame.as_ref().and_then(|f| f.animation.as_deref()) else {
                continue;
            };
            if creature_names.contains(name) {
                continue;
            }
            if sprite.keys.life_stage.is_some() {
                self.fail(&sprite.id, format!("names {name}, which no routine lists"));
            } else {
                groups.entry(name).or_default().push(sprite);
            }
        }
        groups
    }

    /// Sky, soil and burn-off: sequences assembled from their sprites' frame fields.
    fn per_sprite(&mut self) {
        let groups = self.per_sprite_groups();
        for (name, mut frames) in groups {
            frames.sort_by_key(|s| s.frame.as_ref().map(|f| f.index));
            let contiguous = frames.iter().enumerate().all(|(i, s)| {
                s.frame
                    .as_ref()
                    .is_some_and(|f| usize::try_from(f.index) == Ok(i))
            });
            if !contiguous {
                self.fail(name, "its frames are not numbered 0, 1, 2… once each");
            }
            let loops: BTreeSet<Option<bool>> = frames
                .iter()
                .map(|s| s.frame.as_ref().and_then(|f| f.looping))
                .collect();
            let plays: BTreeSet<Option<u32>> = frames
                .iter()
                .map(|s| s.frame.as_ref().and_then(|f| f.play_count))
                .collect();
            if loops.len() != 1 || plays.len() != 1 {
                self.fail(name, "its frames disagree on `loop` or `playCount`");
            }
            let looping = loops.first().copied().flatten().unwrap_or(false);
            let raw: Vec<u32> = frames
                .iter()
                .filter_map(|s| s.frame.as_ref().map(|f| f.duration_ms))
                .collect();
            let durations = self.ms(name, &raw);
            let settled: Vec<bool> = frames
                .iter()
                .map(|s| s.flags.contains(Flags::SETTLED))
                .collect();
            if settled.iter().rev().skip(1).any(|&s| s) {
                self.fail(name, "only its last frame may be settled");
            }
            let top = self.biome.top_animations.iter().find(|t| t.name == name);
            if let Some(top) = top {
                let total: u32 = raw.iter().sum();
                if usize::try_from(top.frames) != Ok(frames.len())
                    || top.duration_ms != total
                    || top.looping != looping
                    || plays.first().copied().flatten() != Some(top.play_count)
                {
                    self.fail(name, format!(
                        "its {} frames of {total} ms disagree with the manifest's {} frames of {} ms",
                        frames.len(),
                        top.frames,
                        top.duration_ms
                    ));
                }
            }
            let next = if looping {
                NextDraft::Repeat
            } else if top.is_some_and(|top| top.remove_final_frame) {
                NextDraft::Clear
            } else if settled.last() == Some(&true) {
                NextDraft::Hold
            } else {
                NextDraft::Clear
            };
            let play_count = u8::try_from(
                plays
                    .first()
                    .copied()
                    .flatten()
                    .unwrap_or(u32::from(!looping)),
            )
            .unwrap_or(1);
            self.drafts.push(AnimationDraft {
                name: name.to_owned(),
                keys: AnimationKeys::default(),
                frames: frames.iter().map(|s| s.id.clone()).collect(),
                durations_ms: durations,
                looping,
                play_count,
                next,
                transition_ms: None,
                cycle_displacement: 0,
                sync: None,
            });
        }
        for top in &self.biome.top_animations {
            if !self.drafts.iter().any(|d| d.name == top.name) {
                self.fail(&top.name, "no sprite is one of its frames");
            }
        }
    }

    /// Attached effects: a finite sequence of attached-effect sprites, in step with a routine,
    /// removed after a last frame of opacity 0.
    fn effects(&mut self) {
        let biome = self.biome;
        for effect in &biome.effects {
            let name = effect.tag.as_str();
            // An effect's frames are the attached-effect sprites whose host frame belongs to
            // the routine it syncs to, so two effects can never share frames.
            let sync = biome
                .creature_animations
                .iter()
                .find(|a| a.name == effect.sync);
            if sync.is_none() {
                self.fail(
                    name,
                    format!("it syncs to {}, which does not exist", effect.sync),
                );
            }
            let hosts: BTreeSet<&str> = sync
                .iter()
                .flat_map(|a| a.frames.iter().map(String::as_str))
                .collect();
            let mut frames: Vec<&SpriteDraft> = biome
                .sprites
                .iter()
                .filter(|s| {
                    s.keys.part.as_deref() == Some("attached-effect")
                        && s.attachment
                            .as_deref()
                            .is_some_and(|host| hosts.contains(host))
                })
                .collect();
            frames.sort_by_key(|s| s.frame.as_ref().map(|f| f.index));
            let species = sync.map(|a| a.species.clone());
            let life_stage = sync.map(|a| a.life_stage.clone());
            let foreign = frames.iter().any(|s| {
                s.frame.as_ref().is_none_or(|f| f.animation.is_some())
                    || s.keys.species != species
                    || s.keys.life_stage != life_stage
            });
            if foreign {
                self.fail(
                    name,
                    "an effect frame is not an unnamed frame of its routine's species and stage",
                );
            }
            let contiguous = frames.iter().enumerate().all(|(i, s)| {
                s.frame
                    .as_ref()
                    .is_some_and(|f| usize::try_from(f.index) == Ok(i))
            });
            if frames.is_empty()
                || !contiguous
                || usize::try_from(effect.frames) != Ok(frames.len())
            {
                self.fail(
                    name,
                    format!(
                        "it declares {} frames, and {} attached-effect sprites number 0, 1, 2…",
                        effect.frames,
                        frames.len()
                    ),
                );
            }
            if effect.looping || frames.last().is_some_and(|s| s.opacity != 0) {
                self.fail(name, "an attached effect plays once and ends at opacity 0");
            }
            let raw: Vec<u32> = frames
                .iter()
                .filter_map(|s| s.frame.as_ref().map(|f| f.duration_ms))
                .collect();
            let durations = self.ms(name, &raw);
            self.drafts.push(AnimationDraft {
                name: name.to_owned(),
                keys: AnimationKeys {
                    species,
                    life_stage,
                    routine: frames.first().and_then(|s| s.keys.state.clone()),
                },
                frames: frames.iter().map(|s| s.id.clone()).collect(),
                durations_ms: durations,
                looping: false,
                play_count: u8::try_from(effect.play_count).unwrap_or(1),
                next: NextDraft::Clear,
                transition_ms: None,
                cycle_displacement: 0,
                sync: Some(effect.sync.clone()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_destination_is_the_leftmost_match() {
        assert_eq!(
            destination("finish cycle, then adult.idle"),
            Some("adult.idle")
        );
        assert_eq!(
            destination("juvenile.idle; never replay"),
            Some("juvenile.idle")
        );
        assert_eq!(
            destination("adult.hide, same final silhouette"),
            Some("adult.hide")
        );
        assert_eq!(
            destination("adult.flight; keep origin at last perch"),
            Some("adult.flight")
        );
        assert_eq!(
            destination("then egg.hatch or adult.idle"),
            Some("egg.hatch")
        );
        assert_eq!(destination("to cocoon.x-y-z."), Some("cocoon.x-y-z"));
        assert_eq!(destination("then pupa.x"), Some("pupa.x"));
    }

    #[test]
    fn holds_and_loops_name_no_destination() {
        // The woodlouse text copied onto the springtail, harvestman and earthworm names the
        // brood, but not as `brood.<routine>`, so it is a hold, as delivered.
        assert_eq!(
            destination(
                "hold until lifecycle permits hatch; woodlouse brood represents protected pouch young"
            ),
            None
        );
        assert_eq!(destination("finish cycle then idle; long quiet hold"), None);
        assert_eq!(
            destination("repeat only during selected idle routine"),
            None
        );
        assert_eq!(destination("adult."), None);
        assert_eq!(destination("adult.Idle"), None);
        assert_eq!(destination("adults.idle"), None);
    }

    #[test]
    fn a_stage_inside_a_word_still_matches_as_the_regex_does() {
        assert_eq!(destination("preadult.idle"), Some("adult.idle"));
        assert_eq!(destination("×adult.idle"), Some("adult.idle"));
        assert_eq!(destination(""), None);
    }
}
