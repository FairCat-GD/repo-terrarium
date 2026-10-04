//! Deterministic git repositories for tests, benches and `cargo xtask fixture`.
//!
//! Everything here writes, so it is compiled only for this crate's tests and behind the
//! `fixture` feature, which only xtask and dev-dependencies may turn on. Repositories are built
//! with gix in a directory the caller owns, with fictional authors, fixed times and no reflogs,
//! so the same script always produces the same object ids. Nothing is read from the user's git
//! configuration.

mod arc;
mod synthetic;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gix::bstr::{BString, ByteSlice};
use gix::objs::tree::EntryKind;
use gix::refs::transaction::{Change as RefChange, LogChange, PreviousValue, RefEdit, RefLog};
use terrarium_truth::CommitId;

pub use arc::{
    ARC_BRANCHES, ARC_DELETED_DIR, ARC_END_SECS, ARC_RENAMED_FROM, ARC_RENAMED_TO, ARC_START_SECS,
    ARC_TAGS, Arc, ArcCommit, Phase, arc,
};
pub use synthetic::synthetic;

pub type FixtureError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type FixtureResult<T> = Result<T, FixtureError>;

/// Fictional people who write fixture commits.
pub const AUTHORS: &[(&str, &str)] = &[
    ("Wren Alder", "wren.alder@example.invalid"),
    ("Tamsin Reed", "tamsin.reed@example.invalid"),
    ("Odile Fern", "odile.fern@example.invalid"),
    ("Baz Thornby", "baz.thornby@example.invalid"),
];

/// When a commit or tag was made, and by whom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    /// UTC seconds.
    pub secs: i64,
    /// The time zone offset written into the object, in seconds east of UTC.
    pub offset: i32,
    /// An index into [`AUTHORS`].
    pub author: usize,
}

impl Stamp {
    pub const fn at(secs: i64) -> Stamp {
        Stamp {
            secs,
            offset: 0,
            author: 0,
        }
    }

    fn signature(self) -> gix::actor::Signature {
        let (name, email) = AUTHORS[self.author % AUTHORS.len()];
        gix::actor::Signature {
            name: name.into(),
            email: email.into(),
            time: gix::date::Time::new(self.secs, self.offset),
        }
    }
}

/// One change to a branch's files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Write(String, Vec<u8>),
    Executable(String, Vec<u8>),
    Remove(String),
    Symlink(String, String),
    /// A submodule entry pointing at a commit of another repository.
    Gitlink(String, CommitId),
}

impl Edit {
    pub fn write(path: &str, text: &str) -> Edit {
        Edit::Write(path.to_owned(), text.as_bytes().to_vec())
    }
}

/// A directory of a tree being built, remembering its object id until something below it
/// changes, so a commit rewrites only the trees it touched.
#[derive(Clone, Debug, Default)]
struct Dir {
    entries: BTreeMap<BString, Node>,
    id: Option<gix::ObjectId>,
}

#[derive(Clone, Debug)]
enum Node {
    File(EntryKind, gix::ObjectId),
    Dir(Dir),
}

impl Dir {
    fn upsert(&mut self, path: &[u8], kind: EntryKind, id: gix::ObjectId) {
        self.id = None;
        match path.split_once_str(b"/") {
            None => {
                self.entries.insert(path.into(), Node::File(kind, id));
            }
            Some((head, rest)) => {
                let node = self
                    .entries
                    .entry(head.into())
                    .or_insert_with(|| Node::Dir(Dir::default()));
                if let Node::File(..) = node {
                    *node = Node::Dir(Dir::default());
                }
                if let Node::Dir(dir) = node {
                    dir.upsert(rest, kind, id);
                }
            }
        }
    }

