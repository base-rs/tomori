// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A cache of assets on the device, by digest: what a client fetched from its server, kept across sessions in a folder
//! of its own, at most so many bytes together, the least recently used let go first, and checked against its digest
//! whenever it is written or read. It is a [`Source`] like the bundled ones, so that nothing that asks for assets
//! changes; a client adds it to its library first, so that an asset its server serves is read as its server serves it,
//! and what the app bundles is the fallback, for a server that serves nothing. See `design/clients.md`.
//!
//! Each asset is a file named by its digest, 64 hexadecimal digits, so that no name from anywhere is ever a path; a
//! download being written is `DIGEST.part` beside it, kept to resume from, and only one is kept at a time. Files of any
//! other name are left alone. How recently each was used is its file's modification time, which reading it sets, so
//! that the order survives a restart.
//!
//! ```
//! use tomori_assets::{AssetRef, Cache, Digest, Library};
//!
//! let folder = std::env::temp_dir().join(format!("tomori-cache-doc-{}", std::process::id()));
//! let cache = std::sync::Arc::new(Cache::open(&folder, 1 << 20).unwrap());
//! let digest = Digest::of(b"lantern");
//! let mut writer = cache.writer(&digest, 0).unwrap();
//! writer.write(b"lantern").unwrap();
//! writer.finish().unwrap();
//! let library = Library::default();
//! library.add(cache.clone());
//! assert_eq!(library.bytes(&AssetRef::new("garden", 1, "lantern.glb", digest)).unwrap(), b"lantern");
//! std::fs::remove_dir_all(folder).unwrap();
//! ```
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use sha2::{Digest as _, Sha256};

use crate::{AssetRef, Digest, Source};

/// The suffix of a download being written.
const PARTIAL: &str = ".part";

/// How many of a download's first bytes its [`Writer`] keeps, enough to tell its kind by its signature.
const HEAD_BYTES: usize = 16;

/// An asset kept: its length, and when it was last used, as an order.
struct Entry {
    length: u64,
    used: u64,
}

#[derive(Default)]
struct Index {
    entries: HashMap<Digest, Entry>,
    total: u64,
    /// The next use's order.
    next: u64,
}

impl Index {
    fn touch(&mut self, digest: &Digest) {
        let next = self.next;
        let found = self.entries.get_mut(digest).map(|entry| entry.used = next);
        self.next += u64::from(found.is_some());
    }

    /// Keep `digest`, `length` bytes long, as used now.
    fn insert(&mut self, digest: Digest, length: u64) {
        self.remove(&digest);
        let used = self.next;
        self.next += 1;
        self.total += length;
        self.entries.insert(digest, Entry { length, used });
    }

    fn remove(&mut self, digest: &Digest) {
        if let Some(entry) = self.entries.remove(digest) {
            self.total -= entry.length;
        }
    }
}

/// Assets fetched and kept on the device, by digest. See the [module documentation](self).
pub struct Cache {
    root: PathBuf,
    most: u64,
    index: Mutex<Index>,
}

