// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::*;
use crate::Library;

/// An empty folder of its own for a test.
fn folder(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("tomori-assets-cache-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    folder
}

fn asset(bytes: &[u8]) -> AssetRef {
    AssetRef::new("garden", 1, "thing.glb", Digest::of(bytes))
}

/// Write `bytes` into `cache` whole.
fn keep(cache: &Cache, bytes: &[u8]) -> io::Result<()> {
    let mut writer = cache.writer(&Digest::of(bytes), 0)?;
    writer.write(bytes)?;
    writer.finish()
}

#[test]
fn an_asset_written_whole_is_kept_and_read_again_after_a_restart() {
    let root = folder("restart");
    let cache = Arc::new(Cache::open(&root, 1000).unwrap());
    assert_eq!(
        (cache.root(), cache.most(), cache.total()),
        (root.as_path(), 1000, 0)
    );
    // Its folder is its user's alone.
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&fs::metadata(&root).unwrap().permissions())
            & 0o777,
        0o700
    );
    let lantern = b"a lantern";
    assert!(!cache.contains(&Digest::of(lantern)));
    assert_eq!(cache.read(&asset(lantern), 100), None);
    keep(&cache, lantern).unwrap();
    assert!(cache.contains(&Digest::of(lantern)));
    assert_eq!(cache.total(), lantern.len() as u64);
    let library = Library::default();
    library.add(cache.clone());
    assert_eq!(library.bytes(&asset(lantern)).unwrap(), lantern);
    // Read within a limit, it is cut, as every source cuts what it reads, and the library refuses it.
    assert_eq!(cache.read(&asset(lantern), 4).unwrap(), b"a la");
    // Kept again, it is the same asset.
    keep(&cache, lantern).unwrap();
    assert_eq!(cache.total(), lantern.len() as u64);
    drop(library);
    drop(cache);
    let again = Cache::open(&root, 1000).unwrap();
    assert!(again.contains(&Digest::of(lantern)));
    assert_eq!(again.read(&asset(lantern), 100).unwrap(), lantern);
    assert_eq!(again, Cache::open(&root, 1000).unwrap());
    assert_ne!(again, Cache::open(&root, 999).unwrap());
    assert_eq!(
        format!("{again:?}"),
        format!("Cache {{ root: {root:?}, most: 1000 }}")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn what_is_not_its_digest_is_never_kept_nor_read() {
    log::set_max_level(log::LevelFilter::Trace);
    let root = folder("verified");
    let cache = Cache::open(&root, 1000).unwrap();
    // Written as other bytes than its digest names, it is refused and let go of.
    let claimed = Digest::of(b"the lantern");
    let mut writer = cache.writer(&claimed, 0).unwrap();
    writer.write(b"a candle").unwrap();
    let refused = writer.finish().unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    assert!(refused.to_string().ends_with("is not what its digest says"));
    assert!(!cache.contains(&claimed));
    assert_eq!(cache.partial(&claimed), 0);
    // Changed on the device after it was kept, it is let go of as it is read.
    let lantern = b"the lantern";
    keep(&cache, lantern).unwrap();
    fs::write(root.join(claimed.to_string()), b"scrambled").unwrap();
    assert_eq!(cache.read(&asset(lantern), 100), None);
    assert!(!cache.contains(&claimed));
    assert!(!root.join(claimed.to_string()).exists());
    // As it is when its file has gone.
    keep(&cache, lantern).unwrap();
    fs::remove_file(root.join(claimed.to_string())).unwrap();
    assert_eq!(cache.read(&asset(lantern), 100), None);
    assert_eq!(cache.total(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_download_that_stopped_resumes_where_it_left_off() {
    let root = folder("resume");
    let cache = Cache::open(&root, 1000).unwrap();
    let lantern = b"a lantern, fetched in two parts";
    let digest = Digest::of(lantern);
    let mut writer = cache.writer(&digest, 0).unwrap();
    writer.write(&lantern[..10]).unwrap();
    assert_eq!(writer.written(), 10);
    // The connection ends here.
    drop(writer);
    assert!(!cache.contains(&digest));
    assert_eq!(cache.partial(&digest), 10);
    // A download resumes only from what was kept of it.
    assert_eq!(
        cache.writer(&digest, 11).err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
    let mut writer = cache.writer(&digest, 10).unwrap();
    assert_eq!(writer.written(), 10);
    // Its head is what was kept of it, and then what follows, up to a signature's worth.
    assert_eq!(writer.head(), &lantern[..10]);
    writer.write(&lantern[10..]).unwrap();
    assert_eq!(writer.head(), &lantern[..HEAD_BYTES]);
    writer.finish().unwrap();
    assert_eq!(cache.read(&asset(lantern), 100).unwrap(), lantern);
    assert_eq!(cache.partial(&digest), 0);
    // A download of another asset lets go of the one kept, and one begun again starts afresh.
    let mut first = cache.writer(&Digest::of(b"first"), 0).unwrap();
    first.write(b"fir").unwrap();
    drop(first);
    let second = cache.writer(&Digest::of(b"second"), 0).unwrap();
    drop(second);
    assert_eq!(cache.partial(&Digest::of(b"first")), 0);
    let mut again = cache.writer(&Digest::of(b"second"), 0).unwrap();
    again.write(b"second").unwrap();
    again.finish().unwrap();
    assert!(cache.contains(&Digest::of(b"second")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_least_recently_used_is_let_go_of_first_within_its_bound() {
    log::set_max_level(log::LevelFilter::Trace);
    let root = folder("bounded");
    let cache = Cache::open(&root, 30).unwrap();
    let [a, b, c, d] = [b"aaaaaaaaaa", b"bbbbbbbbbb", b"cccccccccc", b"dddddddddd"];
    for bytes in [a, b, c] {
        keep(&cache, bytes).unwrap();
    }
    assert_eq!(cache.total(), 30);
    // A is used, so B is now the least recently used.
    cache.read(&asset(a), 100).unwrap();
    keep(&cache, d).unwrap();
    assert_eq!(cache.total(), 30);
    assert!(!cache.contains(&Digest::of(b)));
    assert!(!root.join(Digest::of(b).to_string()).exists());
    for kept in [a, c, d] {
        assert!(cache.contains(&Digest::of(kept)));
    }
    // One larger than the cache keeps is refused, and nothing is let go of for it.
    let large = [b'l'; 31];
    let refused = keep(&cache, &large).unwrap_err();
    assert!(
        refused
            .to_string()
            .ends_with("is larger than the cache keeps, 30 bytes")
    );
    assert_eq!(cache.total(), 30);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_order_of_use_survives_a_restart_and_a_smaller_bound_lets_go_of_the_oldest() {
    let root = folder("order");
    let cache = Cache::open(&root, 100).unwrap();
    let [old, new] = [b"an old asset", b"a new asset!"];
    keep(&cache, old).unwrap();
    keep(&cache, new).unwrap();
    drop(cache);
    let at = |bytes: &[u8], seconds| {
        File::options()
            .write(true)
            .open(root.join(Digest::of(bytes).to_string()))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    };
    at(old, 1_000);
    at(new, 2_000);
    // What is not one of its assets is left alone: another file, a digest written otherwise, a folder, a download.
    fs::write(root.join("readme.txt"), b"notes").unwrap();
    let upper = Digest::of(b"upper").to_string().to_uppercase();
    fs::write(root.join(&upper), b"upper").unwrap();
    fs::create_dir(root.join(Digest::of(b"folder").to_string())).unwrap();
    fs::write(
        root.join(format!("{}.part", Digest::of(b"partial"))),
        b"par",
    )
    .unwrap();
    let smaller = Cache::open(&root, 12).unwrap();
    assert!(!smaller.contains(&Digest::of(old)));
    assert!(smaller.contains(&Digest::of(new)));
    assert_eq!(smaller.total(), 12);
    assert!(root.join("readme.txt").exists() && root.join(upper).exists());
    assert_eq!(smaller.partial(&Digest::of(b"partial")), 3);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_library_looks_first_in_a_source_added_first() {
    let first = Library::default();
    first.add(crate::Embedded::new());
    let root = folder("first");
    let cache = Arc::new(Cache::open(&root, 100).unwrap());
    // A bundled copy and a cached copy are the same asset; the cache is asked first.
    keep(&cache, b"bundled").unwrap();
    first.add_first(cache.clone());
    assert_eq!(first.bytes(&asset(b"bundled")).unwrap(), b"bundled");
    fs::remove_file(root.join(Digest::of(b"bundled").to_string())).unwrap();
    first.add(crate::Embedded::new().with(b"bundled"));
    assert_eq!(first.bytes(&asset(b"bundled")).unwrap(), b"bundled");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_cache_whose_folder_cannot_be_made_or_written_says_so() {
    let root = folder("unmade");
    fs::write(&root, b"a file where the folder would be").unwrap();
    assert!(Cache::open(&root, 100).is_err());
    fs::remove_file(&root).unwrap();
    let cache = Cache::open(&root, 100).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert!(cache.writer(&Digest::of(b"anything"), 0).is_err());
    // A download whose folder went before it was kept is not kept.
    fs::create_dir_all(&root).unwrap();
    let mut writer = cache.writer(&Digest::of(b"gone"), 0).unwrap();
    writer.write(b"gone").unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert!(writer.finish().is_err());
    assert!(!cache.contains(&Digest::of(b"gone")));
}
