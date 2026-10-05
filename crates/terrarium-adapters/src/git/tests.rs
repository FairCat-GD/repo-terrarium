use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::ControlFlow;
use std::path::Path;

use proptest::prelude::*;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use terrarium_truth::domain::{DeletedDir, Rename, TagFact};
use terrarium_truth::{CommitFacts, CommitId, RepoSeed, Tick};

use super::*;
use crate::fixture::{self, Edit, FixtureRepo, Stamp};

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
/// 2024-03-04, 00:00 UTC.
const T0: i64 = 1_709_510_400;

fn go(_: Progress) -> ControlFlow<()> {
    ControlFlow::Continue(())
}

fn temp() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

fn new_repo(dir: &TempDir) -> FixtureRepo {
    FixtureRepo::init(&dir.path().join("repo")).expect("init")
}

fn open(repo: &FixtureRepo) -> GitRepo {
    GitRepo::open(repo.path()).expect("opens")
}

fn read(repo: &FixtureRepo) -> History {
    open(repo).read_history("main", &mut go).expect("reads")
}

/// A history brought up to date, and what changed.
struct Update {
    history: History,
    change: Change,
}

fn since_on(repo: &FixtureRepo, previous: &History, branch: &str) -> Update {
    let mut history = previous.clone();
    let change = open(repo)
        .read_since(&mut history, branch, &mut go)
        .expect("reads");
    Update { history, change }
}

fn since(repo: &FixtureRepo, previous: &History) -> Update {
    since_on(repo, previous, &previous.default_branch)
}

fn at(hours: i64) -> Stamp {
    Stamp::at(T0 + hours * HOUR)
}

fn text(word: &str, lines: i64) -> String {
    (0..lines).fold(String::new(), |mut text, n| {
        let _ = writeln!(text, "{word} {n}");
        text
    })
}

fn facts<'h>(history: &'h History, id: &CommitId) -> &'h CommitFacts {
    history
        .commits
        .iter()
        .find(|c| &c.id == id)
        .unwrap_or_else(|| panic!("{} was not read", id.as_str()))
}

fn commit(repo: &mut FixtureRepo, branch: &str, hours: i64, edits: &[Edit]) -> CommitId {
    repo.commit(branch, at(hours), "grow", edits)
        .expect("commits")
}

#[test]
fn numstat_counts_text_lines_and_skips_what_does_not_count() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let other = CommitId::from_hex(&"ab".repeat(20)).unwrap();
    let mut binary = b"leaf\0".to_vec();
    binary.extend_from_slice(text("x", 40).as_bytes());
    let first = commit(
        &mut repo,
        "main",
        0,
        &[
            Edit::write("src/a.rs", &text("fern", 10)),
            Edit::write("vendor/x.rs", &text("moss", 5)),
            Edit::write("Cargo.lock", &text("lock", 4)),
            Edit::write("web/app.min.js", &text("min", 3)),
            Edit::Write("assets/leaf.png".into(), binary),
            Edit::write(
                "assets/forest.psd",
                "version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 9\n",
            ),
            Edit::Symlink("link".into(), "src/a.rs".into()),
            Edit::Gitlink("modules/pond".into(), other),
            Edit::Executable("run.sh".into(), text("sh", 2).into_bytes()),
        ],
    );
    let mut modified = text("fern", 10).replace("fern 3\n", "reed 3\n");
    modified = modified.replace("fern 7\n", "reed 7\n");
    modified.push_str(&text("wren", 3));
    let second = commit(
        &mut repo,
        "main",
        1,
        &[
            Edit::write("src/a.rs", &modified),
            Edit::write("vendor/x.rs", &text("moss", 50)),
            Edit::Symlink("link".into(), "elsewhere".into()),
        ],
    );
    let third = commit(&mut repo, "main", 2, &[Edit::Remove("src/a.rs".into())]);
    let history = read(&repo);
    // 10 + the LFS pointer's 3 + the executable's 2; everything else is excluded or binary.
    let f = facts(&history, &first);
    assert_eq!((f.added, f.removed, f.parents), (15, 0, 0));
    let s = facts(&history, &second);
    assert_eq!((s.added, s.removed, s.parents), (5, 2, 1));
    let t = facts(&history, &third);
    assert_eq!((t.added, t.removed), (0, 13));
    // `src/` held only that file, so the directory is deleted whole.
    assert_eq!(
        t.deleted_dirs,
        [DeletedDir {
            path: "src".into(),
            lines: 13
        }]
    );
    assert_eq!(history.commits.len(), 3);
    assert_eq!(history.commits[0].id, first);
}

