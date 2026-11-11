// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_client::playback::spatial::{self, Listener};
use tomori_scene::ambience::{VOICES, gain};

/// How far up a participant's eyes are, standing.
const EYES: f32 = 1.6;
/// How much of a loop's radius the ambience bus fades it over, as its `bus.rs` says.
const FADE: f32 = 0.25;
/// How much louder a mono sound is, heard alike in both ears, than a stereo bed of the same loudness as mastered.
const MONO: f32 = 3.0;

/// Where the levels are heard from: where participants arrive, and at the workbench, at the heart of its curve, both
/// at the eyes.
fn listeners() -> [(&'static str, [f32; 3]); 2] {
    [
        ("arrival", [layout::ARRIVAL.x, EYES, layout::ARRIVAL.z]),
        (
            "workbench",
            [layout::WORKBENCH.x, EYES, layout::WORKBENCH.y],
        ),
    ]
}

fn decibels(factor: f32) -> f32 {
    20.0 * factor.max(1e-9).log10()
}

/// How loud a mono sound at `gain` decibels from `source` is heard at `ear`, by the voices' own law of distance, in
/// decibels as mastered: 0 is a −23 LUFS bed at unity.
fn placed(ear: [f32; 3], source: [f32; 3], gain: f32) -> f32 {
    let listener = Listener {
        position: ear,
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    gain + MONO + decibels(spatial::place(&listener, source, 0.0, 48_000, true).gain[0])
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// A loop heard at `ear`, from the point of its path nearest, fading out at its radius.
fn heard_loop(sound: &Loop, ear: [f32; 3]) -> f32 {
    let nearest = sound.path.nearest(ear);
    let fade = ((sound.radius - distance(nearest, ear)) / (sound.radius * FADE)).clamp(0.0, 1.0);
    placed(ear, nearest, sound.gain) + decibels(fade)
}

/// The beds together, as their powers add.
fn beds(ambience: &Ambience) -> f32 {
    10.0 * ambience
        .beds
        .iter()
        .map(|bed| gain(bed.gain).powi(2))
        .sum::<f32>()
        .log10()
}

/// A call heard at `ear`: from where it is nearest, at the top of its gain; the middle of where it may be, by distance,
/// at the middle of its gain; and furthest, at the bottom, each place in the spread about every point as likely.
fn heard_call(call: &Call, ear: [f32; 3]) -> [f32; 3] {
    let mut distances: Vec<f32> = call
        .around
        .iter()
        .flat_map(|point| {
            (0..64).map(move |step| {
                let reach = call.spread * ((step / 8) as f32 / 7.0).sqrt();
                let angle = (step % 8) as f32 * std::f32::consts::TAU / 8.0;
                [
                    point[0] + reach * angle.cos(),
                    point[1],
                    point[2] + reach * angle.sin(),
                ]
            })
        })
        .map(|source| distance(source, ear))
        .collect();
    distances.sort_by(f32::total_cmp);
    let at = |distance: f32, gain: f32| placed(ear, [ear[0], ear[1] + distance, ear[2]], gain);
    [
        at(distances[0], call.gain[1]),
        at(
            distances[distances.len() / 2],
            (call.gain[0] + call.gain[1]) / 2.0,
        ),
        at(distances[distances.len() - 1], call.gain[0]),
    ]
}

#[test]
fn its_levels_keep_the_beds_quiet_the_water_soft_and_the_birds_a_little_above_them() {
    let ambience = with(&[]);
    let beds = beds(&ambience);
    let [river, waterfall] = &ambience.loops[..] else {
        panic!("a river and a waterfall");
    };
    // Louder at the fence nearest each: the right side's, level with the river's bend, and the back's, before the fall.
    let fence = [
        [layout::FENCE.high.x, EYES, -5.0],
        [-3.0, EYES, layout::FENCE.low.y],
    ];
    let mut table = String::from(
        "| Sound | Arrival | Workbench | Nearest fence |\n| --- | --- | --- | --- |\n",
    );
    let mut row = |name: &str, levels: [String; 3]| {
        table.push_str(&format!(
            "| {name} | {} | {} | {} |\n",
            levels[0], levels[1], levels[2]
        ));
    };
    let level = |decibels: f32| format!("{:.0} LUFS", decibels - 33.0);
    for bed in &ambience.beds {
        row(bed.slot.name(), [0; 3].map(|_| level(bed.gain)));
    }
    row("beds together", [0; 3].map(|_| level(beds)));
    let ears = listeners();
    for ((name, sound), at) in [("river", river), ("waterfall", waterfall)]
        .into_iter()
        .zip(fence)
    {
        let [arrival, workbench] = ears.map(|(_, ear)| level(heard_loop(sound, ear)));
        row(name, [arrival, workbench, level(heard_loop(sound, at))]);
    }
    for (name, call) in ["tūī", "bellbird", "kererū", "ruru"]
        .iter()
        .zip(&ambience.calls)
    {
        let [arrival, workbench] = ears.map(|(_, ear)| {
            let [nearest, middle, furthest] = heard_call(call, ear);
            format!(
                "{} (nearest {}, furthest {})",
                level(middle),
                level(nearest),
                level(furthest)
            )
        });
        row(name, [arrival, workbench, String::from("as in the yard")]);
    }
    // At the default level, low, 10 dB beneath as mastered, each sound as heard over its own length, before ducking.
    println!("{table}");
    for (_, ear) in ears {
        // The water is heard, but beneath the beds from the yard.
        for sound in [river, waterfall] {
            let heard = heard_loop(sound, ear);
            assert!(
                heard < beds - 1.0 && heard > beds - 10.0,
                "{heard} beside {beds}"
            );
        }
        // The birds a little above the beds, as they usually are, never startling nearby, and heard however far.
        for call in &ambience.calls {
            let [nearest, middle, furthest] = heard_call(call, ear);
            assert!(
                middle > beds && middle < beds + 6.0,
                "{middle} beside {beds}"
            );
            assert!(nearest < beds + 12.0, "{nearest} beside {beds}");
            assert!(furthest > beds - 8.0, "{furthest} beside {beds}");
        }
    }
    // Louder at the fence than from the arrival, but still not above the beds.
    for (sound, ear) in [river, waterfall].into_iter().zip(fence) {
        let heard = heard_loop(sound, ear);
        assert!(heard > heard_loop(sound, listeners()[0].1) + 2.0);
        assert!(heard < beds + 1.0, "{heard} beside {beds}");
    }
}

#[test]
fn the_farmyard_declares_every_slot_each_silent_until_its_sound_is_found() {
    let silent = with(&[]);
    assert_eq!(silent.slots().map(Slot::name).collect::<Vec<_>>(), SLOTS);
    assert!(silent.slots().all(Slot::is_missing));
    // Each sound the build found is one of its slots, a slot of its own first, so that the check runs however many
    // are found.
    assert!(
        std::iter::once(("wind", ""))
            .chain(files().iter().copied())
            .all(|(slot, _)| SLOTS.contains(&slot))
    );
    assert_eq!(ambience().slots().count(), SLOTS.len());
}

#[test]
fn a_sound_found_is_loaded_from_the_program_s_library_by_its_digest() {
    const RIVER: &[u8] = b"the river's sound, as a test bundles it";
    let river = Digest::of(RIVER).to_string();
    let absent = Digest::of(b"a sound no source has").to_string();
    Library::shared().add(embedded(&[RIVER]));
    let found = [
        ("river", river.as_str()),
        ("waterfall", absent.as_str()),
        ("wind", "not a digest"),
    ];
    let ambience = with(&found);
    let slots: Vec<&Slot> = ambience.slots().collect();
    let named = |name: &str| *slots.iter().find(|slot| slot.name() == name).unwrap();
    // Found, it loads; found but in no source, it is silent, saying why; and a digest that cannot be read is none.
    assert!(!named("river").is_missing());
    assert!(!named("waterfall").is_missing());
    assert!(named("wind").is_missing());
    assert!(named("bush").is_missing());
    assert_eq!(named("river").load().as_deref(), Some(RIVER));
    assert_eq!(named("waterfall").load(), None);
    assert_eq!(
        asset("river", &river).unwrap().path(),
        "farmyard/sounds/river.ogg"
    );
    bundle();
    bundle();
}

#[test]
fn its_sounds_are_where_the_backdrop_has_them_and_leave_voices_for_the_birds() {
    let ambience = with(&[]);
    assert_eq!(ambience.beds.len(), 2);
    assert!(ambience.beds.iter().all(|bed| bed.gain < 0.0));
    let [river, waterfall] = &ambience.loops[..] else {
        panic!("a river and a waterfall");
    };
    // The river from where it leaves the pool, over its lip, along each of its bends.
    assert_eq!(river.path.points()[0], layout::RIVER[1].to_array());
    assert_eq!(river.path.points().len(), layout::RIVER.len() - 1);
    assert_eq!(
        river.path.points().last().copied(),
        layout::RIVER.last().map(|point| point.to_array())
    );
    // Heard from the arrival, about 15 m from the river and 26 m from the waterfall, and fading out before the far
    // side of the yard from each.
    let arrival = layout::ARRIVAL.to_array();
    let distance = |a: [f32; 3], b: [f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    assert!(distance(river.path.nearest(arrival), arrival) < river.radius * 0.75);
    assert_eq!(waterfall.path.points(), [layout::WATERFALL.to_array()]);
    assert!(distance(layout::WATERFALL.to_array(), arrival) < waterfall.radius * 0.75);
    // Four kinds of bird, each with variants, from the canopy of every patch of bush, with voices left for them.
    assert_eq!(ambience.calls.len(), 4);
    for call in &ambience.calls {
        assert!(call.variants.len() >= 2);
        assert_eq!(call.around.len(), layout::BUSH.len());
        assert!(
            call.around
                .iter()
                .zip(layout::BUSH)
                .all(|(point, (middle, _))| {
                    point[0] == middle.x && point[2] == middle.y && point[1] > CANOPY - 1.0
                })
        );
        assert_eq!(call.spread, BUSH_SPREAD);
        assert!(call.every[0] >= Duration::from_secs(25));
        assert!(call.pitch[0] < 1.0 && call.pitch[1] > 1.0);
    }
    assert!(ambience.beds.len() + ambience.loops.len() + 4 <= VOICES);
}

#[test]
fn every_slot_has_its_sound_credited_and_within_the_limits() {
    // Every slot's sound is found, and every sound found is a slot's.
    let mut slots = SLOTS.to_vec();
    slots.sort_unstable();
    assert_eq!(
        files().iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
        slots
    );
    bundle();
    let ambience = ambience();
    assert!(ambience.slots().all(|slot| !slot.is_missing()));
    let mut together = 0;
    for slot in ambience.slots() {
        // Each loads from the program's library, Ogg Vorbis within the largest a slot plays, credited by name.
        let bytes = slot.load().unwrap();
        assert!(bytes.starts_with(b"OggS"), "{}", slot.name());
        assert_eq!(&bytes[28..35], b"\x01vorbis", "{}", slot.name());
        assert!(bytes.len() <= tomori_scene::ambience::MOST_BYTES);
        assert!(
            LICENSE.contains(&format!("`{}.ogg`", slot.name())),
            "{} is not in LICENSE.md",
            slot.name()
        );
        together += bytes.len();
    }
    assert!(together <= 6 << 20);
    // Nothing else is in the folder but the sounds, their notice and their readme.
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sounds");
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.ends_with(".ogg"))
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["LICENSE.md", "readme.md"]);
}

#[test]
fn the_waterfall_is_loud_on_the_rock_before_it_as_its_place_makes_it() {
    let ambience = with(&[]);
    let beds = beds(&ambience);
    let [river, waterfall] = &ambience.loops[..] else {
        panic!("a river and a waterfall");
    };
    // Standing on the waterfall rock, its spray about them, and at the lookout over the pool, at the eyes.
    let rock = (layout::WATERFALL_ROCK + glam::Vec3::Y * EYES).to_array();
    let (lookout, _, _) = crate::trail::frame(crate::trail::VIEWPOINTS[1].2);
    let lookout = [lookout.x, EYES, lookout.y];
    let (on_rock, at_lookout) = (heard_loop(waterfall, rock), heard_loop(waterfall, lookout));
    println!("waterfall: rock {on_rock:.1}, lookout {at_lookout:.1}, beds {beds:.1}");
    // Nothing but where it is makes it so: on the rock, well within its radius, it is far louder than at the lookout
    // and above the beds, which it was beneath from the yard, and the river, which leaves the pool beyond it.
    assert!(distance(layout::WATERFALL.to_array(), rock) < waterfall.radius * 0.5);
    assert!(on_rock > at_lookout + 8.0, "{on_rock} {at_lookout}");
    assert!(on_rock > beds + 6.0, "{on_rock} {beds}");
    assert!(on_rock > heard_loop(river, rock), "{on_rock}");
}
