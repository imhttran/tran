//! Private byte store for archive photograph assets.
//!
//! The store keeps photograph bytes in a content-addressed tree under one
//! configurable root. The default root is `data/photographs`, which sits in the
//! already-ignored `data/` directory. Only relative storage keys are stored; the
//! store never records or returns an absolute path, and every write and read
//! stays inside the root.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// Default store root, inside the already-ignored `data/` directory.
pub const DEFAULT_PHOTOGRAPH_STORE_ROOT: &str = "data/photographs";

/// Errors raised by the private photograph store. No variant carries bytes or
/// an absolute path.
#[derive(Debug, Error)]
pub enum PhotographStoreError {
    /// The key is not a content-hash key inside the root.
    #[error("invalid photograph storage key: {0}")]
    InvalidKey(String),
    /// No stored object exists for the key.
    #[error("unknown photograph storage key: {0}")]
    UnknownKey(String),
    /// An existing object holds different bytes than the caller supplied.
    #[error("the stored photograph bytes do not match the key")]
    ContentMismatch,
    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The outcome of one [`PhotographStore::put`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutOutcome {
    /// The relative content-hash key.
    pub key: String,
    /// Lowercase hex SHA-256 of the bytes.
    pub content_sha256: String,
    /// True when a new file was written; false when the identical object
    /// already existed and was reused.
    pub created: bool,
}

/// Private, content-addressed byte store for photograph assets.
#[derive(Debug, Clone)]
pub struct PhotographStore {
    root: PathBuf,
}

impl Default for PhotographStore {
    fn default() -> Self {
        Self::new(DEFAULT_PHOTOGRAPH_STORE_ROOT)
    }
}

impl PhotographStore {
    /// Creates a store rooted at `root`. A relative root resolves against the
    /// process working directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The configured root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the relative content-hash key for `bytes` without writing.
    pub fn key_for(&self, bytes: &[u8]) -> String {
        storage_key(&content_hash(bytes))
    }

    /// Stores `bytes` and returns the relative key.
    ///
    /// A repeated put of identical bytes returns the existing key and writes
    /// nothing. A different byte sequence produces a different key.
    pub fn put(&self, bytes: &[u8]) -> Result<PutOutcome, PhotographStoreError> {
        let hash = content_hash(bytes);
        let key = storage_key(&hash);
        let path = self.resolve_key(&key)?;
        if path.exists() {
            let existing = fs::read(&path)?;
            if existing == bytes {
                return Ok(PutOutcome {
                    key,
                    content_sha256: hash,
                    created: false,
                });
            }
            return Err(PhotographStoreError::ContentMismatch);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes)?;
        fs::rename(&temp, &path)?;
        Ok(PutOutcome {
            key,
            content_sha256: hash,
            created: true,
        })
    }

    /// Reads the bytes for a key this store produced.
    ///
    /// An unknown key, an absolute path, a key with `..`, or any key outside the
    /// content-hash scheme is an error.
    pub fn read(&self, key: &str) -> Result<Vec<u8>, PhotographStoreError> {
        let path = self.resolve_key(key)?;
        if !path.is_file() {
            return Err(PhotographStoreError::UnknownKey(key.to_string()));
        }
        Ok(fs::read(&path)?)
    }

    /// Reports whether the store holds an object for `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.resolve_key(key)
            .map(|path| path.is_file())
            .unwrap_or(false)
    }

    /// Maps a relative content-hash key to a path inside the root. Every other
    /// key shape is rejected before any path is built.
    fn resolve_key(&self, key: &str) -> Result<PathBuf, PhotographStoreError> {
        if !is_storage_key(key) {
            return Err(PhotographStoreError::InvalidKey(key.to_string()));
        }
        let candidate = Path::new(key);
        let safe = !candidate.is_absolute()
            && candidate
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if !safe {
            return Err(PhotographStoreError::InvalidKey(key.to_string()));
        }
        Ok(self.root.join(candidate))
    }
}

/// Returns the lowercase hex SHA-256 of `bytes`.
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Derives the relative storage key `<first two hex>/<sha256>`.
pub fn storage_key(sha256: &str) -> String {
    let split = sha256.len().min(2);
    format!("{}/{}", &sha256[..split], sha256)
}

/// True only for a relative `<2 hex>/<64 lowercase hex>` key.
fn is_storage_key(key: &str) -> bool {
    let Some((prefix, name)) = key.split_once('/') else {
        return false;
    };
    if name.contains('/') {
        return false;
    }
    prefix.len() == 2
        && name.len() == 64
        && name
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))
        && prefix
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))
        && name.starts_with(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, PhotographStore) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = PhotographStore::new(dir.path().join("photographs"));
        (dir, store)
    }

    #[test]
    fn content_hash_is_lowercase_hex_sha256() {
        // Known SHA-256 of the empty input and of `abc`.
        assert_eq!(
            content_hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            content_hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn storage_key_uses_the_first_two_hex_characters() {
        let hash = content_hash(b"abc");
        assert_eq!(storage_key(&hash), format!("ba/{hash}"));
    }

    #[test]
    fn identical_bytes_deduplicate_to_one_key() {
        let (_dir, store) = temp_store();
        let first = store.put(b"one photograph").expect("first put");
        let second = store.put(b"one photograph").expect("second put");
        assert_eq!(first.key, second.key);
        assert!(first.created);
        assert!(!second.created);
    }

    #[test]
    fn a_second_put_of_identical_bytes_reuses_the_stored_file() {
        let (dir, store) = temp_store();
        let first = store.put(b"same bytes").expect("first put");
        let stored = store.root().join(&first.key);
        let before = fs::read(&stored).expect("read stored");
        let second = store.put(b"same bytes").expect("second put");
        assert_eq!(first.key, second.key);
        assert!(!second.created);
        assert_eq!(fs::read(&stored).expect("read stored again"), before);
        // Exactly one object file exists under the root.
        let files: Vec<_> = walk_files(dir.path().join("photographs"));
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn different_bytes_produce_two_keys() {
        let (_dir, store) = temp_store();
        let first = store.put(b"alpha").expect("first put");
        let second = store.put(b"beta").expect("second put");
        assert_ne!(first.key, second.key);
        assert!(first.created);
        assert!(second.created);
    }

    #[test]
    fn read_returns_the_stored_bytes() {
        let (_dir, store) = temp_store();
        let outcome = store.put(b"readable bytes").expect("put");
        assert_eq!(store.read(&outcome.key).expect("read"), b"readable bytes");
    }

    #[test]
    fn unknown_absolute_and_parent_keys_are_rejected() {
        let (_dir, store) = temp_store();
        let unknown = storage_key(&content_hash(b"never stored"));
        assert!(matches!(
            store.read(&unknown),
            Err(PhotographStoreError::UnknownKey(_))
        ));
        assert!(matches!(
            store.read("/etc/passwd"),
            Err(PhotographStoreError::InvalidKey(_))
        ));
        assert!(matches!(
            store.read("aa/../../etc/passwd"),
            Err(PhotographStoreError::InvalidKey(_))
        ));
        assert!(matches!(
            store.put(b"x").and_then(|_| store.read("../escape")),
            Err(PhotographStoreError::InvalidKey(_))
        ));
    }

    #[test]
    fn writes_stay_under_the_configured_root() {
        let (dir, store) = temp_store();
        let outcome = store.put(b"scoped bytes").expect("put");
        assert!(!Path::new(&outcome.key).is_absolute());
        let full = store.root().join(&outcome.key);
        assert!(full.starts_with(dir.path().join("photographs")));
        assert!(full.is_file());
    }

    #[test]
    fn default_root_is_under_the_ignored_data_directory() {
        let store = PhotographStore::default();
        assert_eq!(store.root(), Path::new("data/photographs"));
        assert!(!store.root().is_absolute());
    }

    fn walk_files(root: PathBuf) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![root];
        while let Some(path) = stack.pop() {
            let Ok(entries) = fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files
    }
}