#[test]
fn a_rename_is_listed_and_is_not_a_deletion() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let names = ["loam", "clay", "humus"];
    let edits: Vec<Edit> = names
        .iter()
        .map(|n| Edit::write(&format!("src/soil/{n}.rs"), &text(n, 30)))
        .collect();
    let mut edits = edits;
    edits.push(Edit::write("src/lib.rs", "pub mod ground;\n"));
    commit(&mut repo, "main", 0, &edits);
    let mut moved = Vec::new();
    for n in names {
        moved.push(Edit::Remove(format!("src/soil/{n}.rs")));
        let body = text(n, 30).replace(&format!("{n} 4\n"), "changed\n");
        moved.push(Edit::write(&format!("src/ground/{n}.rs"), &body));
    }
    let rename = commit(&mut repo, "main", 1, &moved);
    let removed: Vec<Edit> = names
        .iter()
        .map(|n| Edit::Remove(format!("src/ground/{n}.rs")))
        .collect();
    let deletion = commit(&mut repo, "main", 2, &removed);
    let history = read(&repo);
    let r = facts(&history, &rename);
    assert!(r.deleted_dirs.is_empty(), "{:?}", r.deleted_dirs);
    let expected: Vec<Rename> = ["clay", "humus", "loam"]
        .iter()
        .map(|n| Rename {
            from: format!("src/soil/{n}.rs"),
            to: format!("src/ground/{n}.rs"),
        })
        .collect();
    assert_eq!(r.renames, expected);
    assert_eq!((r.added, r.removed), (3, 3));
    let d = facts(&history, &deletion);
    assert_eq!(
        d.deleted_dirs,
        [DeletedDir {
            path: "src/ground".into(),
            lines: 90
        }]
    );
    assert_eq!(d.removed, 90);
    assert!(d.renames.is_empty());
}

#[test]
fn a_directory_moved_unchanged_is_renamed_file_by_file() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let names = ["loam", "clay", "humus"];
    let mut edits: Vec<Edit> = names
        .iter()
        .map(|n| Edit::write(&format!("src/soil/{n}.rs"), &text(n, 30)))
        .collect();
    edits.push(Edit::write("src/lib.rs", "pub mod soil;\n"));
    commit(&mut repo, "main", 0, &edits);
    let mut moved = Vec::new();
    for n in names {
        moved.push(Edit::Remove(format!("src/soil/{n}.rs")));
        moved.push(Edit::write(&format!("src/ground/{n}.rs"), &text(n, 30)));
    }
    let rename = commit(&mut repo, "main", 1, &moved);
    let history = read(&repo);
    let r = facts(&history, &rename);
    assert_eq!((r.added, r.removed), (0, 0));
    assert!(r.deleted_dirs.is_empty(), "{:?}", r.deleted_dirs);
    assert_eq!(r.renames.len(), 3, "{:?}", r.renames);
}

#[test]
fn deleted_directories_are_topmost_and_skip_files_renamed_out() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    commit(
        &mut repo,
        "main",
        0,
        &[
            Edit::write("old/a.rs", &text("a", 20)),
            Edit::write("old/sub/b.rs", &text("b", 10)),
            Edit::write("old/c.rs", &text("c", 25)),
            Edit::write("old/vendor/v.rs", &text("v", 99)),
            Edit::write("keep.rs", &text("k", 1)),
        ],
    );
    let deletion = commit(
        &mut repo,
        "main",
        1,
        &[
            Edit::Remove("old/a.rs".into()),
            Edit::Remove("old/sub/b.rs".into()),
            Edit::Remove("old/c.rs".into()),
            Edit::Remove("old/vendor/v.rs".into()),
            Edit::write("new/c.rs", &text("c", 25)),
        ],
    );
    let history = read(&repo);
    let d = facts(&history, &deletion);
    assert_eq!(
        d.deleted_dirs,
        [DeletedDir {
            path: "old".into(),
            lines: 30
        }]
    );
    assert_eq!(
        d.renames,
        [Rename {
            from: "old/c.rs".into(),
            to: "new/c.rs".into()
        }]
    );
}

#[test]
fn merges_have_two_parents_and_diff_against_the_first() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    commit(&mut repo, "main", 0, &[Edit::write("a.rs", &text("a", 3))]);
    repo.branch("topic", "main").unwrap();
    let side = commit(&mut repo, "topic", 1, &[Edit::write("t.rs", &text("t", 7))]);
    commit(&mut repo, "main", 2, &[Edit::write("m.rs", &text("m", 4))]);
    let merge = repo.merge("main", "topic", at(3), "merge topic").unwrap();
    let history = read(&repo);
    assert_eq!(history.commits.len(), 4, "the topic commit is counted once");
    let m = facts(&history, &merge);
    assert_eq!((m.parents, m.added, m.removed), (2, 7, 0));
    assert!(m.on_default_first_parent);
    assert!(!facts(&history, &side).on_default_first_parent);
    assert_eq!(history.commits.last().map(|c| &c.id), Some(&merge));
}

#[test]
fn tags_peel_to_commits_with_their_dates() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let first = commit(&mut repo, "main", 0, &[Edit::write("a.rs", "a\n")]);
    let second = commit(&mut repo, "main", 5, &[Edit::write("a.rs", "b\n")]);
    repo.tag_annotated("v1.0.0", &first, at(30), "one").unwrap();
    repo.tag_lightweight("light", &second).unwrap();
    let inner = repo
        .tag_annotated("inner", &second, at(40), "inner")
        .unwrap();
    repo.tag_annotated("outer", &inner, at(50), "outer")
        .unwrap();
    repo.tag_annotated("early", &second, at(-100), "before its commit")
        .unwrap();
    let tree = repo.tree_of(&second).unwrap();
    repo.tag_lightweight("a-tree", &tree).unwrap();
    let history = read(&repo);
    let tag = |name: &str, hours: u32| TagFact {
        name: name.into(),
        tick: Tick::new(hours),
    };
    assert_eq!(facts(&history, &first).tags, [tag("v1.0.0", 30)]);
    assert_eq!(
        facts(&history, &second).tags,
        [
            tag("early", 5),
            tag("inner", 40),
            tag("light", 5),
            tag("outer", 50)
        ]
    );
    assert_eq!(history.state.tags.len(), 6);
}