    fn remove(&mut self, path: &[u8]) {
        self.id = None;
        match path.split_once_str(b"/") {
            None => {
                self.entries.remove(path.as_bstr());
            }
            Some((head, rest)) => {
                if let Some(Node::Dir(dir)) = self.entries.get_mut(head.as_bstr()) {
                    dir.remove(rest);
                    if dir.entries.is_empty() {
                        self.entries.remove(head.as_bstr());
                    }
                }
            }
        }
    }

    fn write(&mut self, repo: &gix::Repository) -> FixtureResult<gix::ObjectId> {
        if let Some(id) = self.id {
            return Ok(id);
        }
        let mut entries = Vec::with_capacity(self.entries.len());
        for (name, node) in &mut self.entries {
            let (mode, oid) = match node {
                Node::File(kind, id) => ((*kind).into(), *id),
                Node::Dir(dir) => (EntryKind::Tree.into(), dir.write(repo)?),
            };
            entries.push(gix::objs::tree::Entry {
                mode,
                filename: name.clone(),
                oid,
            });
        }
        entries.sort();
        let id = repo.write_object(&gix::objs::Tree { entries })?.detach();
        self.id = Some(id);
        Ok(id)
    }

    fn flatten(&self, prefix: &[u8], out: &mut BTreeMap<BString, (EntryKind, gix::ObjectId)>) {
        for (name, node) in &self.entries {
            let mut path = prefix.to_vec();
            if !path.is_empty() {
                path.push(b'/');
            }
            path.extend_from_slice(name);
            match node {
                Node::File(kind, id) => {
                    out.insert(path.into(), (*kind, *id));
                }
                Node::Dir(dir) => dir.flatten(&path, out),
            }
        }
    }

    fn load(repo: &gix::Repository, tree: gix::ObjectId) -> FixtureResult<Dir> {
        let mut dir = Dir {
            entries: BTreeMap::new(),
            id: Some(tree),
        };
        let tree = repo.find_tree(tree)?;
        for entry in tree.decode()?.entries {
            let node = if entry.mode.is_tree() {
                Node::Dir(Dir::load(repo, entry.oid.to_owned())?)
            } else {
                Node::File(entry.mode.kind(), entry.oid.to_owned())
            };
            dir.entries.insert(entry.filename.to_owned(), node);
        }
        Ok(dir)
    }
}

#[derive(Clone, Debug, Default)]
struct Branch {
    tip: Option<gix::ObjectId>,
    files: Dir,
}

/// A repository being built.
pub struct FixtureRepo {
    repo: gix::Repository,
    path: PathBuf,
    branches: BTreeMap<String, Branch>,
}

fn object_id(id: &CommitId) -> gix::ObjectId {
    gix::ObjectId::from_hex(id.as_str().as_bytes()).expect("a commit id is lowercase hex")
}

fn commit_id(id: gix::ObjectId) -> CommitId {
    CommitId::from_hex(&id.to_hex().to_string()).expect("gix prints lowercase hex")
}

impl FixtureRepo {
    /// An empty repository with a worktree at `path`, whose `HEAD` names `main`. The worktree
    /// stays empty: readers need only the object database.
    pub fn init(path: &Path) -> FixtureResult<FixtureRepo> {
        let capabilities = gix::fs::Capabilities {
            precompose_unicode: false,
            ignore_case: false,
            executable_bit: true,
            symlink: true,
        };
        let mut repo = gix::ThreadSafeRepository::init_opts(
            path,
            gix::create::Kind::WithWorktree,
            gix::create::Options {
                fs_capabilities: Some(capabilities),
                ..gix::create::Options::default()
            },
            gix::open::Options::isolated(),
        )?
        .to_thread_local();
        repo.refs.write_reflog = gix::refs::store::WriteReflog::Disable;
        let fixture = FixtureRepo {
            repo,
            path: path.to_owned(),
            branches: BTreeMap::new(),
        };
        fixture.set_head("main")?;
        Ok(fixture)
    }

