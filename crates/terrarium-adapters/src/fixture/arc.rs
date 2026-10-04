//! The scripted arc: about two years of a fictional repository that thrives, struggles, dies
//! and recovers, with tags, a feature branch merged back, a rename, a large directory deletion
//! and an unmerged experiment. Golden replays and the engine's tests run on it, with the
//! observations in `tests/fixtures/arc/observations.jsonl`.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::Path;

use terrarium_truth::{CommitId, Pcg32};

use super::{Edit, FixtureRepo, FixtureResult, Stamp};

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;

/// Monday 2024-01-08, 00:00 UTC: day 0 of the arc.
pub const ARC_START_SECS: i64 = 1_704_672_000;
/// Saturday 2025-12-20, 00:00 UTC: the day after the last commit.
pub const ARC_END_SECS: i64 = ARC_START_SECS + 712 * DAY;
/// Five months past the last commit: where the golden replay and the engine's tests end it,
/// long enough for dormancy to mist the glass.
pub const ARC_REPLAY_END_SECS: i64 = ARC_END_SECS + 150 * DAY;

/// The directory renamed in July of the first year, and where it went.
pub const ARC_RENAMED_FROM: &str = "src/soil";
pub const ARC_RENAMED_TO: &str = "src/ground";
/// The directory one commit deletes whole in November of the first year.
pub const ARC_DELETED_DIR: &str = "src/legacy";
/// The arc's tags, in the order they are made.
pub const ARC_TAGS: &[&str] = &["v0.1.0", "v0.2.0", "v1.0.0", "v2.0.0"];
/// Every local branch the arc ends with.
pub const ARC_BRANCHES: &[&str] = &["experiment/moss", "feature/trellis", "main"];

/// The arc's four stretches, by what the repository's activity and health are meant to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Jan–Sep 2024: two commits most weekdays.
    Thriving,
    /// Oct 2024–Feb 2025: three commits a week, health falling.
    Struggling,
    /// Mar–Jul 2025: no commits at all.
    Dead,
    /// Aug–Dec 2025: commits resume and health climbs back.
    Recovering,
}

impl Phase {
    /// The phase of a UTC time within the arc.
    pub fn at(secs: i64) -> Phase {
        match (secs - ARC_START_SECS).div_euclid(DAY) {
            ..267 => Phase::Thriving,
            267..418 => Phase::Struggling,
            418..571 => Phase::Dead,
            _ => Phase::Recovering,
        }
    }

    /// The first day of the phase, in UTC seconds.
    pub fn start_secs(self) -> i64 {
        ARC_START_SECS
            + DAY
                * match self {
                    Phase::Thriving => 0,
                    Phase::Struggling => 267,
                    Phase::Dead => 418,
                    Phase::Recovering => 571,
                }
    }
}

/// One commit of the arc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArcCommit {
    pub id: CommitId,
    /// Committer time, UTC seconds.
    pub secs: i64,
    pub phase: Phase,
    /// The branch it was made on.
    pub branch: &'static str,
}

/// The arc as built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arc {
    /// In the order they were made, which is time order.
    pub commits: Vec<ArcCommit>,
    pub default_branch: &'static str,
    /// Tag name to the commit it marks.
    pub tags: BTreeMap<&'static str, CommitId>,
    /// The merge of `feature/trellis` into `main`.
    pub merge: CommitId,
    /// The commit that renames [`ARC_RENAMED_FROM`].
    pub rename: CommitId,
    /// The commit that deletes [`ARC_DELETED_DIR`].
    pub deletion: CommitId,
}

impl Arc {
    /// The latest commit on `main` at or before `secs`: where `HEAD` was, for an observation
    /// recorded then.
    pub fn main_tip_at(&self, secs: i64) -> Option<&ArcCommit> {
        self.commits
            .iter()
            .filter(|c| c.branch == "main" && c.secs <= secs)
            .max_by_key(|c| c.secs)
    }
}

const WORDS: &[&str] = &[
    "fern", "moss", "reed", "wren", "loam", "sedge", "birch", "clover", "lichen", "rowan",
];
const VERBS: &[&str] = &["tend", "prune", "water", "graft", "mulch", "stake", "sow"];

/// The arc's text, drawn from a fixed stream.
struct Writer {
    rng: Pcg32,
    serial: u32,
    files: BTreeMap<String, Vec<String>>,
}

impl Writer {
    fn below(&mut self, bound: u32) -> u32 {
        self.rng
            .below(NonZeroU32::new(bound).expect("a positive bound"))
    }

    fn line(&mut self) -> String {
        self.serial += 1;
        let word = WORDS[self.below(10) as usize];
        let value = self.below(1000);
        format!("    let {word}_{} = {value};", self.serial)
    }

    fn new_file(&mut self, path: &str, lines: usize) -> Edit {
        let body: Vec<String> = (0..lines).map(|_| self.line()).collect();
        self.files.insert(path.to_owned(), body);
        self.edit(path)
    }

    fn edit(&self, path: &str) -> Edit {
        let mut text = self.files[path].join("\n");
        text.push('\n');
        Edit::Write(path.to_owned(), text.into_bytes())
    }

