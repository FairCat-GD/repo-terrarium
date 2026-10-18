//! The history read and replayed: what a read found, the base resumed or replayed in full, the
//! view shown and its scene, and the cache row written for the base.

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use terrarium_adapters::git::{Change, GitError, GitRepo, History, Progress, RewriteReason};
use terrarium_store::{ObservationId, SimCache, StoreError};
use terrarium_truth::domain::LeftCommit;
use terrarium_truth::{
    Basis, Observation, Prepared, SIM_VERSION, SceneInput, SimState, Tick, observation_tick,
};

use super::{Fault, Live, MISSING_HISTORY, Problem, git_fault};
use crate::observe;
use crate::shown::Remembered;

/// How a refresh reached its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replay {
    /// From the start: no usable cached state, or truth refused to resume it.
    Full,
    /// From the cached state before the open hour.
    Resumed,
    /// Nothing new and the same hour: the state shown stands.
    Unchanged,
}

/// What a history read found.
pub(super) struct HistoryRead {
    pub(super) changed: bool,
    /// Read again in full because earlier facts changed.
    pub(super) rewritten: bool,
    /// The default tip's move, when it moved without fast-forwarding, and what it took off.
    pub(super) moved: Option<observe::Rewrite>,
}

/// What a move made while nothing watched took off the chain, as read back: a read the
/// repository cannot complete leaves it unknown, as when the commits are gone, rather than
/// failing the refresh, which would fail the same way every time after. Only a stop ends it.
pub(super) fn read_back(
    read: Result<Option<Vec<LeftCommit>>, GitError>,
) -> Result<Option<Vec<LeftCommit>>, Fault> {
    match read {
        Ok(left) => Ok(left),
        Err(GitError::Cancelled) => Err(Fault::Cancelled),
        Err(_) => Ok(None),
    }
}

/// The state shown, and the tick it was advanced through.
pub(super) struct View {
    pub(super) live: SimState,
    pub(super) through: Option<Tick>,
}

impl Live {
    /// The view's scene, playing the event [`Showing`] chooses. What an earlier session's
    /// scenes played counts as seen once the history it is keyed to is read.
    ///
    /// [`Showing`]: crate::showing::Showing
    pub(super) fn scene(&mut self) -> Option<SceneInput> {
        let view = self.view.as_ref()?;
        if let (Some(remembered), Some(history)) = (self.remembered.take(), &self.history)
            && let Some(seen) = remembered.seen_for(
                &self.terrarium.seed,
                &self.terrarium.repo.default_branch,
                history.grid,
            )
        {
            self.showing.also_seen(seen);
        }
        Some(self.showing.scene(&view.live))
    }

    /// Stores what the scenes have had the chance to play, when it changed.
    pub(super) fn remember_seen(&mut self) -> Result<(), StoreError> {
        let (Some(seen), Some(history)) = (self.showing.seen(), &self.history) else {
            return Ok(());
        };
        if self.stored_seen.as_ref() == Some(seen) {
            return Ok(());
        }
        Remembered::new(
            &self.terrarium.seed,
            &self.terrarium.repo.default_branch,
            history.grid,
            seen.clone(),
        )
        .write(&mut self.store)?;
        self.stored_seen = Some(seen.clone());
        Ok(())
    }

