// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::Environment;
use tomori_assets::Embedded;

/// Bytes that start as `kind`'s must, `length` long in all, leaked so that an embedded source can hold them.
pub(crate) fn content(kind: ContentKind, tag: &str, length: usize) -> &'static [u8] {
    let signature: &[u8] = if kind == ContentKind::Model {
        b"glTF"
    } else if kind == ContentKind::Image {
        b"\x89PNG\r\n\x1a\n"
    } else if kind == ContentKind::Texture {
        b"\xabKTX 20\xbb\r\n\x1a\n"
    } else {
        b"OggS"
    };
    let mut bytes = signature.to_vec();
    bytes.extend_from_slice(tag.as_bytes());
    bytes.resize(length.max(bytes.len()), b'.');
    Box::leak(bytes.into_boxed_slice())
}

fn served(environment: &str, name: &str, kind: ContentKind, bytes: &[u8]) -> ServedAsset {
    ServedAsset::new(AssetRef::new(environment, 1, name, Digest::of(bytes)), kind)
}

fn presenting(environments: &[(&str, u32)]) -> Environments {
    Environments::new(
        environments
            .iter()
            .map(|(id, version)| Environment::new(*id, *id).with_version(*version)),
    )
}

#[test]
fn the_store_keeps_and_offers_what_each_environment_declares_as_it_should_be() {
    log::set_max_level(log::LevelFilter::Trace);
    let lantern = content(ContentKind::Model, "lantern", 64);
    let chime = content(ContentKind::Sound, "chime", 32);
    let leaf = content(ContentKind::Image, "leaf", 40);
    let moss = content(ContentKind::Texture, "moss", 48);
    let library = Library::default();
    library.add(
        Embedded::new()
            .with(lantern)
            .with(chime)
            .with(leaf)
            .with(moss),
    );
    let garden = Environment::new("garden", "Garden")
        .with_asset(
            served("garden", "lantern.glb", ContentKind::Model, lantern)
                .with_licence("by Ada, CC BY 4.0"),
        )
        .with_asset(served("garden", "chime.ogg", ContentKind::Sound, chime));
    let pond = Environment::new("pond", "Pond")
        .with_version(2)
        // Shared with the garden, it is kept once.
        .with_asset(served("pond", "lantern.glb", ContentKind::Model, lantern))
        .with_asset(served("pond", "leaf.png", ContentKind::Image, leaf))
        .with_asset(served("pond", "moss.ktx2", ContentKind::Texture, moss));
    let store = ContentStore::default();
    store.register(&Environments::new([garden.clone(), pond.clone()]), &library);
    let offered = store.offered(&presenting(&[("garden", 1)]));
    assert_eq!(
        offered,
        [
            OfferedAsset {
                environment: "garden".to_owned(),
                version: 1,
                name: "lantern.glb".to_owned(),
                digest: Digest::of(lantern).to_string(),
                length: 64,
                kind: ContentKind::Model,
                licence: Some("by Ada, CC BY 4.0".to_owned()),
            },
            OfferedAsset {
                environment: "garden".to_owned(),
                version: 1,
                name: "chime.ogg".to_owned(),
                digest: Digest::of(chime).to_string(),
                length: 32,
                kind: ContentKind::Sound,
                licence: None,
            },
        ]
    );
    // Only what an environment presented at its version declares is found.
    let (bytes, kind) = store
        .find(&Digest::of(chime), &presenting(&[("garden", 1)]))
        .unwrap();
    assert_eq!((&*bytes, kind), (chime, ContentKind::Sound));
    assert!(
        store
            .find(&Digest::of(leaf), &presenting(&[("garden", 1)]))
            .is_none()
    );
    assert!(
        store
            .find(&Digest::of(leaf), &presenting(&[("pond", 1)]))
            .is_none()
    );
    assert!(
        store
            .find(&Digest::of(leaf), &presenting(&[("pond", 2)]))
            .is_some()
    );
    assert!(
        store
            .find(&Digest::of(b"never kept"), &presenting(&[("pond", 2)]))
            .is_none()
    );
    assert_eq!(store.offered(&presenting(&[("pond", 2)])).len(), 3);
    let (bytes, kind) = store
        .find(&Digest::of(moss), &presenting(&[("pond", 2)]))
        .unwrap();
    assert_eq!((&*bytes, kind), (moss, ContentKind::Texture));
    // Registered again, what is kept is not read again, and what no environment declares now is let go.
    let empty = Library::default();
    store.register(&Environments::new([pond]), &empty);
    assert!(
        store
            .find(&Digest::of(leaf), &presenting(&[("pond", 2)]))
            .is_some()
    );
    let kept = store
        .inner
        .read()
        .unwrap()
        .kept
        .contains_key(&Digest::of(chime));
    assert!(!kept);
    store.register(&Environments::new([garden]), &empty);
    assert_eq!(store.offered(&presenting(&[("garden", 1)])).len(), 1);
}