    /// Appends a few lines to `path` and rewrites one or two.
    fn tend(&mut self, path: &str) -> Edit {
        let grow = 1 + self.below(6) as usize;
        let rewrite = 1 + self.below(2) as usize;
        for _ in 0..grow {
            let line = self.line();
            self.files.get_mut(path).expect("a known file").push(line);
        }
        for _ in 0..rewrite {
            let len = u32::try_from(self.files[path].len()).expect("short files");
            let at = self.below(len) as usize;
            let line = self.line();
            self.files.get_mut(path).expect("a known file")[at] = line;
        }
        self.edit(path)
    }

    fn rename(&mut self, from: &str, to: &str) -> [Edit; 2] {
        let body = self.files.remove(from).expect("a known file");
        self.files.insert(to.to_owned(), body);
        [Edit::Remove(from.to_owned()), self.tend(to)]
    }
}

struct Script {
    repo: FixtureRepo,
    writer: Writer,
    commits: Vec<ArcCommit>,
    /// Files the daily work edits.
    tended: Vec<String>,
}

impl Script {
    fn commit(
        &mut self,
        branch: &'static str,
        secs: i64,
        message: &str,
        edits: &[Edit],
    ) -> FixtureResult<CommitId> {
        let author = self.writer.below(4) as usize;
        let stamp = Stamp {
            secs,
            offset: [0, 3_600, -18_000, 19_800][author],
            author,
        };
        let id = self.repo.commit(branch, stamp, message, edits)?;
        self.record(branch, secs, &id);
        Ok(id)
    }

    fn record(&mut self, branch: &'static str, secs: i64, id: &CommitId) {
        self.commits.push(ArcCommit {
            id: id.clone(),
            secs,
            phase: Phase::at(secs),
            branch,
        });
    }

    /// A day's ordinary work on `main`: one to three tended files.
    fn work(&mut self, secs: i64) -> FixtureResult<CommitId> {
        let count = 1 + self.writer.below(3);
        let mut edits = Vec::new();
        let mut names = Vec::new();
        for _ in 0..count {
            let pick = self.writer.below(u32::try_from(self.tended.len())?) as usize;
            let path = self.tended[pick].clone();
            if !names.contains(&path) {
                edits.push(self.writer.tend(&path));
                names.push(path);
            }
        }
        let verb = VERBS[self.writer.below(7) as usize];
        let stem = names[0].rsplit('/').next().unwrap_or("tree").to_owned();
        self.commit("main", secs, &format!("{verb} {stem}"), &edits)
    }
}

const fn day(n: i64) -> i64 {
    ARC_START_SECS + n * DAY
}

/// Commits and tags the arc's tests look for.
#[derive(Default)]
struct Landmarks {
    tags: BTreeMap<&'static str, CommitId>,
    merge: Option<CommitId>,
    rename: Option<CommitId>,
    deletion: Option<CommitId>,
}

impl Script {
    /// Day 0: the meadow, its legacy roots, and files that never count (a lockfile, vendored
    /// code, a binary image) or count as tiny text (an LFS pointer).
    fn plant(&mut self) -> FixtureResult<()> {
        let w = &mut self.writer;
        let mut first = vec![
            w.new_file("README.md", 12),
            w.new_file("src/meadow.rs", 40),
            w.new_file("src/pond/frog.rs", 30),
            w.new_file("src/soil/loam.rs", 25),
            w.new_file("src/soil/clay.rs", 25),
            w.new_file("src/soil/humus.rs", 25),
            w.new_file("Cargo.toml", 8),
            w.new_file("Cargo.lock", 60),
            w.new_file("vendor/moss/lib.rs", 80),
        ];
        for name in LEGACY {
            first.push(w.new_file(&format!("{ARC_DELETED_DIR}/{name}.rs"), 110));
        }
        let mut leaf = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        leaf.extend((0..=255_u8).cycle().take(300));
        first.push(Edit::Write("assets/leaf.png".into(), leaf));
        first.push(Edit::write(
            "assets/forest.psd",
            "version https://git-lfs.github.com/spec/v1\n\
             oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
             size 52311\n",
        ));
        self.commit("main", day(0) + 9 * HOUR, "plant the meadow", &first)?;
        Ok(())
    }

    /// Day `n`'s ordinary work on `main`, by phase and weekday.
    fn daily(&mut self, n: i64) -> FixtureResult<()> {
        let weekday = n % 7;
        let secs = day(n);
        let at = |hour: i64, w: &mut Writer| secs + hour * HOUR + i64::from(w.below(90)) * 60;
        let hours: &[i64] = match Phase::at(secs) {
            Phase::Thriving if weekday < 5 && n > 0 => &[9, 15],
            Phase::Thriving if weekday < 5 => &[15],
            Phase::Struggling if matches!(weekday, 0 | 2 | 4) => &[11],
            Phase::Recovering if matches!(weekday, 1 | 3) => &[10, 16],
            Phase::Recovering if weekday < 5 => &[10],
            _ => &[],
        };
        for &hour in hours {
            let when = at(hour, &mut self.writer);
            self.work(when)?;
        }
        Ok(())
    }