    /// Reads the history in full the first time and what is new afterwards.
    pub(super) fn read_history(
        &mut self,
        git: &GitRepo,
        branch: &str,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<HistoryRead, Fault> {
        let started = Instant::now();
        let after = self.progress_after;
        let mut last = None;
        let mut on_progress = |p: Progress| {
            if cancel.load(Ordering::Relaxed) {
                return ControlFlow::Break(());
            }
            if p.total > 0 && started.elapsed() >= after {
                let percent = u8::try_from(p.diffed.saturating_mul(100) / p.total).unwrap_or(100);
                if last != Some(percent) {
                    last = Some(percent);
                    progress(percent);
                }
            }
            ControlFlow::Continue(())
        };
        let Some(history) = self.history.as_mut() else {
            let history = git
                .read_history(branch, &mut on_progress)
                .map_err(git_fault)?;
            // A move made while nothing watched: the cached state's default tip has left the
            // first-parent chain without the branch fast-forwarding past it. What it took off
            // is read back from the object database, which keeps it until it is collected.
            let moved = match self
                .base
                .as_ref()
                .and_then(|base| base.habitat.default_tip.clone())
                .filter(|tip| !history.on_default_chain(tip))
                .zip(history.state.branches.get(branch).cloned())
                .filter(|(old, new)| !history.is_ancestor(old, new))
            {
                Some((old, new)) => {
                    // Its own pace, not the history's: the read's percent is not shown again.
                    let mut stop = |_: Progress| {
                        if cancel.load(Ordering::Relaxed) {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    };
                    let left = read_back(git.left_between(&old, &history, &mut stop))?;
                    Some(observe::Rewrite { old, new, left })
                }
                None => None,
            };
            self.history = Some(history);
            return Ok(HistoryRead {
                changed: true,
                rewritten: false,
                moved,
            });
        };
        let old = history.state.branches.get(branch).cloned();
        let change = git
            .read_since(history, branch, &mut on_progress)
            .map_err(git_fault)?;
        let moved = match &change {
            Change::Rewritten { reason, left_chain }
                if *reason != RewriteReason::DefaultBranchChanged =>
            {
                let new = history.state.branches.get(branch).cloned();
                old.zip(new)
                    .filter(|(old, new)| old != new && !history.is_ancestor(old, new))
                    .map(|(old, new)| observe::Rewrite {
                        old,
                        new,
                        left: Some(left_chain.clone()),
                    })
            }
            _ => None,
        };
        Ok(HistoryRead {
            changed: change != Change::Unchanged,
            rewritten: matches!(change, Change::Rewritten { .. }),
            moved,
        })
    }

    /// Replays through `through`: the base resumed or replayed in full through the hour before,
    /// then a copy advanced through `through`. Caches the base when it moved.
    pub(super) fn replay(
        &mut self,
        through: Option<Tick>,
    ) -> Result<(Replay, Option<Problem>), Fault> {
        let history = self.history.as_ref().ok_or(MISSING_HISTORY)?;
        let basis = Basis::new(self.terrarium.seed, history.grid, &history.default_branch);
        let prepared = Prepared::new(basis.clone(), &history.commits, &self.observations)
            .map_err(|err| Fault::Problem(Problem::Replay(err)))?;
        let replay_err = |err| Fault::Problem(Problem::Replay(err));
        let mut replay = Replay::Resumed;
        let (base, live) = match through {
            None => (None, SimState::new(basis)),
            Some(tick) if tick == Tick::ZERO => {
                let mut live = SimState::new(basis);
                live.advance_through(&prepared, tick).map_err(replay_err)?;
                replay = Replay::Full;
                (None, live)
            }
            Some(tick) => {
                let before = Tick::new(tick.hours() - 1);
                let resumed = self
                    .base
                    .take()
                    .filter(|base| base.next_tick <= tick)
                    .and_then(|mut base| {
                        base.advance_through(&prepared, before).ok().map(|()| base)
                    });
                let base = if let Some(base) = resumed {
                    base
                } else {
                    replay = Replay::Full;
                    let mut base = SimState::new(basis);
                    base.advance_through(&prepared, before)
                        .map_err(replay_err)?;
                    base
                };
                let mut live = base.clone();
                live.advance_through(&prepared, tick).map_err(replay_err)?;
                (Some(base), live)
            }
        };
        let mut problem = None;
        if let Some(base) = &base
            && (replay == Replay::Full || self.cached_tick != Some(base.next_tick))
        {
            let cache = cache_of(base, history, &self.observations);
            match cache.and_then(|cache| self.store.write_sim_cache(self.terrarium.id, &cache)) {
                Ok(()) => self.cached_tick = Some(base.next_tick),
                Err(err) => problem = Some(err.into()),
            }
        }
        self.base = base;
        self.view = Some(View { live, through });
        Ok((replay, problem))
    }
}

/// The cache row for `base`. Its high-water mark is the newest observation recorded before the
/// base's next tick; truth's own check of what the state consumed is what decides whether it
/// resumes.
fn cache_of(
    base: &SimState,
    history: &History,
    observations: &[Observation],
) -> Result<SimCache, StoreError> {
    let high_water = observations
        .iter()
        .filter(|o| {
            observation_tick(history.grid, o.recorded_at).is_some_and(|t| t < base.next_tick)
        })
        .map(|o| o.id)
        .max()
        .map(ObservationId::new);
    Ok(SimCache {
        sim_version: SIM_VERSION,
        tips_hash: history.state.state_hash(),
        obs_high_water: high_water,
        tick: base.next_tick,
        state: postcard::to_allocvec(base).map_err(|_| StoreError::Corrupt("state"))?,
    })
}