impl Cache {
    /// The cache in the folder `root`, made if it is not there, readable only by its user where the system has
    /// permissions, keeping at most `most` bytes, with what it kept before: each file named by a digest, the least
    /// recently used first let go of if together they are more.
    pub fn open(root: impl Into<PathBuf>, most: u64) -> io::Result<Self> {
        let root = root.into();
        let mut folder = fs::DirBuilder::new();
        folder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut folder, 0o700);
        let mut found: Vec<(SystemTime, Digest, u64)> = folder
            .create(&root)
            .and_then(|()| fs::read_dir(&root))?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                // Only a file named by a digest, written as this cache writes it, is one of its assets.
                let name = entry.file_name();
                let digest = name.to_str().and_then(|name| {
                    Digest::parse(name).filter(|digest| digest.to_string() == name)
                });
                let metadata = entry.metadata().ok().filter(fs::Metadata::is_file);
                digest.zip(metadata).map(|(digest, metadata)| {
                    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    (modified, digest, metadata.len())
                })
            })
            .collect();
        // The oldest first, so that each is used after those used before it.
        found.sort();
        let mut index = Index::default();
        for (_, digest, length) in found {
            index.insert(digest, length);
        }
        let cache = Self {
            root,
            most,
            index: Mutex::new(index),
        };
        cache.evict(None);
        Ok(cache)
    }

    /// Where it keeps its files.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The most bytes it keeps together.
    pub fn most(&self) -> u64 {
        self.most
    }

    /// How many bytes it keeps now.
    pub fn total(&self) -> u64 {
        self.lock().total
    }

    /// Whether it keeps the asset `digest`.
    pub fn contains(&self, digest: &Digest) -> bool {
        self.lock().entries.contains_key(digest)
    }

    /// How much of a download of `digest` it kept from before, to resume from: none unless that download is the one
    /// it keeps.
    pub fn partial(&self, digest: &Digest) -> u64 {
        fs::metadata(self.partial_path(digest)).map_or(0, |metadata| metadata.len())
    }

    /// Write the asset `digest` from `offset`, after the bytes of its download kept from before, or from the start,
    /// for `0`, letting go of any other download kept.
    pub fn writer(&self, digest: &Digest, offset: u64) -> io::Result<Writer<'_>> {
        let path = self.partial_path(digest);
        let mut hasher = Sha256::new();
        let (file, head) = fs::read_dir(&self.root)
            .map(|entries| {
                for other in entries.filter_map(Result::ok).map(|entry| entry.path()) {
                    if other != path && other.to_string_lossy().ends_with(PARTIAL) {
                        let _ = fs::remove_file(other);
                    }
                }
            })
            .and_then(|()| {
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .truncate(offset == 0)
                    .open(&path)
            })
            // What was kept of it is read again, so that the whole is checked against its digest.
            .and_then(|mut file| {
                let mut kept = Vec::new();
                (&mut file)
                    .take(offset)
                    .read_to_end(&mut kept)
                    .map(|_| (file, kept))
            })
            .and_then(|(mut file, mut kept)| {
                if (kept.len() as u64) < offset {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a download resumes only from what was kept of it",
                    ));
                }
                hasher.update(&kept);
                kept.truncate(HEAD_BYTES);
                file.set_len(offset)
                    .and_then(|()| file.seek(io::SeekFrom::End(0)))
                    .map(|_| (file, kept))
            })?;
        Ok(Writer {
            cache: self,
            digest: *digest,
            path,
            file,
            hasher,
            written: offset,
            head,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Index> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn path(&self, digest: &Digest) -> PathBuf {
        self.root.join(digest.to_string())
    }

    fn partial_path(&self, digest: &Digest) -> PathBuf {
        self.root.join(format!("{digest}{PARTIAL}"))
    }

    /// Let go of the least recently used until it keeps at most its bound, never `keep`.
    fn evict(&self, keep: Option<&Digest>) {
        let mut index = self.lock();
        loop {
            let oldest = (index.total > self.most)
                .then(|| {
                    index
                        .entries
                        .iter()
                        .filter(|(digest, _)| Some(*digest) != keep)
                        .min_by_key(|(_, entry)| entry.used)
                        .map(|(digest, _)| *digest)
                })
                .flatten();
            let Some(oldest) = oldest else {
                break;
            };
            index.remove(&oldest);
            let _ = fs::remove_file(self.path(&oldest));
            log::debug!("Let go of the cached asset {oldest}, used least recently.");
        }
    }

    /// Forget `digest` and remove its file, as it is not what its name says.
    fn forget(&self, digest: &Digest) {
        self.lock().remove(digest);
        let _ = fs::remove_file(self.path(digest));
    }
}

/// Equal when they keep the same folder within the same bound.
impl PartialEq for Cache {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.most == other.most
    }
}

impl Eq for Cache {}

impl std::fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Cache")
            .field("root", &self.root)
            .field("most", &self.most)
            .finish()
    }
}

impl Source for Cache {
    fn read(&self, asset: &AssetRef, limit: usize) -> Option<Vec<u8>> {
        let digest = asset.digest();
        if !self.contains(&digest) {
            return None;
        }
        let path = self.path(&digest);
        let mut bytes = Vec::new();
        let read = File::open(&path).and_then(|file| {
            (&file)
                .take(limit as u64)
                .read_to_end(&mut bytes)
                // Read now, so that it is let go of after those used before it, after a restart too.
                .map(|_| {
                    let _ = file.set_modified(SystemTime::now());
                })
        });
        if read.is_err() || (bytes.len() < limit && !asset.matches(&bytes)) {
            log::warn!(
                "The cached asset {} was not what its digest says, and is let go of.",
                asset.path()
            );
            self.forget(&digest);
            return None;
        }
        self.lock().touch(&digest);
        Some(bytes)
    }
}

/// A download of an asset into the cache, written in order and checked against its digest as it is, which the cache
/// keeps once it is whole and what its digest says.
pub struct Writer<'a> {
    cache: &'a Cache,
    digest: Digest,
    path: PathBuf,
    file: File,
    hasher: Sha256,
    written: u64,
    /// Its first bytes, at most [`HEAD_BYTES`].
    head: Vec<u8>,
}

impl Writer<'_> {
    /// Write `bytes` after those written before.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let written = self.file.write_all(bytes);
        written.map(|()| {
            self.hasher.update(bytes);
            self.written += bytes.len() as u64;
            let room = HEAD_BYTES.saturating_sub(self.head.len());
            self.head.extend(bytes.iter().take(room));
        })
    }

    /// Its first bytes, those kept from before included, at most 16: enough to check that it starts as its kind must.
    pub fn head(&self) -> &[u8] {
        &self.head
    }

    /// How many bytes of it are written, those kept from before included.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Keep the asset, if it is what its digest says and fits the cache's bound, letting go of the least recently used
    /// to make room; otherwise let go of what was written, and say why.
    pub fn finish(self) -> io::Result<()> {
        let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
        drop(self.file);
        let refused = if Digest::from_bytes(self.hasher.finalize().into()) != self.digest {
            Some(invalid(format!(
                "{} is not what its digest says",
                self.digest
            )))
        } else if self.written > self.cache.most {
            Some(invalid(format!(
                "{} is larger than the cache keeps, {} bytes",
                self.digest, self.cache.most
            )))
        } else {
            None
        };
        if let Some(refused) = refused {
            let _ = fs::remove_file(&self.path);
            return Err(refused);
        }
        let renamed = fs::rename(&self.path, self.cache.path(&self.digest));
        renamed.map(|()| {
            self.cache.lock().insert(self.digest, self.written);
            self.cache.evict(Some(&self.digest));
        })
    }
}

#[cfg(test)]
mod tests;
