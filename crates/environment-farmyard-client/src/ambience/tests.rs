// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_scene::ambience::VOICES;

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
    // The river from the pool at the waterfall's foot, along each of its bends.
    assert_eq!(river.path.points()[0], layout::POOL.to_array());
    assert_eq!(river.path.points().len(), layout::RIVER.len());
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
