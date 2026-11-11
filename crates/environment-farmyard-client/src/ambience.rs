// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's ambience, an evening in the New Zealand foothills: wind and distant bush everywhere, the river and the
//! waterfall where the backdrop has them, the campsite's bonfire crackling, and native birds calling now and then from
//! its bush. Each is a slot, its sound
//! `assets/sounds/SLOT.ogg`, which the build finds; a slot without one is silent until one is dropped in. See
//! `design/farmyard.md` and `assets/sounds/readme.md`.
use std::sync::Once;
use std::time::Duration;

use tomori_assets::{AssetRef, Digest, Library};
use tomori_scene::ambience::{Ambience, Bed, Call, Loop, Order, Run, Slot};

use crate::{ID, VERSION, layout};
use tomori_environment_farmyard::campsite;

mod found {
    include!(concat!(env!("OUT_DIR"), "/sounds.rs"));
}

/// Every slot, in the order the ambience declares them: its beds, its loops, and each call's variants.
pub const SLOTS: [&str; 15] = [
    "wind",
    "bush",
    "river",
    "waterfall",
    "fire",
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
pub const LICENSE_TITLE: &str = "The farmyard's sounds, by the Department of Conservation (NZ), eklemp and Freesound's recordists (CC BY 4.0 and CC0 1.0)";

/// How far from the bonfire it is heard, fading out over the last quarter, before the yard, and how loud: a little
/// beneath a −23 LUFS bed at unity as mastered, so that beside it, sitting at the campsite, it crackles a few decibels
/// over the beds and the waterfall beyond, but never over anyone speaking there.
pub const FIRE_RADIUS: f32 = 12.0;
pub const FIRE_GAIN: f32 = -2.0;

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
    // The river runs from where it leaves the pool, over its lip, round the yard's right side; the pool itself is still,
    // and the waterfall is what is heard there.
    let river: Vec<[f32; 3]> = at("river").collect();
    let canopy: Vec<[f32; 3]> = at("bush").map(|[x, y, z]| [x, y + CANOPY, z]).collect();
    let birds = |names: &[&str], every: [u64; 2]| {
        Call::new(names.iter().map(|name| slot(name)).collect())
            .every(Duration::from_secs(every[0]), Duration::from_secs(every[1]))
            .around(&canopy, BUSH_SPREAD)
    };
    let waterfall = at("waterfall").next().unwrap_or_default();
    // The beds are a quiet bed beneath everything. The water and the birds are placed, and lose to distance what the
    // beds do not, about 17 dB for the river 15 m from the yard and 19 dB for the waterfall 26 m away, and 21 dB for
    // a bird in the canopy 20 to 30 m away; so the water is raised to be heard a few decibels beneath the beds from
    // the yard, and the birds to be a little above them. `tests.rs` checks the levels where participants stand.
    Ambience::new()
        .with_bed(Bed::new(slot("wind")).gain(-8.0))
        .with_bed(Bed::new(slot("bush")).gain(-10.0))
        .with_loop(Loop::along(slot("river"), &river).radius(35.0).gain(5.0))
        .with_loop(
            Loop::at(slot("waterfall"), waterfall)
                .radius(60.0)
                .gain(8.0),
        )
        .with_loop(
            Loop::at(slot("fire"), campsite::heart().to_array())
                .radius(FIRE_RADIUS)
                .gain(FIRE_GAIN),
        )
        // A tūī often sings a phrase or two, sometimes three, then pauses, a phrase again as often as another.
        .with_call(
            birds(&["tui-1", "tui-2", "tui-3"], [25, 70])
                .pitch(0.96, 1.04)
                .gain(14.0, 18.0)
                .run(tui()),
        )
        .with_call(
            birds(&["bellbird-1", "bellbird-2", "bellbird-3"], [30, 90])
                .pitch(0.97, 1.03)
                .gain(13.0, 17.0),
        )
        .with_call(
            birds(&["kereru-1", "kereru-2"], [90, 240])
                .pitch(0.9, 1.05)
                .gain(12.0, 17.0),
        )
        // The farmyard is always evening, when the ruru calls, but rarely, as one owl would, several times in a row.
        .with_call(
            birds(&["ruru-1", "ruru-2"], [120, 300])
                .pitch(0.98, 1.02)
                .gain(13.0, 17.0)
                .run(ruru()),
        )
}

/// A ruru's run: usually three to six calls, most often four or five, sometimes just one, 1.5 to 2.5 s apart, its two
/// calls in turn, as one owl's "more-pork" again and again.
fn ruru() -> Run {
    Run::weighted(&[15.0, 5.0, 15.0, 25.0, 25.0, 15.0])
        .gap(Duration::from_millis(1_500), Duration::from_millis(2_500))
}

/// A tūī's run: a phrase, most often, or two, or now and then three, 1 to 3 s apart, any of its phrases, the last too.
fn tui() -> Run {
    Run::weighted(&[60.0, 30.0, 10.0])
        .gap(Duration::from_secs(1), Duration::from_secs(3))
        .order(Order::Random)
}

#[cfg(test)]
mod tests;
