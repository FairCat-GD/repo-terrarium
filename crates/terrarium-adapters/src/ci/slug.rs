//! Which GitHub repository the weather reads: `owner/name`, typed in the settings or read from
//! the repository's `origin` remote.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The hosts an `origin` on GitHub names: the web host, and the SSH host on port 443.
const HOSTS: [&str; 3] = ["github.com", "www.github.com", "ssh.github.com"];

/// A GitHub repository, `owner/name`, with the characters GitHub allows: an owner of letters,
/// digits and hyphens, a name of letters, digits, `.`, `-` and `_`, never `.` or `..`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Slug {
    owner: String,
    name: String,
}

impl Slug {
    /// `owner/name`, as typed: surrounding space is ignored, and a trailing `.git` is not part
    /// of the name.
    pub fn parse(text: &str) -> Option<Slug> {
        let (owner, name) = text.trim().split_once('/')?;
        Slug::of(owner, name)
    }

    fn of(owner: &str, name: &str) -> Option<Slug> {
        let name = name.strip_suffix(".git").unwrap_or(name);
        let owner_ok = (1..=39).contains(&owner.len())
            && owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        let name_ok = (1..=100).contains(&name.len())
            && name != "."
            && name != ".."
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        (owner_ok && name_ok).then(|| Slug {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// The repository a remote URL names on GitHub: `https://github.com/o/r(.git)`,
    /// `git@github.com:o/r(.git)`, `ssh://git@github.com/o/r(.git)` or
    /// `ssh://git@ssh.github.com:443/o/r.git`. `None` for any other host.
    pub fn from_remote_url(url: &str) -> Option<Slug> {
        let url = url.trim();
        let (host, path) = if let Some((_, rest)) = url.split_once("://") {
            let (authority, path) = rest.split_once('/')?;
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            let host = host.split_once(':').map_or(host, |(host, _)| host);
            (host, path)
        } else {
            // `[user@]host:path`, git's scp-like form; a `/` before the `:` is a local path.
            let (authority, path) = url.split_once(':')?;
            if authority.contains('/') {
                return None;
            }
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            (host, path)
        };
        if !HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
            return None;
        }
        let path = path.trim_start_matches('/').trim_end_matches('/');
        let (owner, name) = path.split_once('/')?;
        Slug::of(owner, name)
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl TryFrom<String> for Slug {
    type Error = String;

    fn try_from(text: String) -> Result<Slug, String> {
        Slug::parse(&text).ok_or(text)
    }
}

impl From<Slug> for String {
    fn from(slug: Slug) -> String {
        slug.to_string()
    }
}