#[test]
fn an_older_unrelated_root_moves_tick_zero_but_not_the_seed() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let root = commit(&mut repo, "main", 240, &[Edit::write("a.rs", "a\n")]);
    commit(&mut repo, "main", 241, &[Edit::write("a.rs", "b\n")]);
    let orphan = commit(&mut repo, "archive", 0, &[Edit::write("z.rs", "z\n")]);
    let history = read(&repo);
    assert_eq!(history.grid.origin_secs(), T0);
    assert_eq!(facts(&history, &orphan).tick, Tick::ZERO);
    assert_eq!(facts(&history, &root).tick, Tick::new(240));
    assert_eq!(facts(&history, &orphan).parents, 0);
    let identity = open(&repo).identity("main").unwrap();
    assert_eq!(identity.root, root);
    assert_eq!(identity.seed, RepoSeed::from_root_commit(&root));
    assert!(!identity.shallow);
}

#[test]
fn a_shallow_boundary_stands_in_for_the_root() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let ids: Vec<CommitId> = (0..5)
        .map(|n| {
            commit(
                &mut repo,
                "main",
                n,
                &[Edit::write("a.rs", &text("a", 2 + n))],
            )
        })
        .collect();
    repo.make_shallow(&ids[2..3]).unwrap();
    let history = read(&repo);
    assert_eq!(history.commits.len(), 3);
    assert_eq!(history.notes.shallow_boundary, ids[2..3]);
    assert!(history.notes.missing_objects.is_empty());
    let boundary = facts(&history, &ids[2]);
    assert_eq!(
        (boundary.added, boundary.parents, boundary.tick),
        (4, 1, Tick::ZERO)
    );
    let identity = open(&repo).identity("main").unwrap();
    assert_eq!(
        (identity.root.clone(), identity.shallow),
        (ids[2].clone(), true)
    );
}

#[test]
fn commit_times_are_clamped_to_their_parents() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let first = commit(&mut repo, "main", 100, &[Edit::write("a.rs", "a\n")]);
    let earlier = commit(&mut repo, "main", 10, &[Edit::write("a.rs", "b\n")]);
    let later = commit(&mut repo, "main", 120, &[Edit::write("a.rs", "c\n")]);
    let history = read(&repo);
    assert_eq!(facts(&history, &first).tick, Tick::ZERO);
    assert_eq!(facts(&history, &earlier).tick, Tick::ZERO);
    assert_eq!(facts(&history, &later).tick, Tick::new(20));
    let order: Vec<&CommitId> = history.commits.iter().map(|c| &c.id).collect();
    assert_eq!(order, [&first, &earlier, &later], "parents come first");
}

#[test]
fn dates_before_1970_are_read_as_1970() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let old = repo
        .commit(
            "main",
            Stamp::at(-400 * DAY),
            "ancient",
            &[Edit::write("a.rs", "a\n")],
        )
        .unwrap();
    let history = read(&repo);
    assert_eq!(history.grid.origin_secs(), 0);
    assert_eq!(facts(&history, &old).tick, Tick::ZERO);
}

#[test]
fn an_unparseable_date_takes_its_parents_time_or_1970() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    commit(&mut repo, "main", 10, &[Edit::write("a.rs", "a\n")]);
    let garbled = repo
        .commit_with_raw_time("main", "not-a-time", "garbled")
        .unwrap();
    let history = read(&repo);
    assert_eq!(history.grid.origin_secs(), T0 + 10 * HOUR);
    assert_eq!(
        facts(&history, &garbled).tick,
        Tick::ZERO,
        "the parent's time"
    );

    let mut orphan = FixtureRepo::init(&dir.path().join("orphan")).unwrap();
    let root = orphan
        .commit_with_raw_time("main", "never", "a root without a date")
        .unwrap();
    let child = commit(&mut orphan, "main", 3, &[Edit::write("b.rs", "b\n")]);
    let history = read(&orphan);
    assert_eq!(
        history.grid.origin_secs(),
        0,
        "a root without a date is 1970"
    );
    assert_eq!(facts(&history, &root).tick, Tick::ZERO);
    assert!(facts(&history, &child).tick > Tick::new(400_000));
}

#[test]
fn a_dangling_ref_is_skipped() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let tip = commit(&mut repo, "main", 0, &[Edit::write("a.rs", "a\n")]);
    let ghost = "ef".repeat(20);
    repo.write_git_file("refs/heads/pruned", format!("{ghost}\n").as_bytes())
        .unwrap();
    repo.write_git_file("refs/heads/garbled", b"not a ref\n")
        .unwrap();
    repo.write_git_file("refs/tags/gone", format!("{ghost}\n").as_bytes())
        .unwrap();
    let git = open(&repo);
    let state = git.state().unwrap();
    assert_eq!(
        state.branches,
        BTreeMap::from([("main".to_owned(), tip.clone())])
    );
    assert_eq!(git.default_branch(), Ok("main".into()));
    let history = read(&repo);
    assert_eq!(history.commits.len(), 1);
    assert!(history.commits[0].tags.is_empty());
}

