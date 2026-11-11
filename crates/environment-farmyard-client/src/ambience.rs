// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's ambience, an evening in the New Zealand foothills: wind and distant bush everywhere, the river and the
//! waterfall where the backdrop has them, and native birds calling now and then from its bush. Each is a slot, its sound
//! `assets/sounds/SLOT.ogg`, which the build finds; a slot without one is silent until one is dropped in. See
//! `design/farmyard.md` and `assets/sounds/readme.md`.
use std::sync::Once;
use std::time::Duration;

use tomori_assets::{AssetRef, Digest, Library};
use tomori_scene::ambience::{Ambience, Bed, Call, Loop, Slot};

use crate::{ID, VERSION, layout};

mod found {
    include!(concat!(env!("OUT_DIR"), "/sounds.rs"));
}

/// Every slot, in the order the ambience declares them: its beds, its loops, and each call's variants.
pub const SLOTS: [&str; 14] = [
    "wind",
    "bush",
    "river",
    "waterfall",
    "tui-1",
    "tui-2",
    "tui-3",
    "bellbird-1",
    "bellbird-2",
    "bellbird-3",
    "kereru-1",
    "kereru-2",
    "ruru-1",
    "ruru-2",
];

/// The licence notice of the farmyard's sounds, which says where each came from.
pub const LICENSE: &str = include_str!("../assets/sounds/LICENSE.md");
/// The heading it is shown under with a build's other bundled assets'.
pub const LICENSE_TITLE: &str = "The farmyard's sounds";

/// How far above the ground of its bush a bird calls from, in the canopy.
const CANOPY: f32 = 6.0;
/// How far across the ground of its middle a bird of a patch of bush calls from.
const BUSH_SPREAD: f32 = 5.0;

/// The asset of the sound `slot` whose digest is `digest`, as the APK carries it at `farmyard/sounds/SLOT.ogg`.
fn asset(slot: &str, digest: &str) -> Option<AssetRef> {
    Some(AssetRef::new(
        ID,
        VERSION,
        format!("sounds/{slot}.ogg"),
        Digest::parse(digest)?,
    ))
}

/// The slot `name`, its sound loaded from the program's library by the digest `found` gives it, or silent.
fn slot(name: &str, found: &[(&str, &str)]) -> Slot {
    let asset = found
        .iter()
        .find(|(slot, _)| *slot == name)
        .and_then(|(slot, digest)| asset(slot, digest));
    match asset {
        Some(asset) => Slot::new(name, move || match Library::shared().bytes(&asset) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                log::warn!("Ambience: {error}");
                None
            }
        }),
        None => Slot::missing(name),
    }
}

/// An embedded source of `sounds`, which Android's APK carries instead.
#[cfg(any(test, not(target_os = "android")))]
fn embedded(sounds: &[&'static [u8]]) -> tomori_assets::Embedded {
    sounds
        .iter()
        .fold(tomori_assets::Embedded::new(), |embedded, bytes| {
            embedded.with(bytes)
        })
}

/// Add the farmyard's embedded sounds to the program's library, once.
pub fn bundle() {
    static BUNDLED: Once = Once::new();
    BUNDLED.call_once(|| {
        #[cfg(not(target_os = "android"))]
        Library::shared().add(embedded(found::EMBEDDED));
    });
}

/// Every sound the build found in `assets/sounds`, by slot, with its digest.
pub fn files() -> &'static [(&'static str, &'static str)] {
    found::FOUND
}

/// The farmyard's ambience, in its own coordinates, with the sounds the build found.
pub fn ambience() -> Ambience {
    with(found::FOUND)
}

/// The farmyard's ambience with the sounds `found` names.
fn with(found: &[(&str, &str)]) -> Ambience {
    let slot = |name: &str| slot(name, found);
    let sounds = layout::sounds();
    let at = |name: &'static str| {
        sounds
            .iter()
            .filter(move |(sound, _)| *sound == name)
            .map(|(_, place)| place.to_array())
    };
    // The river runs from the pool at the waterfall's foot, round the yard's right side.
    let river: Vec<[f32; 3]> = at("pool").chain(at("river")).collect();
    let canopy: Vec<[f32; 3]> = at("bush").map(|[x, y, z]| [x, y + CANOPY, z]).collect();
    let birds = |names: &[&str], every: [u64; 2]| {
        Call::new(names.iter().map(|name| slot(name)).collect())
            .every(Duration::from_secs(every[0]), Duration::from_secs(every[1]))
            .around(&canopy, BUSH_SPREAD)
    };
    let waterfall = at("waterfall").next().unwrap_or_default();
    Ambience::new()
        .with_bed(Bed::new(slot("wind")).gain(-6.0))
        .with_bed(Bed::new(slot("bush")).gain(-8.0))
        .with_loop(Loop::along(slot("river"), &river).radius(35.0).gain(-2.0))
        .with_loop(Loop::at(slot("waterfall"), waterfall).radius(60.0))
        .with_call(
            birds(&["tui-1", "tui-2", "tui-3"], [25, 70])
                .pitch(0.96, 1.04)
                .gain(-4.0, 0.0),
        )
        .with_call(
            birds(&["bellbird-1", "bellbird-2", "bellbird-3"], [30, 90])
                .pitch(0.97, 1.03)
                .gain(-6.0, -1.0),
        )
        .with_call(
            birds(&["kereru-1", "kereru-2"], [90, 240])
                .pitch(0.9, 1.05)
                .gain(-6.0, 0.0),
        )
        .with_call(
            birds(&["ruru-1", "ruru-2"], [60, 180])
                .pitch(0.98, 1.02)
                .gain(-6.0, -2.0),
        )
}

#[cfg(test)]
mod tests;
