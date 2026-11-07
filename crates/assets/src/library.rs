// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where assets come from, and the library that loads them: it asks each source in turn for an asset's bytes, uses only
//! bytes whose digest is the asset's, reads them within its limits, and keeps what it read, by digest, so that every
//! instance of an asset, in every environment, shares one model.
//!
//! Today assets are bundled with the app: on Android in the APK's `assets` folder, which the app adds as a source, and
//! elsewhere embedded in the environment's crate, which adds them as an [`Embedded`] source. A source that fetches them
//! from the environment's server side, by digest, can be added beside these without changing what asks for them.
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use crate::model::{Limits, Model};
use crate::{AssetRef, Digest, Error};

/// Somewhere assets' bytes can be read from.
pub trait Source: Send + Sync {
    /// The bytes of `asset`, if this source has them, reading at most `limit` bytes: a source that has more returns
    /// them cut, which the library then refuses as not the asset. The library checks the bytes against the asset's
    /// digest, so a source need not.
    fn read(&self, asset: &AssetRef, limit: usize) -> Option<Vec<u8>>;
}

/// Assets embedded in the program, by digest, as an environment's crate bundles them with `include_bytes!`.
#[derive(Default)]
pub struct Embedded {
    assets: Vec<(Digest, &'static [u8])>,
}

impl Embedded {
    pub fn new() -> Self {
        Self::default()
    }

    /// This source, with `bytes`, which are whichever asset has their digest.
    pub fn with(mut self, bytes: &'static [u8]) -> Self {
        self.assets.push((Digest::of(bytes), bytes));
        self
    }
}

impl Source for Embedded {
    fn read(&self, asset: &AssetRef, limit: usize) -> Option<Vec<u8>> {
        self.assets
            .iter()
            .find(|(digest, _)| *digest == asset.digest())
            .map(|(_, bytes)| bytes[..bytes.len().min(limit)].to_vec())
    }
}

/// Assets in a folder, each at its [`AssetRef::path`] beneath it, as a desktop build or a test may keep them.
pub struct Directory {
    root: PathBuf,
}

impl Directory {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Source for Directory {
    fn read(&self, asset: &AssetRef, limit: usize) -> Option<Vec<u8>> {
        // A name that would leave the folder is not looked for.
        let path = asset.path();
        if path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
            || path.contains('\\')
        {
            return None;
        }
        let file = std::fs::File::open(self.root.join(path)).ok()?;
        let mut bytes = Vec::new();
        file.take(limit as u64).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    }
}

/// Loads assets from its sources, checked against their digests and read within its limits, and keeps each it has
/// loaded, by digest.
pub struct Library {
    sources: RwLock<Vec<Arc<dyn Source>>>,
    models: Mutex<HashMap<Digest, Arc<Model>>>,
    limits: Limits,
}

impl Default for Library {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl Library {
    /// A library without sources, reading models within `limits`.
    pub fn new(limits: Limits) -> Self {
        Self {
            sources: RwLock::default(),
            models: Mutex::default(),
            limits,
        }
    }

    /// The program's own library, which the app and environments add their sources to.
    pub fn shared() -> &'static Library {
        static SHARED: OnceLock<Library> = OnceLock::new();
        SHARED.get_or_init(Library::default)
    }

    /// Look for assets in `source` too, after the sources added before it.
    pub fn add(&self, source: impl Source + 'static) {
        self.sources
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(source));
    }

    /// The bytes of `asset` from the first source that has them as the asset's digest says, or why not: no source has
    /// it, or none has it as it should be.
    pub fn bytes(&self, asset: &AssetRef) -> Result<Vec<u8>, Error> {
        let sources = self
            .sources
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut found = false;
        for source in sources {
            let Some(bytes) = source.read(asset, self.limits.bytes + 1) else {
                continue;
            };
            if asset.matches(&bytes) {
                return Ok(bytes);
            }
            found = true;
            let path = asset.path();
            log::warn!("Asset {path} was found, but not as its digest says.");
        }
        Err(if found {
            Error::mismatch(format!(
                "{} is not the asset its digest names",
                asset.path()
            ))
        } else {
            Error::missing(format!("{} is in none of the asset sources", asset.path()))
        })
    }