#[test]
fn a_missing_parent_is_noted_and_still_counted() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    commit(&mut repo, "main", 0, &[Edit::write("a.rs", "a\n")]);
    let ghost = CommitId::from_hex(&"cd".repeat(20)).unwrap();
    let merge = repo
        .commit_with_parents(
            "main",
            at(1),
            "merge a ghost",
            &[Edit::write("b.rs", "b\n")],
            std::slice::from_ref(&ghost),
        )
        .unwrap();
    let history = read(&repo);
    let m = facts(&history, &merge);
    assert_eq!((m.parents, m.added), (2, 1));
    assert_eq!(history.notes.missing_objects, [ghost]);
}

#[test]
fn a_replace_ref_changes_nothing() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    let first = commit(&mut repo, "main", 0, &[Edit::write("a.rs", "a\n")]);
    commit(&mut repo, "main", 1, &[Edit::write("a.rs", "b\n")]);
    let before = read(&repo);
    let impostor = commit(
        &mut repo,
        "impostor",
        50,
        &[Edit::write("x.rs", &text("x", 9))],
    );
    repo.delete_branch("impostor").unwrap();
    repo.replace(&first, &impostor).unwrap();
    assert_eq!(read(&repo), before);
}

#[test]
fn progress_is_reported_and_a_cancel_stops_the_read() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    for n in 0..4 {
        commit(
            &mut repo,
            "main",
            n,
            &[Edit::write("a.rs", &text("a", 1 + n))],
        );
    }
    let mut seen = Vec::new();
    open(&repo)
        .read_history("main", &mut |p| {
            seen.push((p.diffed, p.total));
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(seen, [(0, 4), (1, 4), (2, 4), (3, 4), (4, 4)]);
    let mut calls = 0;
    let cancelled = open(&repo).read_history("main", &mut |_| {
        calls += 1;
        if calls > 2 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert_eq!(cancelled, Err(GitError::Cancelled));
}

#[test]
fn the_default_branch_follows_origin_head_then_main_master_and_head() {
    let dir = temp();
    let build = |name: &str, branches: &[&str], head: Option<&str>| {
        let mut repo = FixtureRepo::init(&dir.path().join(name)).unwrap();
        for (n, branch) in branches.iter().enumerate() {
            commit(
                &mut repo,
                branch,
                i64::try_from(n).unwrap(),
                &[Edit::write("a.rs", branch)],
            );
        }
        match head {
            Some(branch) => repo.set_head(branch).unwrap(),
            None => repo.detach_head(&repo.tip(branches[0])).unwrap(),
        }
        repo
    };
    let origin = build("origin", &["main", "dev"], Some("main"));
    origin.set_origin_head("dev").unwrap();
    assert_eq!(open(&origin).default_branch(), Ok("dev".into()));
    let main = build("main", &["dev", "master", "main"], Some("dev"));
    assert_eq!(open(&main).default_branch(), Ok("main".into()));
    let master = build("master", &["dev", "master"], Some("dev"));
    assert_eq!(open(&master).default_branch(), Ok("master".into()));
    let head = build("head", &["dev", "trunk"], Some("trunk"));
    assert_eq!(open(&head).default_branch(), Ok("trunk".into()));
    let detached = build("detached", &["trunk"], None);
    assert_eq!(
        open(&detached).default_branch(),
        Err(GitError::NoDefaultBranch)
    );
    let empty = FixtureRepo::init(&dir.path().join("empty")).unwrap();
    assert_eq!(open(&empty).default_branch(), Err(GitError::NoCommits));
    assert_eq!(
        open(&head)
            .read_history("main", &mut go)
            .map(|h| h.commits.len()),
        Err(GitError::BranchMissing("main".into()))
    );
    assert_eq!(
        open(&head).identity("gone"),
        Err(GitError::BranchMissing("gone".into()))
    );
    assert!(matches!(
        GitRepo::open(&dir.path().join("nowhere")),
        Err(GitError::Open(_))
    ));
}

#[test]
fn two_identical_builds_read_identically() {
    let dir = temp();
    let build = |name: &str| {
        let mut repo = FixtureRepo::init(&dir.path().join(name)).unwrap();
        commit(&mut repo, "main", 0, &[Edit::write("a.rs", &text("a", 5))]);
        repo.branch("side", "main").unwrap();
        commit(&mut repo, "side", 1, &[Edit::write("b.rs", &text("b", 5))]);
        commit(&mut repo, "main", 2, &[Edit::write("a.rs", &text("a", 8))]);
        repo.merge("main", "side", at(3), "merge").unwrap();
        repo
    };
    let (one, two) = (build("one"), build("two"));
    assert_eq!(read(&one), read(&two));
    assert_eq!(
        open(&one).state().unwrap().state_hash(),
        open(&two).state().unwrap().state_hash()
    );
}

// --- Incremental reads -------------------------------------------------------------------

fn base(dir: &TempDir) -> (FixtureRepo, Vec<CommitId>) {
    let mut repo = new_repo(dir);
    let ids = (0..3)
        .map(|n| {
            commit(
                &mut repo,
                "main",
                10 + n,
                &[Edit::write("a.rs", &text("a", 1 + n))],
            )
        })
        .collect();
    (repo, ids)
}

fn assert_full(repo: &FixtureRepo, update: &Update) {
    let full = open(repo)
        .read_history(&update.history.default_branch, &mut go)
        .unwrap();
    assert_eq!(update.history, full);
}

#[test]
fn an_unchanged_repository_reads_as_unchanged() {
    let dir = temp();
    let (repo, _) = base(&dir);
    let previous = read(&repo);
    let update = since(&repo, &previous);
    assert_eq!(update.change, Change::Unchanged);
    assert_eq!(update.history, previous);
}

#[test]
fn new_commits_are_appended() {
    let dir = temp();
    let (mut repo, _) = base(&dir);
    let previous = read(&repo);
    let a = commit(&mut repo, "main", 20, &[Edit::write("b.rs", "b\n")]);
    let b = commit(&mut repo, "main", 21, &[Edit::write("b.rs", "c\n")]);
    let update = since(&repo, &previous);
    assert_eq!(
        update.change,
        Change::Appended {
            ids: vec![a, b],
            earliest_tick: Tick::new(10)
        }
    );
    assert_full(&repo, &update);
}

#[test]
fn a_cancelled_update_leaves_the_history_as_it_was() {
    let dir = temp();
    let (mut repo, _) = base(&dir);
    let previous = read(&repo);
    commit(&mut repo, "main", 20, &[Edit::write("b.rs", "b\n")]);
    let mut history = previous.clone();
    let stopped = open(&repo).read_since(&mut history, "main", &mut |_| ControlFlow::Break(()));
    assert_eq!(stopped, Err(GitError::Cancelled));
    assert_eq!(history, previous);
}

#[test]
fn a_restored_old_branch_is_appended_before_the_tip() {
    let dir = temp();
    let (mut repo, ids) = base(&dir);
    repo.branch("old", "main").unwrap();
    repo.reset("old", &ids[0]).unwrap();
    let early = commit(&mut repo, "old", 11, &[Edit::write("o.rs", "o\n")]);
    repo.delete_branch("old").unwrap();
    let previous = read(&repo);
    repo.reset("old", &early).unwrap();
    let update = since(&repo, &previous);
    assert_eq!(
        update.change,
        Change::Appended {
            ids: vec![early],
            earliest_tick: Tick::new(1)
        }
    );
    assert_full(&repo, &update);
}

/// A change to a repository, and the reason it forces a full read.
type Case = (
    &'static str,
    fn(&mut FixtureRepo, &[CommitId]),
    RewriteReason,
);

#[test]
fn history_that_changed_is_read_again() {
    let dir = temp();
    let cases: [Case; 6] = [
        (
            "amend",
            |repo, ids| {
                repo.reset("main", &ids[1]).unwrap();
                commit(repo, "main", 30, &[Edit::write("a.rs", "amended\n")]);
            },
            RewriteReason::CommitsDropped,
        ),
        (
            "unmerged branch deleted",
            |repo, _| repo.delete_branch("side").unwrap(),
            RewriteReason::CommitsDropped,
        ),
        (
            "older root",
            |repo, _| {
                commit(repo, "archive", 0, &[Edit::write("z.rs", "z\n")]);
            },
            RewriteReason::GridMoved,
        ),
        (
            "new tag",
            |repo, ids| repo.tag_lightweight("v9", &ids[0]).unwrap(),
            RewriteReason::TagsChanged,
        ),
        (
            "deeper shallow boundary",
            |repo, ids| {
                repo.write_git_file("shallow", format!("{}\n", ids[0].as_str()).as_bytes())
                    .unwrap();
            },
            RewriteReason::ShallowChanged,
        ),
        (
            "first-parent chain moved",
            |repo, _| {
                commit(repo, "side", 40, &[Edit::write("s.rs", "more\n")]);
                let merged = repo.merge("side", "main", at(41), "merge main").unwrap();
                repo.reset("main", &merged).unwrap();
            },
            RewriteReason::FirstParentChanged,
        ),
    ];
    for (name, change, reason) in cases {
        let case_dir = TempDir::new_in(dir.path()).unwrap();
        let (mut repo, ids) = base(&case_dir);
        repo.branch("side", "main").unwrap();
        repo.reset("side", &ids[0]).unwrap();
        commit(&mut repo, "side", 12, &[Edit::write("s.rs", "s\n")]);
        repo.write_git_file("shallow", format!("{}\n", ids[1].as_str()).as_bytes())
            .unwrap();
        let previous = read(&repo);
        change(&mut repo, &ids);
        let update = since(&repo, &previous);
        assert_eq!(update.change, Change::Rewritten(reason), "{name}");
        assert_full(&repo, &update);
    }
}

#[test]
fn another_default_branch_reads_again() {
    let dir = temp();
    let (mut repo, _) = base(&dir);
    repo.branch("trunk", "main").unwrap();
    let previous = read(&repo);
    let update = since_on(&repo, &previous, "trunk");
    assert_eq!(
        update.change,
        Change::Rewritten(RewriteReason::DefaultBranchChanged)
    );
    assert_eq!(update.history.default_branch, "trunk");
    assert_full(&repo, &update);
}

#[derive(Clone, Debug)]
enum Op {
    Commit(usize),
    Branch(usize),
    Merge(usize, usize),
    Tag(usize),
    Rewind(usize),
    Delete(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..3_usize).prop_map(Op::Commit),
        1 => (0..3_usize).prop_map(Op::Branch),
        1 => (0..3_usize, 0..3_usize).prop_map(|(a, b)| Op::Merge(a, b)),
        1 => (0..3_usize).prop_map(Op::Tag),
        1 => (0..3_usize).prop_map(Op::Rewind),
        1 => (1..3_usize).prop_map(Op::Delete),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12,
        rng_seed: proptest::test_runner::RngSeed::Fixed(0x6769_7468_6973),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn chained_incremental_reads_equal_a_full_read(ops in prop::collection::vec(op(), 1..7)) {
        let dir = temp();
        let mut repo = new_repo(&dir);
        let names = ["main", "b1", "b2"];
        let mut live = vec!["main"];
        commit(&mut repo, "main", 0, &[Edit::write("a.rs", "0\n")]);
        let mut previous = read(&repo);
        for (step, op) in ops.into_iter().enumerate() {
            let hours = 10 * i64::try_from(step + 1).unwrap();
            let pick = |i: usize| live[i % live.len()];
            match op {
                Op::Commit(i) => {
                    let branch = pick(i);
                    commit(&mut repo, branch, hours, &[Edit::write(&format!("{branch}.rs"), &hours.to_string())]);
                }
                Op::Branch(i) => {
                    let name = names[1 + i % 2];
                    if !live.contains(&name) {
                        repo.branch(name, pick(i)).unwrap();
                        live.push(name);
                    }
                }
                Op::Merge(a, b) => {
                    let (into, from) = (pick(a), pick(b));
                    if into != from {
                        repo.merge(into, from, at(hours), "merge").unwrap();
                    }
                }
                Op::Tag(i) => {
                    let tip = repo.tip(pick(i));
                    repo.tag_lightweight(&format!("t{step}"), &tip).unwrap();
                }
                Op::Rewind(i) => {
                    let branch = pick(i);
                    let tip = repo.tip(branch);
                    let parent = previous.commits.iter().rev().find(|c| c.id != tip && c.tick.hours() > 0);
                    if let Some(parent) = parent.map(|c| c.id.clone()) {
                        repo.reset(branch, &parent).unwrap();
                    }
                }
                Op::Delete(i) => {
                    let name = names[i];
                    if let Some(at) = live.iter().position(|n| *n == name) {
                        repo.delete_branch(name).unwrap();
                        live.remove(at);
                    }
                }
            }
            let update = since(&repo, &previous);
            let full = read(&repo);
            prop_assert_eq!(&update.history, &full);
            if let Change::Appended { ids, earliest_tick } = &update.change {
                let old: BTreeMap<_, _> = previous.commits.iter().map(|c| (&c.id, c)).collect();
                for c in &full.commits {
                    match old.get(&c.id) {
                        Some(before) => prop_assert_eq!(*before, c),
                        None => prop_assert!(ids.contains(&c.id)),
                    }
                }
                prop_assert_eq!(ids.len(), full.commits.len() - previous.commits.len());
                let earliest = full.commits.iter().filter(|c| ids.contains(&c.id)).map(|c| c.tick).min();
                prop_assert_eq!(Some(*earliest_tick), earliest);
            }
            previous = update.history;
        }
    }
}

// --- Safety --------------------------------------------------------------------------------

/// Every file under `root`, by relative path, with the SHA-256 of its bytes.
fn snapshot(root: &Path) -> BTreeMap<String, [u8; 32]> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, [u8; 32]>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(
                    relative,
                    Sha256::digest(std::fs::read(&path).unwrap()).into(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

/// Opens, identifies and reads `repo` every way the adapter can.
fn read_every_way(repo: &FixtureRepo) -> History {
    let git = open(repo);
    let branch = git.default_branch().unwrap();
    git.identity(&branch).unwrap();
    git.state().unwrap();
    let history = git.read_history(&branch, &mut go).unwrap();
    let mut again = history.clone();
    git.read_since(&mut again, &branch, &mut go).unwrap();
    history
}

fn sentinel_script(dir: &Path, sentinel: &Path) -> std::path::PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let script = dir.join("sentinel.sh");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho ran >> '{}'\ncat\n", sentinel.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }
    #[cfg(not(unix))]
    {
        let script = dir.join("sentinel.cmd");
        std::fs::write(
            &script,
            format!("@echo ran >> \"{}\"\r\n", sentinel.display()),
        )
        .unwrap();
        script
    }
}

#[test]
fn nothing_the_repository_configures_ever_runs() {
    let dir = temp();
    let sentinel = dir.path().join("sentinel");
    let script = sentinel_script(dir.path(), &sentinel);
    let script = script.display().to_string().replace('\\', "/");
    let mut repo = new_repo(&dir);
    repo.append_git_config(&format!(
        "[core]\n\tfsmonitor = \"{script}\"\n\thooksPath = \"{dir}\"\n\
         [diff]\n\texternal = \"{script}\"\n\
         [diff \"evil\"]\n\ttextconv = \"{script}\"\n\tcommand = \"{script}\"\n\tbinary = false\n\
         [filter \"evil\"]\n\tclean = \"{script}\"\n\tsmudge = \"{script}\"\n\
         \tprocess = \"{script}\"\n\trequired = true\n",
        dir = dir.path().display().to_string().replace('\\', "/"),
    ))
    .unwrap();
    repo.write_git_file("info/attributes", b"* diff=evil filter=evil\n")
        .unwrap();
    commit(
        &mut repo,
        "main",
        0,
        &[
            Edit::write(".gitattributes", "* diff=evil filter=evil\n"),
            Edit::write("src/a.rs", &text("a", 30)),
            Edit::write("src/b.rs", &text("b", 30)),
        ],
    );
    commit(
        &mut repo,
        "main",
        1,
        &[
            Edit::write("src/a.rs", &text("a", 35)),
            Edit::Remove("src/b.rs".into()),
            Edit::write("src/c.rs", &text("b", 31)),
        ],
    );
    let history = read_every_way(&repo);
    commit(
        &mut repo,
        "main",
        2,
        &[Edit::write("src/a.rs", &text("z", 5))],
    );
    let update = since(&repo, &history);
    assert!(matches!(update.change, Change::Appended { .. }));
    assert_eq!(
        history.commits[1].renames.len(),
        1,
        "the rename tracker ran"
    );
    assert_eq!(history.commits[1].added, 6, "blob diffs ran");
    assert!(!sentinel.exists(), "a configured program ran");
}

fn configured_pair(dir: &TempDir) -> (FixtureRepo, FixtureRepo) {
    let build = |name: &str| {
        let mut repo = FixtureRepo::init(&dir.path().join(name)).unwrap();
        commit(
            &mut repo,
            "main",
            0,
            &[
                Edit::write(".gitattributes", "*.rs binary\n*.txt -diff\n"),
                Edit::write("src/a.rs", &text("a", 40)),
                Edit::write("notes.txt", &text("n", 10)),
            ],
        );
        commit(
            &mut repo,
            "main",
            1,
            &[
                Edit::Remove("src/a.rs".into()),
                Edit::write("src/moved.rs", &text("a", 40).replace("a 1\n", "b\n")),
                Edit::write("notes.txt", &text("m", 12)),
            ],
        );
        repo
    };
    let (plain, configured) = (build("plain"), build("configured"));
    configured
        .append_git_config(
            "[diff]\n\talgorithm = minimal\n\trenames = false\n\trenameLimit = 1\n\
             [core]\n\tbigFileThreshold = 1\n\tignoreCase = true\n",
        )
        .unwrap();
    configured
        .write_git_file("info/attributes", b"* -diff\n* binary\n")
        .unwrap();
    (plain, configured)
}

#[test]
fn repository_config_and_attributes_never_change_the_facts() {
    let dir = temp();
    let (plain, configured) = configured_pair(&dir);
    let history = read(&plain);
    assert_eq!(read(&configured), history);
    // 40 + 10, and the two lines of `.gitattributes` itself.
    assert_eq!(
        history.commits[0].added, 52,
        "attributes never make text binary"
    );
    assert_eq!(history.commits[1].renames.len(), 1);
    assert_eq!(
        (history.commits[1].added, history.commits[1].removed),
        (13, 11)
    );
}

#[test]
fn reading_never_writes_into_the_repository() {
    let dir = temp();
    let (_, mut repo) = configured_pair(&dir);
    repo.branch("side", "main").unwrap();
    commit(&mut repo, "side", 2, &[Edit::write("s.rs", "s\n")]);
    repo.tag_annotated("v1", &repo.tip("main"), at(3), "v1")
        .unwrap();
    let before = snapshot(repo.path());
    read_every_way(&repo);
    assert_eq!(snapshot(repo.path()), before);
}

// --- Generated histories ---------------------------------------------------------------------

#[test]
fn the_arc_reads_as_scripted() {
    let dir = temp();
    let arc = fixture::arc(&dir.path().join("arc")).unwrap();
    let git = GitRepo::open(&dir.path().join("arc")).unwrap();
    assert_eq!(git.default_branch(), Ok("main".into()));
    let history = git.read_history("main", &mut go).unwrap();
    assert_eq!(history.commits.len(), arc.commits.len());
    // Pinned: golden replays and the committed observations are built on this script.
    assert_eq!(history.commits.len(), 604);
    let mut ids: Vec<&CommitId> = arc.commits.iter().map(|c| &c.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        arc.commits.len(),
        "every scripted commit is distinct"
    );
    let branches: Vec<&str> = history.state.branches.keys().map(String::as_str).collect();
    assert_eq!(branches, fixture::ARC_BRANCHES);
    // Each branch's commits are in time order, so nothing is clamped and the read order is
    // time order.
    let read_order: Vec<&CommitId> = history.commits.iter().map(|c| &c.id).collect();
    let mut by_time: Vec<&fixture::ArcCommit> = arc.commits.iter().collect();
    by_time.sort_by_key(|c| (c.secs, c.id.clone()));
    let script_order: Vec<&CommitId> = by_time.iter().map(|c| &c.id).collect();
    assert_eq!(read_order, script_order);
    assert_eq!(
        history.grid.origin_secs(),
        fixture::ARC_START_SECS + 9 * HOUR
    );

    let merge = facts(&history, &arc.merge);
    assert_eq!(merge.parents, 2);
    assert!(merge.added > 0);
    let rename = facts(&history, &arc.rename);
    assert_eq!(rename.renames.len(), 3);
    assert!(
        rename
            .renames
            .iter()
            .all(|r| r.from.starts_with(fixture::ARC_RENAMED_FROM)
                && r.to.starts_with(fixture::ARC_RENAMED_TO))
    );
    assert!(rename.deleted_dirs.is_empty());
    let deletion = facts(&history, &arc.deletion);
    assert_eq!(deletion.deleted_dirs.len(), 1);
    assert_eq!(deletion.deleted_dirs[0].path, fixture::ARC_DELETED_DIR);
    assert_eq!(deletion.deleted_dirs[0].lines, 660);
    for name in fixture::ARC_TAGS {
        let tagged = facts(&history, &arc.tags[name]);
        assert!(tagged.tags.iter().any(|t| t.name == *name), "{name}");
    }
    // Excluded and binary files never count: the first commit holds 12 + 40 + 30 + 75 + 8 +
    // 660 lines of text, plus the LFS pointer's 3.
    assert_eq!(history.commits[0].added, 828);
    let identity = git.identity("main").unwrap();
    assert_eq!(identity.root, arc.commits[0].id);
    let silent = history
        .commits
        .windows(2)
        .map(|w| w[1].tick.hours_since(w[0].tick))
        .max()
        .unwrap();
    assert!(silent > 24 * 120, "the dead phase is a long silence");
}

#[test]
fn the_synthetic_history_reads() {
    let dir = temp();
    let repo = fixture::synthetic(&dir.path().join("synthetic"), 40).unwrap();
    let history = read(&repo);
    assert_eq!(history.commits.len(), 40);
    assert_eq!(history.commits[0].added, 3_000 * 20 + 3 * 2_500);
}

#[test]
fn head_says_where_head_is_and_whether_it_is_the_default_tip() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    assert_eq!(open(&repo).head("main"), Err(GitError::NoCommits));
    let first = repo
        .commit("main", at(0), "seed", &[Edit::write("a.txt", "a\n")])
        .unwrap();
    let head = open(&repo).head("main").unwrap();
    assert_eq!(head.commit, first);
    assert_eq!(head.branch.as_deref(), Some("main"));
    assert!(head.at_default_tip);

    // A branch at the same commit is at the default tip; once it moves on, it is not.
    repo.branch("feature/moss", "main").unwrap();
    repo.set_head("feature/moss").unwrap();
    let head = open(&repo).head("main").unwrap();
    assert_eq!(head.branch.as_deref(), Some("feature/moss"));
    assert!(head.at_default_tip);
    let moved = repo
        .commit(
            "feature/moss",
            at(1),
            "moss",
            &[Edit::write("b.txt", "b\n")],
        )
        .unwrap();
    let head = open(&repo).head("main").unwrap();
    assert_eq!(head.commit, moved);
    assert!(!head.at_default_tip);

    // Detached at the default tip: no branch, still at the tip.
    repo.detach_head(&first).unwrap();
    let head = open(&repo).head("main").unwrap();
    assert_eq!(head.branch, None);
    assert_eq!(head.commit, first);
    assert!(head.at_default_tip);
    assert!(!open(&repo).head("trunk").unwrap().at_default_tip);
}

#[test]
fn discover_finds_the_repository_above_a_directory() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    repo.commit("main", at(0), "seed", &[Edit::write("a.txt", "a\n")])
        .unwrap();
    let nested = repo.path().join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();
    let found = GitRepo::discover(&nested).unwrap();
    assert_eq!(
        found.identity("main").unwrap(),
        open(&repo).identity("main").unwrap()
    );
    assert!(matches!(
        GitRepo::discover(&dir.path().join("nowhere")),
        Err(GitError::Open(_))
    ));
}

#[test]
fn ancestry_follows_every_walked_parent() {
    let dir = temp();
    let (mut repo, ids) = base(&dir);
    repo.branch("side", "main").unwrap();
    repo.reset("side", &ids[0]).unwrap();
    let side = commit(&mut repo, "side", 12, &[Edit::write("s.rs", "s\n")]);
    let merged = repo.merge("main", "side", at(13), "merge side").unwrap();
    let history = read(&repo);
    // Through the first parent, through the second, and a commit is its own ancestor.
    assert!(history.is_ancestor(&ids[0], &merged));
    assert!(history.is_ancestor(&side, &merged));
    assert!(history.is_ancestor(&merged, &merged));
    // Never backwards, never across unrelated lines.
    assert!(!history.is_ancestor(&merged, &ids[0]));
    assert!(!history.is_ancestor(&side, &ids[2]));
    // A commit the history does not hold is nobody's ancestor.
    repo.reset("main", &ids[1]).unwrap();
    repo.delete_branch("side").unwrap();
    let rewound = read(&repo);
    assert!(!rewound.is_ancestor(&merged, &ids[1]));
    assert!(!rewound.is_ancestor(&ids[0], &merged));
}

#[test]
fn the_git_directories_are_where_refs_and_head_live() {
    let dir = temp();
    let mut repo = new_repo(&dir);
    repo.commit("main", at(0), "seed", &[Edit::write("a.txt", "a\n")])
        .unwrap();
    let git = open(&repo);
    let git_dir = std::fs::canonicalize(git.git_dir()).unwrap();
    assert_eq!(git_dir, std::fs::canonicalize(repo.git_dir()).unwrap());
    assert!(git_dir.join("HEAD").is_file());
    assert_eq!(
        std::fs::canonicalize(git.common_dir()).unwrap(),
        git_dir,
        "a repository without linked worktrees keeps its refs beside HEAD"
    );
    assert_eq!(
        std::fs::canonicalize(git.workdir().unwrap()).unwrap(),
        std::fs::canonicalize(repo.path()).unwrap()
    );
}
