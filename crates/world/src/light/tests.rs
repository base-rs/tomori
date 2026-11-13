// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Which lights light an instance: by reach, within the budget of eight, the brightest kept and those at the cut
//! fading, in the lights' order, the same each time.
use super::*;
use crate::World;

fn lamp(position: Vec3, reach: f32) -> Light {
    let mut world = World::new();
    Light::new(world.add_root(), position, reach, Vec3::ONE)
}

fn assigned(lights: &[Light], bounds: Option<&Aabb>) -> Vec<Lit> {
    let mut lit = Vec::new();
    assign(lights, bounds, &mut lit);
    lit
}

fn indices(lit: &[Lit]) -> Vec<u32> {
    lit.iter().map(|lit| lit.light).collect()
}

/// A box a metre across at the origin's floor.
fn unit() -> Aabb {
    Aabb::new(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5))
}

#[test]
fn a_light_lights_what_its_reach_meets() {
    let bounds = unit();
    let lights = [
        // Within the box, reaching past it, just reaching its corner, just short of it, and far away.
        lamp(Vec3::new(0.0, 0.5, 0.0), 0.1),
        lamp(Vec3::new(3.0, 0.5, 0.0), 3.0),
        lamp(
            Vec3::new(1.5, 2.0, 1.5),
            1.0f32.hypot(1.0).hypot(1.0) + 0.001,
        ),
        lamp(Vec3::new(3.0, 0.5, 0.0), 2.4),
        lamp(Vec3::new(30.0, 0.5, 0.0), 4.0),
    ];
    let lit = assigned(&lights, Some(&bounds));
    assert_eq!(indices(&lit), [0, 1, 2]);
    assert!(lit.iter().all(|lit| lit.weight == 1.0));
    // A light reaching to within the margin of the box lights it, as rounding might have it reach it, and nothing
    // more than a centimetre beyond.
    let edge = [lamp(Vec3::new(3.0, 0.5, 0.0), 2.495)];
    assert_eq!(indices(&assigned(&edge, Some(&bounds))), [0]);
    let beyond = [lamp(Vec3::new(3.0, 0.5, 0.0), 2.48)];
    assert!(assigned(&beyond, Some(&bounds)).is_empty());
    // One that may be anywhere is lit by every light; an empty box by none; one not all numbers as anywhere.
    assert_eq!(indices(&assigned(&lights, None)), [0, 1, 2, 3, 4]);
    assert!(assigned(&lights, Some(&Aabb::EMPTY)).is_empty());
    let endless = Aabb::new(Vec3::NEG_INFINITY, Vec3::INFINITY);
    assert_eq!(assigned(&lights, Some(&endless)).len(), 5);
    // Nothing lights it that reaches nowhere, gives no light or is nowhere.
    let mut dark = lamp(Vec3::ZERO, 2.0);
    dark.color = Vec3::ZERO;
    let mut dim = lamp(Vec3::ZERO, 2.0);
    dim.intensity = 0.0;
    let nowhere = lamp(Vec3::NAN, 2.0);
    let unreaching = lamp(Vec3::ZERO, 0.0);
    assert!(assigned(&[dark, dim, nowhere, unreaching], Some(&bounds)).is_empty());
    // What it appends follows what was there.
    let mut lit = vec![Lit {
        light: 9,
        weight: 0.5,
    }];
    assign(&lights[..1], Some(&bounds), &mut lit);
    assert_eq!(indices(&lit), [9, 0]);
}

#[test]
fn of_more_than_eight_the_brightest_are_kept_in_the_lights_order_those_at_the_cut_fading() {
    let bounds = unit();
    // Twelve lamps, each a little further, in an order of their own.
    let order = [7, 2, 11, 0, 5, 9, 1, 10, 3, 8, 6, 4];
    let lights: Vec<Light> = order
        .iter()
        .map(|rank| lamp(Vec3::new(0.6 + *rank as f32 * 0.3, 0.5, 0.0), 6.0))
        .collect();
    let lit = assigned(&lights, Some(&bounds));
    assert_eq!(lit.len(), LIGHTS_PER_INSTANCE);
    // The eight nearest, ranks 0 to 7, in the order the lights are.
    let expected: Vec<u32> = (0..lights.len() as u32)
        .filter(|index| order[*index as usize] < 8)
        .collect();
    assert_eq!(indices(&lit), expected);
    // The brightest by far give all their light; the eighth, little brighter than the ninth left out, fades.
    let weight = |rank: usize| {
        let index = order.iter().position(|kept| *kept == rank).unwrap() as u32;
        lit.iter().find(|lit| lit.light == index).unwrap().weight
    };
    assert_eq!(weight(0), 1.0);
    assert!(weight(7) > 0.0 && weight(7) < 1.0, "{}", weight(7));
    assert!(weight(6) >= weight(7));
    // The same each time.
    assert_eq!(assigned(&lights, Some(&bounds)), lit);
}

