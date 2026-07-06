//! Hashing helpers compatible with `GraphRAG` input IDs.

use sha2::{Digest, Sha512};

/// Generate a SHA-512 hex hash from selected item values.
///
/// `GraphRAG` concatenates `str(item[column])` for every selected column before
/// hashing. Text-file input uses this with a single `text` value, so this
/// helper accepts the already ordered values and hashes their concatenation.
#[must_use]
pub fn gen_sha512_hash<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha512::new();
    for value in values {
        hasher.update(value.as_bytes());
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
