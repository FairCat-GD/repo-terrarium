//! What a terrarium is keyed by, and which version of the sim produced it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The version of truth's output (I-13). Any change to what truth computes bumps it, including
/// a change to the postcard or canonical-dump encodings. It is independent of the product
/// version.
pub const SIM_VERSION: SimVersion = SimVersion(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimVersion(pub u32);

/// A terrarium's identity: derived once from the repository's root commit when it is connected,
/// and stored, so relinking the same repository finds it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RepoSeed([u8; 32]);

impl RepoSeed {
    pub const fn from_bytes(bytes: [u8; 32]) -> RepoSeed {
        RepoSeed(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex, 64 characters.
    pub fn to_hex(&self) -> String {
        crate::canon::to_hex(&self.0)
    }

    /// The seed of a repository whose first-parent walk from the default tip ends at `root`:
    /// the SHA-256 of the root commit id's bytes, not of its hex text. Stored terrariums are
    /// keyed by it, so it never changes.
    ///
    /// # Panics
    ///
    /// Never: a [`CommitId`] is lowercase hex of even length by construction.
    pub fn from_root_commit(root: &CommitId) -> RepoSeed {
        let bytes = crate::canon::from_hex(root.as_str()).expect("a commit id is lowercase hex");
        RepoSeed(Sha256::digest(&bytes).into())
    }

    /// The inverse of [`RepoSeed::to_hex`]; only lowercase hex of the right length parses.
    pub fn from_hex(text: &str) -> Option<RepoSeed> {
        let bytes = crate::canon::from_hex(text)?;
        bytes.try_into().ok().map(RepoSeed)
    }
}

/// A git object id in lowercase hex: 40 characters for SHA-1, 64 for SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct CommitId(String);

impl CommitId {
    pub fn from_hex(text: &str) -> Option<CommitId> {
        let valid = matches!(text.len(), 40 | 64)
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| CommitId(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommitId {
    type Error = &'static str;

    fn try_from(text: String) -> Result<CommitId, Self::Error> {
        CommitId::from_hex(&text).ok_or("not a lowercase hex object id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sim_is_at_version_5() {
        assert_eq!(SIM_VERSION, SimVersion(5));
    }

    #[test]
    fn seeds_round_trip_through_lowercase_hex() {
        let mut bytes = [0; 32];
        for (byte, value) in bytes.iter_mut().zip(0xa0..) {
            *byte = value;
        }
        let seed = RepoSeed::from_bytes(bytes);
        let hex = seed.to_hex();
        assert_eq!(&hex[..8], "a0a1a2a3");
        assert_eq!(RepoSeed::from_hex(&hex), Some(seed));
        assert_eq!(RepoSeed::from_hex(&hex.to_uppercase()), None);
        assert_eq!(RepoSeed::from_hex(&hex[2..]), None);
        assert_eq!(RepoSeed::from_hex(&hex[1..]), None);
    }

    #[test]
    fn the_seed_hashes_the_root_commit_ids_bytes() {
        // Pinned: stored terrariums are found by this value. The SHA-256 of the 20 bytes
        // 0x01 0x23 … 0x67, as `printf '0123…4567' | xxd -r -p | shasum -a 256` prints it.
        let root = CommitId::from_hex("0123456789abcdef0123456789abcdef01234567").expect("an id");
        assert_eq!(
            RepoSeed::from_root_commit(&root).to_hex(),
            "7e74ce75b5f2f89240b33afff241e209f98f7b8920af1b572957c8b030430d7a"
        );
    }

    #[test]
    fn commit_ids_are_lowercase_hex_of_either_length() {
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(
            CommitId::from_hex(sha1).map(|id| id.0),
            Some(sha1.to_owned())
        );
        assert!(CommitId::from_hex(sha256).is_some());
        assert!(CommitId::from_hex(&sha1.to_uppercase()).is_none());
        assert!(CommitId::from_hex(&sha1[1..]).is_none());
        assert!(CommitId::from_hex(&sha1.replace('0', "g")).is_none());
        let decoded: Result<CommitId, _> =
            postcard::from_bytes(&postcard::to_allocvec("abc").expect("encodes"));
        assert!(decoded.is_err());
    }
}
