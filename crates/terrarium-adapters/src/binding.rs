//! The repository a setting was given for (I-5). The opt-in that runs Knip, and the GitHub
//! repository typed for the CI weather, each hold for the repository they were set for, never
//! for the next one connected: another repository's code never runs on an opt-in it was not
//! given, and one repository's CI never reads as another's weather.

use serde::{Deserialize, Serialize};
use terrarium_truth::RepoSeed;

use crate::ci::Slug;

/// A repository, as the setting names it: its seed and the folder it was connected at. Both
/// must match, so a fork sharing the seed in another folder is another repository, and a
/// repository moved to another folder is asked again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoBinding {
    /// The seed, as lowercase hex.
    pub seed: String,
    /// The folder, as the terrarium records it.
    pub path: String,
}

impl RepoBinding {
    pub fn new(seed: &RepoSeed, path: &str) -> RepoBinding {
        RepoBinding {
            seed: seed.to_hex(),
            path: path.to_owned(),
        }
    }
}

/// The GitHub repository typed for one repository's CI weather (`ci.github.repo`): stored as
/// `{"seed": …, "path": …, "repo": "owner/name"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiRepository {
    #[serde(flatten)]
    pub binding: RepoBinding,
    pub repo: Slug,
}