    /// The repository's worktree directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// The commit at the tip of `branch`.
    ///
    /// # Panics
    ///
    /// When the branch has no commit.
    pub fn tip(&self, branch: &str) -> CommitId {
        let tip = self.branches.get(branch).and_then(|b| b.tip);
        commit_id(tip.unwrap_or_else(|| panic!("branch {branch} has no commit")))
    }

    fn write_ref(&self, name: &str, target: gix::refs::Target) -> FixtureResult<()> {
        self.repo.edit_reference(RefEdit {
            change: RefChange::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: false,
                    message: BString::default(),
                },
                expected: PreviousValue::Any,
                new: target,
            },
            name: name.try_into()?,
            deref: false,
        })?;
        Ok(())
    }

    fn write_ref_to(&self, name: &str, id: gix::ObjectId) -> FixtureResult<()> {
        self.write_ref(name, gix::refs::Target::Object(id))
    }

    /// Writes a commit on `branch` with `edits` applied to its files. A branch with no commit
    /// yet starts from an empty tree, so its first commit is a root.
    pub fn commit(
        &mut self,
        branch: &str,
        stamp: Stamp,
        message: &str,
        edits: &[Edit],
    ) -> FixtureResult<CommitId> {
        self.commit_with_parents(branch, stamp, message, edits, &[])
    }

    /// [`FixtureRepo::commit`] with `extra` parents after the branch's tip, which need not
    /// exist in this repository.
    pub fn commit_with_parents(
        &mut self,
        branch: &str,
        stamp: Stamp,
        message: &str,
        edits: &[Edit],
        extra: &[CommitId],
    ) -> FixtureResult<CommitId> {
        let mut state = self.branches.remove(branch).unwrap_or_default();
        let result = self.apply_and_commit(&mut state, stamp, message, edits, extra);
        if let Ok(id) = &result {
            state.tip = Some(object_id(id));
        }
        self.branches.insert(branch.to_owned(), state);
        let id = result?;
        self.write_ref_to(&format!("refs/heads/{branch}"), object_id(&id))?;
        Ok(id)
    }

    fn apply_and_commit(
        &self,
        state: &mut Branch,
        stamp: Stamp,
        message: &str,
        edits: &[Edit],
        extra: &[CommitId],
    ) -> FixtureResult<CommitId> {
        for edit in edits {
            match edit {
                Edit::Write(path, bytes) => {
                    let id = self.repo.write_blob(bytes)?.detach();
                    state.files.upsert(path.as_bytes(), EntryKind::Blob, id);
                }
                Edit::Executable(path, bytes) => {
                    let id = self.repo.write_blob(bytes)?.detach();
                    state
                        .files
                        .upsert(path.as_bytes(), EntryKind::BlobExecutable, id);
                }
                Edit::Remove(path) => state.files.remove(path.as_bytes()),
                Edit::Symlink(path, target) => {
                    let id = self.repo.write_blob(target.as_bytes())?.detach();
                    state.files.upsert(path.as_bytes(), EntryKind::Link, id);
                }
                Edit::Gitlink(path, commit) => {
                    state
                        .files
                        .upsert(path.as_bytes(), EntryKind::Commit, object_id(commit));
                }
            }
        }
        let tree = state.files.write(&self.repo)?;
        let mut parents: Vec<gix::ObjectId> = state.tip.into_iter().collect();
        parents.extend(extra.iter().map(object_id));
        let signature = stamp.signature();
        let commit = gix::objs::Commit {
            tree,
            parents: parents.into_iter().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: format!("{message}\n").into(),
            extra_headers: Vec::new(),
        };
        Ok(commit_id(self.repo.write_object(&commit)?.detach()))
    }

    /// Writes a commit on `branch`, with its files unchanged, whose author and committer lines
    /// end in `time` exactly as given, which need not be a valid git date.
    pub fn commit_with_raw_time(
        &mut self,
        branch: &str,
        time: &str,
        message: &str,
    ) -> FixtureResult<CommitId> {
        use gix::objs::Write as _;
        let mut state = self.branches.remove(branch).unwrap_or_default();
        let tree = state.files.write(&self.repo);
        let result = tree.and_then(|tree| {
            let parent = state
                .tip
                .map(|tip| format!("parent {}\n", tip.to_hex()))
                .unwrap_or_default();
            let (name, email) = AUTHORS[0];
            let text = format!(
                "tree {}\n{parent}author {name} <{email}> {time}\n\
                 committer {name} <{email}> {time}\n\n{message}\n",
                tree.to_hex()
            );
            self.repo
                .objects
                .write_buf(gix::objs::Kind::Commit, text.as_bytes())
                .map_err(|err| FixtureError::from(err.to_string()))
        });
        if let Ok(id) = &result {
            state.tip = Some(*id);
        }
        self.branches.insert(branch.to_owned(), state);
        let id = result?;
        self.write_ref_to(&format!("refs/heads/{branch}"), id)?;
        Ok(commit_id(id))
    }

    /// Merges `from` into `into` with a merge commit: every file of `from` that `into` lacks or
    /// holds differently is taken from `from`.
    pub fn merge(
        &mut self,
        into: &str,
        from: &str,
        stamp: Stamp,
        message: &str,
    ) -> FixtureResult<CommitId> {
        let source = self
            .branches
            .get(from)
            .ok_or_else(|| format!("no branch {from}"))?;
        let mut theirs = BTreeMap::new();
        source.files.flatten(b"", &mut theirs);
        let their_tip = source.tip.ok_or("merging a branch without commits")?;
        let mut ours = BTreeMap::new();
        if let Some(target) = self.branches.get(into) {
            target.files.flatten(b"", &mut ours);
        }
        let state = self.branches.entry(into.to_owned()).or_default();
        for (path, entry) in theirs {
            if ours.get(&path) != Some(&entry) {
                state.files.upsert(&path, entry.0, entry.1);
            }
        }
        self.commit_with_parents(into, stamp, message, &[], &[commit_id(their_tip)])
    }

    /// Starts `name` at the tip of `from`.
    pub fn branch(&mut self, name: &str, from: &str) -> FixtureResult<()> {
        let state = self
            .branches
            .get(from)
            .cloned()
            .ok_or_else(|| format!("no branch {from}"))?;
        let tip = state.tip.ok_or("branching from a branch without commits")?;
        self.branches.insert(name.to_owned(), state);
        self.write_ref_to(&format!("refs/heads/{name}"), tip)
    }

    /// Moves `branch` to `to`, as `git reset --hard` would, even backwards.
    pub fn reset(&mut self, branch: &str, to: &CommitId) -> FixtureResult<()> {
        let id = object_id(to);
        let tree = self.repo.find_commit(id)?.tree_id()?.detach();
        let files = Dir::load(&self.repo, tree)?;
        self.branches.insert(
            branch.to_owned(),
            Branch {
                tip: Some(id),
                files,
            },
        );
        self.write_ref_to(&format!("refs/heads/{branch}"), id)
    }

    pub fn delete_branch(&mut self, name: &str) -> FixtureResult<()> {
        self.branches.remove(name);
        self.repo.edit_reference(RefEdit {
            change: RefChange::Delete {
                expected: PreviousValue::Any,
                log: RefLog::AndReference,
            },
            name: format!("refs/heads/{name}").try_into()?,
            deref: false,
        })?;
        Ok(())
    }

    /// An annotated tag on any object, returning the tag object's id.
    pub fn tag_annotated(
        &mut self,
        name: &str,
        target: &CommitId,
        stamp: Stamp,
        message: &str,
    ) -> FixtureResult<CommitId> {
        let target = object_id(target);
        let kind = self.repo.find_header(target)?.kind();
        let tag = gix::objs::Tag {
            target,
            target_kind: kind,
            name: name.into(),
            tagger: Some(stamp.signature()),
            message: format!("{message}\n").into(),
            signature: None,
        };
        let id = self.repo.write_object(&tag)?.detach();
        self.write_ref_to(&format!("refs/tags/{name}"), id)?;
        Ok(commit_id(id))
    }

    /// A lightweight tag: a reference straight to `target`.
    pub fn tag_lightweight(&mut self, name: &str, target: &CommitId) -> FixtureResult<()> {
        self.write_ref_to(&format!("refs/tags/{name}"), object_id(target))
    }

    /// Points `HEAD` at `branch`, which need not exist yet.
    pub fn set_head(&self, branch: &str) -> FixtureResult<()> {
        self.write_ref(
            "HEAD",
            gix::refs::Target::Symbolic(format!("refs/heads/{branch}").try_into()?),
        )
    }

    /// Detaches `HEAD` at `commit`.
    pub fn detach_head(&self, commit: &CommitId) -> FixtureResult<()> {
        self.write_ref_to("HEAD", object_id(commit))
    }

    /// Writes `refs/remotes/origin/<branch>` at the branch's tip, and `refs/remotes/origin/HEAD`
    /// pointing at it, as a clone would.
    pub fn set_origin_head(&self, branch: &str) -> FixtureResult<()> {
        let remote = format!("refs/remotes/origin/{branch}");
        self.write_ref_to(&remote, object_id(&self.tip(branch)))?;
        self.write_ref(
            "refs/remotes/origin/HEAD",
            gix::refs::Target::Symbolic(remote.as_str().try_into()?),
        )
    }

    /// A replace ref, as `git replace <original> <replacement>` writes.
    pub fn replace(&self, original: &CommitId, replacement: &CommitId) -> FixtureResult<()> {
        self.write_ref_to(
            &format!("refs/replace/{}", original.as_str()),
            object_id(replacement),
        )
    }

    /// The tree of `commit`.
    pub fn tree_of(&self, commit: &CommitId) -> FixtureResult<CommitId> {
        Ok(commit_id(
            self.repo
                .find_commit(object_id(commit))?
                .tree_id()?
                .detach(),
        ))
    }

    /// Makes the repository a shallow clone cut at `boundary`: writes `.git/shallow` and
    /// deletes every commit object behind it, as a shallow fetch would never have them.
    pub fn make_shallow(&self, boundary: &[CommitId]) -> FixtureResult<()> {
        let mut behind = Vec::new();
        let mut stack: Vec<gix::ObjectId> = Vec::new();
        for id in boundary {
            stack.extend(
                self.repo
                    .find_commit(object_id(id))?
                    .parent_ids()
                    .map(gix::Id::detach),
            );
        }
        while let Some(id) = stack.pop() {
            if behind.contains(&id) || !self.repo.has_object(id) {
                continue;
            }
            stack.extend(self.repo.find_commit(id)?.parent_ids().map(gix::Id::detach));
            behind.push(id);
        }
        let mut lines: Vec<&str> = boundary.iter().map(CommitId::as_str).collect();
        lines.sort_unstable();
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(self.git_dir().join("shallow"), text)?;
        for id in behind {
            self.remove_object(&commit_id(id))?;
        }
        Ok(())
    }

    /// Deletes a loose object, as a partial clone would lack it.
    pub fn remove_object(&self, id: &CommitId) -> FixtureResult<()> {
        let hex = id.as_str();
        std::fs::remove_file(
            self.git_dir()
                .join("objects")
                .join(&hex[..2])
                .join(&hex[2..]),
        )?;
        Ok(())
    }

    /// Appends `text` to `.git/config`.
    pub fn append_git_config(&self, text: &str) -> FixtureResult<()> {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.git_dir().join("config"))?;
        file.write_all(text.as_bytes())?;
        Ok(())
    }

    /// Writes a file inside the git directory, such as `info/attributes`.
    pub fn write_git_file(&self, relative: &str, bytes: &[u8]) -> FixtureResult<()> {
        let path = self.git_dir().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)?;
        Ok(())
    }
}