#[test]
fn a_light_passing_another_at_the_cut_fades_rather_than_flickers() {
    let bounds = unit();
    // Eight lamps close by, and a ninth swept from far to near, past the eighth's brightness.
    let mut lights: Vec<Light> = (0..8)
        .map(|index| lamp(Vec3::new(0.6 + index as f32 * 0.1, 0.5, 0.0), 6.0))
        .collect();
    lights.push(lamp(Vec3::new(5.0, 0.5, 0.0), 6.0));
    let total = |lights: &[Light]| -> f32 {
        assigned(lights, Some(&bounds))
            .iter()
            .map(|lit| lit.weight * lights[lit.light as usize].brightness(0.0))
            .sum()
    };
    let mut last = total(&lights);
    let mut swapped = false;
    for step in 1..=4000 {
        lights[8].position.x = 5.0 - step as f32 * 0.001;
        let now = total(&lights);
        // Each millimetre changes the light given by little, where a light coming or going whole would change it by
        // three quarters.
        assert!((now - last).abs() < 0.02, "step {step}: {last} to {now}");
        last = now;
        swapped |= indices(&assigned(&lights, Some(&bounds))).contains(&8);
    }
    assert!(swapped);
    // Lights equally bright are kept first come, and give nothing, as bright as the one left out.
    let same: Vec<Light> = (0..10)
        .map(|_| lamp(Vec3::new(1.0, 0.5, 0.0), 3.0))
        .collect();
    assert_eq!(
        indices(&assigned(&same, Some(&bounds))),
        [0, 1, 2, 3, 4, 5, 6, 7]
    );
    assert!(
        assigned(&same, Some(&bounds))
            .iter()
            .all(|lit| lit.weight == 0.0)
    );
}

#[test]
fn a_light_is_as_bright_as_its_falloff_says() {
    let mut light = lamp(Vec3::ZERO, 4.0);
    light.color = Vec3::new(1.0, 0.5, 0.25);
    light.intensity = 2.0;
    assert_eq!(light.radiance(), Vec3::new(2.0, 1.0, 0.5));
    let full = light.radiance().dot(Vec3::new(0.2126, 0.7152, 0.0722));
    assert!((light.brightness(0.0) - full).abs() < 1e-6);
    assert!((light.brightness(2.0) - full * 0.25).abs() < 1e-6);
    assert_eq!(light.brightness(4.0), 0.0);
    assert_eq!(light.brightness(5.0), 0.0);
    light.falloff = Falloff::Physical;
    assert!((light.brightness(0.0) - full).abs() < 1e-6);
    let at = |distance: f32| light.brightness(distance);
    assert!(at(1.0) < at(0.5) && at(3.9) < at(1.0) && at(3.9) > 0.0);
    assert_eq!(at(4.0), 0.0);
    assert!(light.gives_light());
    light.reach = -1.0;
    assert!(!light.gives_light());
    assert_eq!(light.brightness(0.0), 0.0);
}

#[test]
fn within_a_large_box_the_lights_nearest_its_centre_are_kept_and_anywhere_the_brightest_whole() {
    // A floor 40 m across with eleven lamps over it, all within it, the nearest its middle kept.
    let floor = Aabb::new(Vec3::new(-20.0, 0.0, -20.0), Vec3::new(20.0, 3.0, 20.0));
    let lights: Vec<Light> = (0..11)
        .map(|index| lamp(Vec3::new(16.0 - index as f32 * 3.0, 2.0, 0.0), 4.0))
        .collect();
    let lit = assigned(&lights, Some(&floor));
    // From x = 16 down by 3 m: the middle's nearest are those from x = 10 down to x = −11, leaving out x = 13 and −14.
    assert_eq!(indices(&lit), [2, 3, 4, 5, 6, 7, 8, 9]);
    assert!(lit.iter().all(|lit| lit.weight > 0.0 && lit.weight <= 1.0));
    // Anywhere, the brightest wherever they are, giving all their light.
    let mut bright = lights.clone();
    bright[10].intensity = 3.0;
    bright[0].intensity = 2.0;
    let lit = assigned(&bright, None);
    assert_eq!(indices(&lit), [0, 1, 2, 3, 4, 5, 6, 10]);
    assert!(lit.iter().all(|lit| lit.weight == 1.0));
}
