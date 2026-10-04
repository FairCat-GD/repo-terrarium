//! A large synthetic history for the history-reading bench (N-05).
//!
//! It starts from a tree of 3,000 files nested four directories deep, plus a few large blobs,
//! so that reading it is only fast when each commit's diff stays inside the trees it touched.
//! Then most commits change one to three files; every so often a side branch is merged back, a
//! file is renamed, or one is deleted.

use std::num::NonZeroU32;
use std::path::Path;

use terrarium_truth::Pcg32;

use super::{Edit, FixtureRepo, FixtureResult, Stamp};

const FILES: usize = 3_000;
const LARGE_BLOBS: usize = 3;
/// Monday 2022-01-03, 00:00 UTC.
const START_SECS: i64 = 1_641_168_000;
/// About three years.
const SPAN_SECS: i64 = 3 * 365 * 86_400;

struct Files {
    rng: Pcg32,
    paths: Vec<String>,
    bodies: Vec<Vec<u8>>,
}

impl Files {
    fn below(&mut self, bound: usize) -> usize {
        let bound = u32::try_from(bound).expect("small bounds");
        self.rng
            .below(NonZeroU32::new(bound).expect("a positive bound")) as usize
    }

    fn grow(&mut self, index: usize) -> Edit {
        let line = format!(
            "pub const SEEDLING_{}: u32 = {};\n",
            self.below(1 << 20),
            self.below(997)
        );
        self.bodies[index].extend_from_slice(line.as_bytes());
        Edit::Write(self.paths[index].clone(), self.bodies[index].clone())
    }
}

/// `count` lines, each made by `line` and ended with a newline.
fn lines(count: usize, line: impl Fn(usize) -> String) -> String {
    (0..count).fold(String::new(), |mut text, n| {
        text.push_str(&line(n));
        text.push('\n');
        text
    })
}

/// Builds a synthetic history of `commits` commits in `path`, which must be empty or absent.
pub fn synthetic(path: &Path, commits: usize) -> FixtureResult<FixtureRepo> {
    let mut repo = FixtureRepo::init(path)?;
    let mut files = Files {
        rng: Pcg32::from_seed(0x7379_6e74_6865_7469),
        paths: Vec::with_capacity(FILES),
        bodies: Vec::with_capacity(FILES),
    };
    let mut first = Vec::with_capacity(FILES + LARGE_BLOBS);
    for i in 0..FILES {
        let path = format!(
            "a{}/b{}/c{}/d{}/f{i}.rs",
            i % 10,
            (i / 10) % 10,
            (i / 100) % 5,
            (i / 500) % 6
        );
        let body = lines(20, |line| {
            format!(
                "pub const LEAF_{i}_{line}: u32 = {};",
                (i * 31 + line) % 1009
            )
        });
        first.push(Edit::Write(path.clone(), body.clone().into_bytes()));
        files.paths.push(path);
        files.bodies.push(body.into_bytes());
    }
    for i in 0..LARGE_BLOBS {
        let body = lines(2_500, |line| {
            format!("pub const BOULDER_{i}_{line}: u64 = {line};")
        });
        first.push(Edit::Write(
            format!("large/boulder{i}.rs"),
            body.into_bytes(),
        ));
    }
    let step = SPAN_SECS / i64::try_from(commits.max(1))?;
    let stamp = |n: usize| Stamp {
        secs: START_SECS + step * i64::try_from(n).unwrap_or(i64::MAX / 2),
        offset: 0,
        author: n % 4,
    };
    repo.commit("main", stamp(0), "lay out the forest", &first)?;
    let mut made = 1;
    let mut renamed = 0;
    while made < commits {
        if made % 250 == 0 && made + 5 < commits {
            let side = format!("side/{made}");
            repo.branch(&side, "main")?;
            for k in 0..4 {
                let edit = files.grow((made * 7 + k) % FILES);
                repo.commit(&side, stamp(made), "grow a side shoot", &[edit])?;
                made += 1;
            }
            repo.merge("main", &side, stamp(made), "merge the side shoot")?;
            made += 1;
            continue;
        }
        let edits = if made % 500 == 7 {
            let index = files.below(FILES);
            let from = files.paths[index].clone();
            renamed += 1;
            files.paths[index] = format!("moved/r{renamed}.rs");
            vec![Edit::Remove(from), files.grow(index)]
        } else if made % 700 == 3 {
            let index = files.below(FILES);
            vec![Edit::Remove(files.paths[index].clone())]
        } else {
            let count = 1 + files.below(3);
            (0..count)
                .map(|_| {
                    let index = files.below(FILES);
                    files.grow(index)
                })
                .collect()
        };
        repo.commit("main", stamp(made), "tend the forest", &edits)?;
        made += 1;
    }
    Ok(repo)
}
