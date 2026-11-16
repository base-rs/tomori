// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// The colours of `people`, by name and the colour each chose.
fn resolved(people: &[(&str, Option<u16>)]) -> Vec<u16> {
    let mut colours = vec![999];
    resolve(people.iter().copied(), &mut colours);
    colours
}

/// The sRGB hue, in degrees, that `colour`, linear, shows.
fn shown_hue(colour: [f32; 3]) -> f32 {
    // Every channel of a person's colour is far above sRGB's linear toe.
    let [red, green, blue] = colour.map(|value| 1.055 * value.powf(1.0 / 2.4) - 0.055);
    let most = red.max(green).max(blue);
    let least = red.min(green).min(blue);
    let chroma = most - least;
    let hue = if most == red {
        ((green - blue) / chroma).rem_euclid(6.0)
    } else if most == green {
        (blue - red) / chroma + 2.0
    } else {
        (red - green) / chroma + 4.0
    };
    hue * 60.0
}

#[test]
fn a_name_is_one_colour_on_every_client() {
    // Pinned, so that a change to the hash, which would recolour everyone, is seen.
    assert_eq!(default("Ada"), default("Ada"));
    assert_eq!(default("Ada"), default("  ada "));
    assert_eq!(default("ADA"), default("ada"));
    assert_eq!(fnv1a(b""), 0x811c_9dc5);
    assert_eq!(fnv1a(b"a"), 0xe40c_292c);
    assert_eq!(fnv1a(b"foobar"), 0xbf9c_f968);
    let pinned: Vec<u16> = ["ada", "bob", "samuel", "ayoko"]
        .iter()
        .map(|name| default(name))
        .collect();
    assert_eq!(pinned, PINNED);
    assert_ne!(default("Ada"), default("Bob"));
}

/// The defaults of Ada, Bob, Samuel and Ayoko, as the hash gives them.
const PINNED: [u16; 4] = [115, 310, 74, 132];

#[test]
fn defaults_are_spread_over_every_arc_and_never_reserved() {
    let mut counts = [0; ARCS.len()];
    for index in 0..2_000 {
        let hue = default(&format!("Person {index}"));
        assert!(clear(hue), "{hue}");
        let arc = ARCS
            .iter()
            .position(|&(first, end)| (first..end).contains(&hue))
            .unwrap();
        counts[arc] += 1;
    }
    // Each arc's share is about its length's.
    let total: u16 = ARCS.iter().map(|(first, end)| end - first).sum();
    for (arc, &(first, end)) in ARCS.iter().enumerate() {
        let expected = 2_000.0 * f32::from(end - first) / f32::from(total);
        let count = counts[arc] as f32;
        assert!(
            (count - expected).abs() < expected * 0.25 + 10.0,
            "{arc}: {count} of {expected}"
        );
    }
}

#[test]
fn the_agent_s_colours_and_the_states_are_no_one_s() {
    // The agent's cyan and violet, a worker running, waiting's amber, a stall's and a failure's red.
    for reserved in [
        0, 6, 10, 30, 39, 42, 50, 180, 193, 197, 218, 240, 268, 275, 358,
    ] {
        assert!(!clear(reserved), "{reserved}");
        assert!(check(reserved).is_err());
    }
    for (_, hue) in SWATCHES {
        assert!(clear(hue));
    }
    // A person's colour, as drawn, shows the hue it is, so that the bands are kept clear in what is seen.
    for hue in (0..DEGREES).step_by(7) {
        let shown = shown_hue(rgb(hue));
        let apart = (shown - f32::from(hue)).abs();
        assert!(apart.min(360.0 - apart) < 1.0, "{hue}: {shown}");
    }
    // The coral everyone was is hue 13.
    let coral = rgb(13);
    for (value, was) in coral.iter().zip([1.0, 0.36, 0.24]) {
        assert!((value - was).abs() < 0.01, "{coral:?}");
    }
}

