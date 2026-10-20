//! The label: its sets of lines, each cycling on the plate, and the crossfade from one set to
//! the next.

use crate::blend::scale;
use crate::catalog::Art;
use crate::frame::Draw;
use crate::order::{KeyedDraw, Place, Subject};
use crate::text::{Label, fit};
use crate::tween::durations::{LABEL_CYCLE_MS, LABEL_FADE_MS};
use crate::tween::{Millis, Tween};
use crate::world::{LABEL_X, LABEL_Y};

use super::{Composer, SHOWN, TIME_LAPSE_PACE};

/// The label's text: one set of lines, cycling from when it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LabelLook {
    /// Fitted to the plate once, when the set arrives.
    pub(super) lines: Vec<String>,
    pub(super) since: Millis,
    pub(super) opacity: Tween,
    /// When each line's text arrived: a line swapped in place keeps the cycle's phase, and its
    /// turn counts from then.
    pub(super) arrived: Vec<Millis>,
    /// The lines this set took over unchanged from the set it crossfades from, in the same
    /// phase: drawn whole by this set alone, so they never cross over to themselves.
    pub(super) held: Vec<bool>,
}

impl LabelLook {
    /// The lines showing at `now`, each with its opacity: one line, or two while the cycle
    /// crossfades from one to the next.
    pub(super) fn showing(&self, now: Millis) -> Vec<(usize, u8)> {
        let n = u64::try_from(self.lines.len()).unwrap_or(0);
        if n <= 1 {
            return if n == 1 { vec![(0, SHOWN)] } else { Vec::new() };
        }
        let cycle = u64::from(LABEL_CYCLE_MS);
        let elapsed = now.0.saturating_sub(self.since.0);
        let step = elapsed / cycle;
        let index = |step: u64| usize::try_from(step % n).unwrap_or(0);
        if step == 0 {
            return vec![(0, SHOWN)];
        }
        let started = Millis(self.since.0 + step * cycle);
        let rising = Tween::new(0, SHOWN, started, LABEL_FADE_MS).value(now);
        if rising == SHOWN {
            vec![(index(step), SHOWN)]
        } else {
            vec![(index(step - 1), SHOWN - rising), (index(step), rising)]
        }
    }

    /// When the lines showing next change after `now`: a step of the crossfade running, or
    /// the first step of the next one. `None` for a set of one line, which never cycles.
    pub(super) fn next_change(&self, now: Millis) -> Option<Millis> {
        if self.lines.len() <= 1 {
            return None;
        }
        let cycle = u64::from(LABEL_CYCLE_MS);
        let step = now.0.saturating_sub(self.since.0) / cycle;
        let fade =
            |step: u64| Tween::new(0, SHOWN, Millis(self.since.0 + step * cycle), LABEL_FADE_MS);
        let running = (step > 0).then(|| fade(step).next_step(now)).flatten();
        let next = Millis(self.since.0 + (step + 1) * cycle);
        running.or_else(|| fade(step + 1).next_step(next))
    }
}

impl Composer {
    /// Whether line `index` of the label's lines showing at `now` has had its turn on the plate:
    /// the cycle has moved on from it at least once. A set of one line never moves on, and a
    /// new set starts its cycle again.
    pub fn label_step_done(&self, index: usize, now: Millis) -> bool {
        let Some(look) = self
            .label
            .last()
            .filter(|look| look.opacity.target() == SHOWN)
        else {
            return false;
        };
        if look.lines.len() <= 1 || index >= look.lines.len() {
            return false;
        }
        // The first whole turn of its place that began once its text was there.
        let cycle = u64::from(LABEL_CYCLE_MS);
        let n = u64::try_from(look.lines.len()).unwrap_or(u64::MAX);
        let place = u64::try_from(index).unwrap_or(u64::MAX);
        let arrived = look.arrived.get(index).copied().unwrap_or(look.since);
        let first = arrived.0.saturating_sub(look.since.0).div_ceil(cycle);
        let turn = if first <= place {
            place
        } else {
            place.saturating_add((first - place).div_ceil(n).saturating_mul(n))
        };
        let ends = turn.saturating_add(1).saturating_mul(cycle);
        now.0.saturating_sub(look.since.0) >= ends
    }