#[test]
fn an_asset_is_served_only_as_its_digest_kind_bounds_name_and_licence_allow() {
    crate::test_log::record();
    let lantern = content(ContentKind::Model, "a lantern for refusal", 64);
    let large = content(ContentKind::Sound, "large", (1 << 20) + 1);
    let mural = content(ContentKind::Texture, "a mural too large", (16 << 20) + 1);
    let library = Library::default();
    library.add(Embedded::new().with(lantern).with(large).with(mural));
    let declared = |name: &str, kind, bytes| served("refusing", name, kind, bytes);
    let refusing = Environment::new("refusing", "Refusing")
        // Not in the library.
        .with_asset(declared("missing.glb", ContentKind::Model, b"glTF missing"))
        // Not what its kind must start as.
        .with_asset(declared("lantern.png", ContentKind::Image, lantern))
        .with_asset(declared("lantern.ktx2", ContentKind::Texture, lantern))
        // Larger than its kind may be.
        .with_asset(declared("large.ogg", ContentKind::Sound, large))
        .with_asset(declared("mural.ktx2", ContentKind::Texture, mural))
        // A name too long, or none.
        .with_asset(declared(
            &"n".repeat(NAME_BYTES + 1),
            ContentKind::Model,
            lantern,
        ))
        .with_asset(declared("", ContentKind::Model, lantern))
        // A licence too long.
        .with_asset(
            declared("licensed.glb", ContentKind::Model, lantern)
                .with_licence("l".repeat(LICENCE_BYTES + 1)),
        );
    let watch = crate::test_log::watch("of the environment refusing is not served");
    let store = ContentStore::default();
    store.register(&Environments::new([refusing]), &library);
    assert!(store.offered(&presenting(&[("refusing", 1)])).is_empty());
    let warnings: Vec<String> = watch.records().into_iter().map(|(_, text)| text).collect();
    for reason in [
        "missing.glb of the environment refusing is not served: refusing/missing.glb is in none of the asset sources",
        "lantern.png of the environment refusing is not served: it is not image/png",
        "large.ogg of the environment refusing is not served: it is 1048577 bytes, more than the 1048576 that audio/ogg may be",
        "lantern.ktx2 of the environment refusing is not served: it is not image/ktx2",
        "mural.ktx2 of the environment refusing is not served: it is 16777217 bytes, more than the 16777216 that image/ktx2 may be",
        "refusing/ of the environment refusing is not served: its name is empty or longer than 256 bytes",
        "nnn of the environment refusing is not served: its name is empty or longer than 256 bytes",
        "licensed.glb of the environment refusing is not served: its licence notice is longer than 512 bytes",
    ] {
        assert!(
            warnings.iter().any(|warning| warning.contains(reason)),
            "{reason:?} in {warnings:?}"
        );
    }
    // Kept as one kind, it is not offered as another, the first declaration winning.
    let twice = Environment::new("twice", "Twice")
        .with_asset(declared("lantern.glb", ContentKind::Model, lantern))
        .with_asset(declared("lantern.ogg", ContentKind::Sound, lantern));
    let watch = crate::test_log::watch("lantern.ogg of the environment twice");
    store.register(&Environments::new([twice]), &library);
    assert_eq!(store.offered(&presenting(&[("twice", 1)])).len(), 1);
    assert!(
        watch.records()[0]
            .1
            .ends_with("it is kept as model/gltf-binary already.")
    );
}

#[test]
fn the_store_holds_at_most_its_bound() {
    crate::test_log::record();
    // Enough models of the largest size to pass the bound.
    let count = (STORE_BYTES / ContentKind::Model.most_bytes()) as usize + 1;
    let models: Vec<&'static [u8]> = (0..count)
        .map(|index| content(ContentKind::Model, &format!("model {index}"), 16 << 20))
        .collect();
    let library = Library::default();
    let mut bounded = Environment::new("bounded", "Bounded");
    for (index, model) in models.iter().enumerate() {
        library.add(Embedded::new().with(model));
        bounded = bounded.with_asset(served(
            "bounded",
            &format!("{index}.glb"),
            ContentKind::Model,
            model,
        ));
    }
    let watch = crate::test_log::watch("of the environment bounded is not served");
    let store = ContentStore::default();
    store.register(&Environments::new([bounded]), &library);
    assert_eq!(
        store.offered(&presenting(&[("bounded", 1)])).len(),
        count - 1
    );
    assert!(watch.records()[0].1.ends_with(&format!(
        "the content store would hold more than {STORE_BYTES} bytes."
    )));
}

#[test]
fn a_connection_has_at_most_its_open_requests() {
    let open = Arc::new(AtomicUsize::new(0));
    let permits: Vec<Permit> = (0..OPEN_REQUESTS)
        .map(|_| Permit::take(&open).unwrap())
        .collect();
    assert!(Permit::take(&open).is_none());
    drop(permits);
    assert_eq!(open.load(Ordering::Acquire), 0);
    assert!(Permit::take(&open).is_some());
}

#[test]
fn a_range_is_one_range_of_bytes_or_else_the_whole() {
    let of = |field: &str| Range::of(Some(field), 100);
    assert_eq!(Range::of(None, 100), Range::Whole);
    assert_eq!(of("bytes=0-"), Range::Part(0, 100));
    assert_eq!(of("bytes=10-19"), Range::Part(10, 20));
    assert_eq!(of(" bytes= 10 - 19 "), Range::Part(10, 20));
    assert_eq!(of("bytes=90-200"), Range::Part(90, 100));
    assert_eq!(of(&format!("bytes=90-{}", u64::MAX)), Range::Part(90, 100));
    assert_eq!(of("bytes=-10"), Range::Part(90, 100));
    assert_eq!(of("bytes=-1000"), Range::Part(0, 100));
    assert_eq!(of("bytes=100-"), Range::Unsatisfiable);
    assert_eq!(of("bytes=-0"), Range::Unsatisfiable);
    assert_eq!(Range::of(Some("bytes=-5"), 0), Range::Unsatisfiable);
    // Anything else is answered whole: another unit, several ranges, a reversed range, signs and nonsense.
    for whole in [
        "items=0-1",
        "bytes=0-1,5-6",
        "bytes=20-10",
        "bytes=+1-2",
        "bytes=-",
        "bytes=5",
        "bytes=99999999999999999999999-",
    ] {
        assert_eq!(of(whole), Range::Whole, "{whole}");
    }
}