#[test]
fn every_colour_is_bright_enough_to_glow_and_be_read() {
    for hue in (0..DEGREES).filter(|&hue| clear(hue)) {
        let [red, green, blue] = rgb(hue);
        let luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
        // At least 9 times as bright as the hand menu's backing, about 0.02.
        assert!(luminance > 0.2, "{hue}: {luminance}");
        assert!(
            [red, green, blue]
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
    }
}

#[test]
fn distance_is_the_short_way_round() {
    assert_eq!(distance(10, 350), 20);
    assert_eq!(distance(350, 10), 20);
    assert_eq!(distance(0, 180), 180);
    assert_eq!(distance(90, 90), 0);
}

#[test]
fn nobody_near_another_keeps_their_name_s_colour() {
    let people = [("Ada", None), ("Bob", None)];
    assert!(distance(default("Ada"), default("Bob")) >= APART);
    assert_eq!(resolved(&people), [default("Ada"), default("Bob")]);
    assert!(resolved(&[]).is_empty());
}

#[test]
fn a_later_default_too_near_moves_to_the_nearest_clear_hue_alike_on_every_client() {
    // Two Adas share a default: the first keeps it, and the second moves as little as it can.
    let colours = resolved(&[("Ada", None), ("Ada", None), ("Bob", None)]);
    assert_eq!(colours[0], default("Ada"));
    assert_ne!(colours[1], colours[0]);
    assert!(clear(colours[1]));
    assert!(distance(colours[0], colours[1]) >= APART);
    // Nearest: no clear hue nearer the default is far enough from everyone before it.
    let nearer = (0..DEGREES).filter(|&hue| {
        clear(hue)
            && distance(hue, colours[0]) < distance(colours[1], colours[0])
            && distance(hue, colours[0]) >= APART
    });
    assert_eq!(nearer.count(), 0);
    // The same people in the same order are the same colours, every time.
    assert_eq!(
        colours,
        resolved(&[("Ada", None), ("Ada", None), ("Bob", None)])
    );
    // In the other order, the other moves.
    let swapped = resolved(&[(" ada", None), ("Ada", None)]);
    assert_eq!(swapped[0], default("Ada"));
    assert_eq!(swapped[1], colours[1]);
}

#[test]
fn a_default_moves_higher_first_where_both_ways_are_as_near() {
    assert_eq!(aside(100, &[100]), 130);
    assert_eq!(aside(100, &[100, 130]), 70);
    // Past a reserved band, to the next arc.
    assert_eq!(aside(170, &[170]), 140);
    assert_eq!(aside(20, &[20]), 350);
    assert_eq!(aside(20, &[20, 350]), 54);
}

#[test]
fn a_chosen_colour_always_wins_and_defaults_move_aside_from_it() {
    let ada = default("Ada");
    // Bob chose Ada's default: Bob keeps it, though he comes later, and Ada moves.
    let colours = resolved(&[("Ada", None), ("Bob", Some(ada))]);
    assert_eq!(colours[1], ada);
    assert_ne!(colours[0], ada);
    assert!(distance(colours[0], ada) >= APART);
    // Two who chose one colour both have it: only defaults move.
    assert_eq!(
        resolved(&[("Ada", Some(125)), ("Bob", Some(125))]),
        [125, 125]
    );
    // A chosen colour that is no degree counts as none.
    assert_eq!(resolved(&[("Ada", Some(400))]), [ada]);
}

#[test]
fn a_crowd_is_spread_as_far_apart_as_it_fits() {
    let names: Vec<String> = (0..12).map(|index| format!("Person {index}")).collect();
    let people: Vec<(&str, Option<u16>)> = names.iter().map(|name| (name.as_str(), None)).collect();
    let colours = resolved(&people);
    assert_eq!(colours.len(), 12);
    assert!(colours.iter().all(|&hue| clear(hue)));
    // The first six are kept 30° apart; past what fits, each is as far from the rest as any clear hue is.
    for (index, &hue) in colours.iter().enumerate().take(6) {
        for &other in &colours[..index] {
            assert!(distance(hue, other) >= APART, "{colours:?}");
        }
    }
    for (index, &hue) in colours.iter().enumerate().skip(1) {
        let before = &colours[..index];
        let best = (0..DEGREES)
            .filter(|&hue| clear(hue))
            .map(|hue| clearance(hue, before))
            .max()
            .unwrap();
        assert!(
            clearance(hue, before) >= best.min(APART),
            "{index}: {colours:?}"
        );
    }
    assert_eq!(clearance(10, &[]), 180);
}

#[test]
fn a_colour_is_said_as_a_name_a_hue_or_default() {
    assert_eq!(parse("default"), Ok(None));
    assert_eq!(parse(" Default "), Ok(None));
    assert_eq!(parse(""), Ok(None));
    assert_eq!(parse("Teal"), Ok(Some(165)));
    assert_eq!(parse("pink"), Ok(Some(325)));
    assert_eq!(parse("100"), Ok(Some(100)));
    assert_eq!(parse("100°"), Ok(Some(100)));
    assert!(parse("purple").unwrap_err().contains("teal"));
    assert!(parse("-3").is_err());
    assert!(parse("360").unwrap_err().contains("0 to 359"));
    assert!(parse("240").unwrap_err().contains("kept for the agent"));
    assert_eq!(check(54), Ok(54));
    assert_eq!(name(165), "teal");
    assert_eq!(name(100), "100°");
}