    /// Raises a new set of label lines when they change, crossfading from the old. The lines
    /// are fitted to the plate here, once, rather than on every frame. A time-lapse's label
    /// crossfades at its scene's pace, so a date it shows keeps up with the days.
    pub(super) fn update_label(&mut self, lines: &[String], time_lapse: bool, now: Millis) {
        let fade = if time_lapse {
            LABEL_FADE_MS.div_ceil(TIME_LAPSE_PACE)
        } else {
            LABEL_FADE_MS
        };
        let lines: Vec<String> = lines
            .iter()
            .map(|line| fit(line, self.label_capacity))
            .collect();
        self.label
            .retain(|look| look.opacity.target() == SHOWN || now < look.opacity.end());
        let current = self
            .label
            .last()
            .filter(|look| look.opacity.target() == SHOWN);
        if current.is_some_and(|look| look.lines == lines)
            || (current.is_none() && lines.is_empty())
        {
            return;
        }
        // A set of the same length keeps the cycle's phase: lines that are not showing swap
        // silently, and only a changed line showing crosses over. A set whose first line is the
        // one line showing whole takes over in place. Either way an unchanged line never
        // crosses over to itself, which would dim it.
        let mut since = now;
        let mut arrived = vec![now; lines.len()];
        let mut held = vec![false; lines.len()];
        if let Some(look) = current {
            let showing = look.showing(now);
            if look.lines.len() == lines.len() {
                for (i, (old, new)) in look.lines.iter().zip(&lines).enumerate() {
                    if old == new
                        && let (Some(slot), Some(&kept)) = (arrived.get_mut(i), look.arrived.get(i))
                    {
                        *slot = kept;
                    }
                    if let Some(slot) = held.get_mut(i) {
                        *slot = old == new;
                    }
                }
                since = look.since;
                if showing
                    .iter()
                    .all(|&(i, _)| look.lines.get(i) == lines.get(i))
                    && let Some(look) = self.label.last_mut()
                {
                    // Swapped in place, a line no longer holds what the set before showed.
                    for ((slot, old), new) in look.held.iter_mut().zip(&look.lines).zip(&lines) {
                        *slot = *slot && old == new;
                    }
                    look.lines = lines;
                    look.arrived = arrived;
                    return;
                }
            } else if let [(i, SHOWN)] = showing[..]
                && look.lines.get(i) == lines.first()
                && let Some(look) = self.label.last_mut()
            {
                look.held = vec![false; lines.len()];
                look.lines = lines;
                look.since = now;
                look.arrived = arrived;
                return;
            }
        }
        for look in &mut self.label {
            if look.opacity.target() == SHOWN {
                look.opacity = Tween::new(look.opacity.value(now), 0, now, fade);
            }
        }
        if !lines.is_empty() {
            self.label.push(LabelLook {
                lines,
                since,
                opacity: Tween::new(0, SHOWN, now, fade),
                arrived,
                held,
            });
        }
    }

    /// The label's text, over the plate the scenery draws.
    pub(super) fn push_label(&self, art: &Art, now: Millis, out: &mut Vec<KeyedDraw>) {
        let catalog = &art.catalog;
        let Some(label) = Label::new(catalog) else {
            return;
        };
        // The set rising, if one is: a line it holds is its alone, drawn whole.
        let rising = self
            .label
            .last()
            .filter(|look| look.opacity.target() == SHOWN);
        let holds = |look: &LabelLook, line: usize| {
            rising.is_some_and(|rising| {
                rising.since == look.since
                    && rising.held.get(line) == Some(&true)
                    && rising.lines.get(line) == look.lines.get(line)
            })
        };
        let last = self.label.len().saturating_sub(1);
        for (index, look) in (0_u16..).zip(&self.label) {
            let value = look.opacity.value(now);
            let newest = usize::from(index) == last;
            for (line, line_value) in look.showing(now) {
                let held = holds(look, line);
                if !newest && held {
                    continue;
                }
                let value = if newest && held { SHOWN } else { value };
                if value == 0 {
                    continue;
                }
                let opacity = scale(value, line_value);
                let Some(text) = look.lines.get(line) else {
                    continue;
                };
                let part = u8::try_from(line).unwrap_or(u8::MAX);
                for (item, glyph) in (0_u16..).zip(label.fitted_draws(text, LABEL_X, LABEL_Y)) {
                    let opacity = scale(glyph.opacity, opacity);
                    if opacity == 0 {
                        continue;
                    }
                    let place = Place::new(Subject::Label, part, index).item(item);
                    out.push(KeyedDraw::placed(catalog, place, Draw { opacity, ..glyph }));
                }
            }
        }
    }
}