    /// The model `asset`, loaded once and shared by everything that asks for it.
    pub fn model(&self, asset: &AssetRef) -> Result<Arc<Model>, Error> {
        if let Some(model) = self
            .models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&asset.digest())
        {
            return Ok(model.clone());
        }
        let model = Arc::new(crate::gltf::read(&self.bytes(asset)?, &self.limits)?);
        Ok(self
            .models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(asset.digest())
            .or_insert(model)
            .clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;

    fn reference(bytes: &[u8]) -> AssetRef {
        AssetRef::new("test", 1, "triangle.glb", Digest::of(bytes))
    }

    #[test]
    fn a_library_loads_an_asset_once_from_the_first_source_that_has_it_as_it_should_be() {
        let tiny: &'static [u8] =
            Box::leak(crate::gltf::tests::Glb::arm().bytes().into_boxed_slice());
        let library = Library::default();
        let triangle = reference(tiny);
        assert_eq!(
            library.bytes(&triangle).unwrap_err().kind(),
            ErrorKind::Missing
        );
        // A source with other bytes at the same path is not the asset.
        let folder = std::env::temp_dir().join(format!("tomori-assets-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("test")).unwrap();
        std::fs::write(folder.join("test/triangle.glb"), b"not the triangle").unwrap();
        library.add(Directory::new(&folder));
        assert_eq!(
            library.model(&triangle).unwrap_err().kind(),
            ErrorKind::Mismatch
        );
        library.add(Embedded::new().with(tiny));
        let first = library.model(&triangle).unwrap();
        let again = library.model(&triangle).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        // A folder with the asset is read as well as embedded bytes are, within the limit.
        std::fs::write(folder.join("test/triangle.glb"), tiny).unwrap();
        let read = Library::default();
        read.add(Directory::new(&folder));
        assert_eq!(read.bytes(&triangle).unwrap(), tiny);
        assert_eq!(Directory::new(&folder).read(&triangle, 4).unwrap().len(), 4);
        assert_eq!(
            Embedded::new().with(tiny).read(&triangle, 4).unwrap().len(),
            4
        );
        // A source without the asset is passed over for one with it, and one that cannot be read, such as a folder, has nothing.
        let passed = Library::default();
        passed.add(Embedded::new());
        passed.add(Embedded::new().with(tiny));
        assert_eq!(passed.bytes(&triangle).unwrap(), tiny);
        std::fs::create_dir_all(folder.join("test/folder.glb")).unwrap();
        let folder_asset = AssetRef::new("test", 1, "folder.glb", Digest::of(tiny));
        assert_eq!(Directory::new(&folder).read(&folder_asset, 100), None);
        // A name that would leave the folder, or that is not there, is not read.
        for name in ["../triangle.glb", "a//b", "./x", "a\\b"] {
            let escaping = AssetRef::new("test", 1, name, Digest::of(tiny));
            assert_eq!(Directory::new(&folder).read(&escaping, 100), None);
        }
        let absent = AssetRef::new("test", 1, "absent.glb", Digest::of(tiny));
        assert_eq!(Directory::new(&folder).read(&absent, 100), None);
        std::fs::remove_dir_all(&folder).unwrap();
        // A broken asset is refused, and nothing is kept of it.
        let broken: &'static [u8] = b"broken";
        let library = Library::default();
        library.add(Embedded::new().with(broken));
        assert_eq!(
            library.model(&reference(broken)).unwrap_err().kind(),
            ErrorKind::Invalid
        );
        // The shared library is one.
        assert!(std::ptr::eq(Library::shared(), Library::shared()));
    }
}