    /// One commit on a side branch, starting or tending `path`.
    fn side_work(
        &mut self,
        branch: &'static str,
        path: &str,
        secs: i64,
        message: &str,
    ) -> FixtureResult<()> {
        let edit = if self.writer.files.contains_key(path) {
            self.writer.tend(path)
        } else {
            self.writer.new_file(path, 20)
        };
        self.commit(branch, secs, message, &[edit])?;
        Ok(())
    }

    fn tag(
        &mut self,
        marks: &mut Landmarks,
        name: &'static str,
        secs: i64,
        message: Option<&str>,
    ) -> FixtureResult<()> {
        let tip = self.repo.tip("main");
        match message {
            Some(message) => {
                self.repo
                    .tag_annotated(name, &tip, Stamp::at(secs), message)?;
            }
            None => self.repo.tag_lightweight(name, &tip)?,
        }
        marks.tags.insert(name, tip);
        Ok(())
    }

    /// Day `n`'s scripted event, if it has one.
    fn event(&mut self, n: i64, marks: &mut Landmarks) -> FixtureResult<()> {
        let secs = day(n);
        match n {
            52 => self.tag(marks, "v0.1.0", secs + 18 * HOUR, Some("First sprouts"))?,
            125 => self.tag(marks, "v0.2.0", secs, None)?,
            130 => self.repo.branch("feature/trellis", "main")?,
            133..=144 if n % 7 < 5 => {
                let path = if n % 2 == 0 {
                    "src/trellis/vine.rs"
                } else {
                    "src/trellis/post.rs"
                };
                self.side_work(
                    "feature/trellis",
                    path,
                    secs + 13 * HOUR,
                    "build the trellis",
                )?;
            }
            148 => {
                let at = secs + 17 * HOUR;
                let id = self.repo.merge(
                    "main",
                    "feature/trellis",
                    Stamp::at(at),
                    "Merge branch 'feature/trellis'",
                )?;
                self.record("main", at, &id);
                self.tended.push("src/trellis/vine.rs".into());
                marks.merge = Some(id);
            }
            190 => {
                let mut edits = Vec::new();
                for name in ["loam", "clay", "humus"] {
                    let from = format!("{ARC_RENAMED_FROM}/{name}.rs");
                    let to = format!("{ARC_RENAMED_TO}/{name}.rs");
                    edits.extend(self.writer.rename(&from, &to));
                    for tended in &mut self.tended {
                        if *tended == from {
                            tended.clone_from(&to);
                        }
                    }
                }
                let at = secs + 19 * HOUR;
                marks.rename = Some(self.commit("main", at, "move soil to ground", &edits)?);
            }
            260 => self.tag(marks, "v1.0.0", secs + 18 * HOUR, Some("In bloom"))?,
            310 => {
                let edits: Vec<Edit> = LEGACY
                    .iter()
                    .map(|name| Edit::Remove(format!("{ARC_DELETED_DIR}/{name}.rs")))
                    .collect();
                let at = secs + 20 * HOUR;
                marks.deletion = Some(self.commit("main", at, "clear the legacy roots", &edits)?);
            }
            600 => self.repo.branch("experiment/moss", "main")?,
            601..=603 => {
                let at = secs + 20 * HOUR;
                self.side_work("experiment/moss", "src/moss/spore.rs", at, "try spores")?;
            }
            705 => self.tag(marks, "v2.0.0", secs + 18 * HOUR, Some("Regrowth"))?,
            _ => {}
        }
        Ok(())
    }
}

/// The files of [`ARC_DELETED_DIR`], 110 lines each.
const LEGACY: [&str; 6] = ["roots", "bark", "sap", "knots", "rings", "burrow"];

/// Builds the arc in `path`, which must be empty or absent.
pub fn arc(path: &Path) -> FixtureResult<Arc> {
    let mut script = Script {
        repo: FixtureRepo::init(path)?,
        writer: Writer {
            rng: Pcg32::from_seed(0x0061_7263_2d66_6978),
            serial: 0,
            files: BTreeMap::new(),
        },
        commits: Vec::new(),
        tended: vec![
            "src/meadow.rs".into(),
            "src/pond/frog.rs".into(),
            "src/soil/loam.rs".into(),
            "src/soil/clay.rs".into(),
            "src/soil/humus.rs".into(),
            "README.md".into(),
        ],
    };
    script.plant()?;
    let mut marks = Landmarks::default();
    for n in 0..712 {
        script.daily(n)?;
        script.event(n, &mut marks)?;
    }
    script.repo.set_origin_head("main")?;
    let missing = |what: &str| format!("the arc script lost its {what}");
    Ok(Arc {
        commits: script.commits,
        default_branch: "main",
        tags: marks.tags,
        merge: marks.merge.ok_or_else(|| missing("merge"))?,
        rename: marks.rename.ok_or_else(|| missing("rename"))?,
        deletion: marks.deletion.ok_or_else(|| missing("deletion"))?,
    })
}
