// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The animator: its graph's checks, transitions on parameters, triggers and finished clips, cross-fades, events fired
//! exactly once however time is divided, root motion over loops, and the same state whichever way it is advanced.
use std::sync::Arc;

use glam::{Quat, Vec3, Vec4};

use super::*;
use crate::model::{Animation, Channel, Interpolation, Limits, Node, Property, Transform};

/// A model with one node, `body`, and two animations of it: `stride`, which moves it 1 m along X a second, and `turn`,
/// which turns it a quarter turn over a second.
fn model() -> Model {
    let channel = |property, values: Vec<Vec4>| Channel {
        node: 0,
        property,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 1.0],
        values,
    };
    Model {
        nodes: vec![Node::new("body", Transform::IDENTITY)],
        roots: vec![0],
        animations: vec![
            Animation {
                name: Some("stride".into()),
                channels: vec![channel(Property::Translation, vec![Vec4::ZERO, Vec4::X])],
                duration: 1.0,
            },
            Animation {
                name: Some("turn".into()),
                channels: vec![channel(
                    Property::Rotation,
                    vec![
                        Vec4::from(Quat::IDENTITY),
                        Vec4::from(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                    ],
                )],
                duration: 1.0,
            },
        ],
        ..Model::default()
    }
    .checked(&Limits::default())
    .unwrap()
}

/// Stands, walks while `walking`, startles on `startle` into a sprint played once, and stands again when it finishes.
fn graph() -> Arc<Graph> {
    Arc::new(
        Graph::builder()
            .state("stand", Clip::new(1, 0.0, 1.0).looping())
            .state(
                "walk",
                Clip::new(0, 0.0, 1.0)
                    .looping()
                    .with_event(0.0, "left")
                    .with_event(0.5, "right")
                    .with_motion(RootMotion::new(0.5, vec![0.0, 0.25, 1.0])),
            )
            .state(
                "sprint",
                Clip::new(0, 0.0, 1.0)
                    .at_speed(2.0)
                    .with_event(1.0, "landed")
                    .with_motion(RootMotion::new(1.0, vec![0.0, 2.0])),
            )
            .bool("walking", false)
            .float("fear", 0.0)
            .trigger("startle")
            .transition("stand", "walk", [Condition::Is("walking", true)], 0.0)
            .transition("walk", "stand", [Condition::Is("walking", false)], 0.25)
            .from_any(
                "sprint",
                [
                    Condition::Triggered("startle"),
                    Condition::Above("fear", 0.5),
                ],
                0.0,
            )
            .transition("sprint", "stand", [Condition::Finished], 0.5)
            .transition("stand", "stand", [Condition::Below("fear", -1.0)], 0.0)
            .build()
            .unwrap(),
    )
}

#[test]
fn a_graph_names_what_it_uses_once() {
    let clip = || Clip::new(0, 0.0, 1.0);
    let refused = |builder: Builder| builder.build().unwrap_err();
    assert_eq!(refused(Graph::builder()), "an animator needs a state");
    assert_eq!(
        refused(Graph::builder().state("a", clip()).state("a", clip())),
        "the state a is named twice"
    );
    assert_eq!(
        refused(Graph::builder().state("a", Clip::new(0, 1.0, 1.0))),
        "the state a's clip does not play"
    );
    assert_eq!(
        refused(Graph::builder().state("a", clip().at_speed(0.0))),
        "the state a's clip does not play"
    );
    assert_eq!(
        refused(
            Graph::builder()
                .state("a", clip())
                .bool("x", false)
                .float("x", 0.0)
        ),
        "the parameter x is named twice"
    );
    assert_eq!(
        refused(
            Graph::builder()
                .state("a", clip())
                .transition("a", "b", [], 0.0)
        ),
        "there is no state b"
    );
    assert_eq!(
        refused(
            Graph::builder()
                .state("a", clip())
                .transition("b", "a", [], 0.0)
        ),
        "there is no state b"
    );
    for condition in [
        Condition::Is("x", true),
        Condition::Above("x", 0.0),
        Condition::Below("z", 0.0),
        Condition::Triggered("x"),
    ] {
        assert_eq!(
            refused(
                Graph::builder()
                    .state("a", clip())
                    .float("y", 0.0)
                    .bool("z", false)
                    .transition("a", "a", [condition], 0.0)
            )[..21]
                .to_owned(),
            "there is no parameter"
        );
    }
    let graph = graph();
    assert_eq!(graph.states(), ["stand", "walk", "sprint"]);
    assert_eq!(graph.clip("sprint").unwrap().duration(), 0.5);
    assert_eq!(graph.clip("none"), None);
}

#[test]
fn parameters_move_it_between_states_at_the_moments_they_are_set() {
    let mut animator = Animator::new(graph(), 10.0);
    assert_eq!(
        (animator.state(), animator.entered(), animator.now()),
        ("stand", 10.0, 10.0)
    );
    // A parameter of another kind, or none, is refused.
    assert!(!animator.set("walking", Value::Float(1.0), 10.0));
    assert!(!animator.set("none", Value::Bool(true), 10.0));
    // Set ahead, it applies at its moment, however far past it the animator is advanced.
    assert!(animator.set("walking", Value::Bool(true), 11.0));
    animator.advance(10.5);
    assert_eq!(animator.state(), "stand");
    animator.advance(12.0);
    assert_eq!((animator.state(), animator.entered()), ("walk", 11.0));
    assert_eq!(animator.value("walking"), Some(Value::Bool(true)));
    assert_eq!(animator.value("none"), None);
    // Set for a moment that has passed, it applies now.
    animator.set("walking", Value::Bool(false), 0.0);
    assert_eq!((animator.state(), animator.entered()), ("stand", 12.0));
    // Fading, it waits until the fade has finished to move on, and then does.
    animator.set("walking", Value::Bool(true), 12.1);
    animator.advance(12.2);
    assert_eq!(animator.state(), "stand");
    animator.advance(12.3);
    assert_eq!((animator.state(), animator.entered()), ("walk", 12.25));
    // A trigger with a float above its limit: from any state, once, its trigger taken.
    animator.set("fear", Value::Float(1.0), 13.0);
    animator.set("startle", Value::Trigger(true), 13.0);
    animator.advance(12.9);
    assert_eq!(animator.state(), "walk");
    animator.advance(13.0);
    assert_eq!((animator.state(), animator.entered()), ("sprint", 13.0));
    assert_eq!(animator.value("startle"), Some(Value::Trigger(false)));
    // Played once, it finishes after half a second at twice its speed, and stands again, fading into it.
    animator.advance(13.9);
    assert_eq!((animator.state(), animator.entered()), ("stand", 13.5));
    animator.set("walking", Value::Bool(false), 13.9);
    animator.advance(14.0);
    assert_eq!((animator.state(), animator.entered()), ("stand", 13.5));
    // A transition that leads back to its own state is taken at most so many times at one moment.
    animator.set("fear", Value::Float(-2.0), 14.0);
    assert_eq!(animator.entered(), 14.0);
    // A moment before now changes nothing.
    assert_eq!(animator.advance(1.0), Advance::default());
}

#[test]
fn a_cross_fade_blends_from_the_pose_it_was_in() {
    let model = model();
    let mut animator = Animator::new(graph(), 0.0);
    animator.set("walking", Value::Bool(true), 0.0);
    animator.advance(0.5);
    assert_eq!(animator.position(), 0.5);
    assert_eq!(animator.pose(&model).local(0).translation, Vec3::X * 0.5);
    animator.set("walking", Value::Bool(false), 0.5);
    assert_eq!(animator.fading(), Some(("walk", 0.5, 1.0)));
    animator.advance(0.625);
    let (state, position, left) = animator.fading().unwrap();
    assert_eq!((state, position), ("walk", 0.5));
    assert!((left - 0.5).abs() < 1e-6);
    // Half of the walk's pose, frozen as it was, and half of standing, a little turned.
    let pose = animator.pose(&model);
    assert!((pose.local(0).translation.x - 0.25).abs() < 1e-6);
    assert!(pose.local(0).rotation.angle_between(Quat::IDENTITY) > 0.01);
    animator.advance(1.0);
    assert_eq!(animator.fading(), None);
    assert_eq!(animator.pose(&model).local(0).translation, Vec3::ZERO);
}

#[test]
fn events_fire_once_on_every_loop_however_time_is_divided() {
    let events = |steps: &[f64]| {
        let mut animator = Animator::new(graph(), 0.0);
        animator.set("walking", Value::Bool(true), 0.0);
        let mut fired = Vec::new();
        for step in steps {
            fired.extend(animator.advance(*step).events);
        }
        fired
            .into_iter()
            .map(|event| (event.name, event.state, event.at))
            .collect::<Vec<_>>()
    };
    let all = events(&[2.25]);
    assert_eq!(
        all,
        [
            ("left", "walk", 0.0),
            ("right", "walk", 0.5),
            ("left", "walk", 1.0),
            ("right", "walk", 1.5),
            ("left", "walk", 2.0)
        ]
    );
    // In frames that fall anywhere, on an event or across a loop, each still fires once.
    let frames: Vec<f64> = (1..=27).map(|frame| f64::from(frame) / 12.0).collect();
    assert_eq!(events(&frames), all[..]);
    assert_eq!(events(&[0.5, 0.5, 1.0, 2.25]), all);
    // An event at a clip's end fires as it finishes, once.
    let mut animator = Animator::new(graph(), 0.0);
    animator.set("startle", Value::Trigger(true), 0.0);
    animator.set("fear", Value::Float(1.0), 0.0);
    let fired: Vec<_> = (1..=10)
        .flat_map(|frame| animator.advance(f64::from(frame) / 10.0).events)
        .map(|event| (event.name, event.at))
        .collect();
    assert_eq!(fired, [("landed", 0.5)]);
}

#[test]
fn root_motion_accumulates_over_loops_and_stops_with_the_clip() {
    let moved = |steps: &[f64]| {
        let mut animator = Animator::new(graph(), 0.0);
        animator.set("walking", Value::Bool(true), 0.0);
        animator.set("walking", Value::Bool(false), 2.25);
        steps
            .iter()
            .map(|step| animator.advance(*step).moved)
            .sum::<f64>()
    };
    // Two loops of a metre, and a quarter of a loop, which its table says moves a quarter of the stride's first half.
    assert!((moved(&[3.0]) - 2.125).abs() < 1e-6);
    let frames: Vec<f64> = (1..=90).map(|frame| f64::from(frame) / 30.0).collect();
    assert!((moved(&frames) - 2.125).abs() < 1e-6);
    // A sprint played once moves its whole table, and no further however long it stands.
    let mut animator = Animator::new(graph(), 0.0);
    animator.set("startle", Value::Trigger(true), 0.0);
    animator.set("fear", Value::Float(1.0), 0.0);
    assert!((animator.advance(5.0).moved - 2.0).abs() < 1e-6);
    let motion = RootMotion::new(0.5, vec![0.0, 0.25, 1.0]);
    assert_eq!(
        (
            motion.at(-1.0),
            motion.at(0.75),
            motion.at(9.0),
            motion.total()
        ),
        (0.0, 0.625, 1.0, 1.0)
    );
    assert_eq!(RootMotion::new(1.0, vec![0.0]).at(0.5), 0.0);
}

#[test]
fn the_same_inputs_give_the_same_state_however_it_is_advanced() {
    let run = |steps: Vec<f64>| {
        let mut animator = Animator::new(graph(), 100.0);
        animator.set("walking", Value::Bool(true), 100.3);
        animator.set("walking", Value::Bool(false), 101.7);
        animator.set("fear", Value::Float(0.9), 102.0);
        animator.set("startle", Value::Trigger(true), 102.1);
        let mut moved = 0.0;
        for step in steps {
            moved += animator.advance(step).moved;
        }
        (
            animator.state(),
            animator.entered(),
            animator.position(),
            animator.fading(),
            moved,
        )
    };
    let once = run(vec![103.37]);
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..20 {
        let mut steps = Vec::new();
        let mut now = 100.0;
        while now < 103.37 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            now = (now + (state % 100) as f64 / 1000.0).min(103.37);
            steps.push(now);
        }
        let divided = run(steps);
        assert_eq!((divided.0, divided.1, divided.3), (once.0, once.1, once.3));
        assert!((divided.2 - once.2).abs() < 1e-5);
        assert!((divided.4 - once.4).abs() < 1e-9);
    }
}

#[test]
fn planted_feet_carry_what_they_animate_as_far_as_they_sweep_back() {
    // Two feet: the left planted, sweeping back a metre over the first second while the right lifts and swings
    // forward, and then the other way round, as a walk.
    let channel = |node, values: Vec<Vec4>| Channel {
        node,
        property: Property::Translation,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 1.0, 2.0],
        values,
    };
    let at = |x: f32, y: f32| Vec4::new(x, y, 0.0, 0.0);
    let model = Model {
        nodes: vec![
            Node::new("left", Transform::IDENTITY),
            Node::new("right", Transform::IDENTITY),
        ],
        roots: vec![0, 1],
        animations: vec![Animation {
            name: Some("walk".into()),
            channels: vec![
                channel(0, vec![at(0.5, 0.0), at(-0.5, 0.0), at(0.5, 0.3)]),
                channel(1, vec![at(-0.5, 0.3), at(0.5, 0.0), at(-0.5, 0.0)]),
            ],
            duration: 2.0,
        }],
        ..Model::default()
    }
    .checked(&Limits::default())
    .unwrap();
    let motion = RootMotion::planted(&model, 0, (0.0, 2.0), &[0, 1], Vec3::X, 2.0);
    assert_eq!(motion.step, 0.5);
    assert_eq!(motion.distances.len(), 5);
    assert!((motion.at(1.0) - 1.0).abs() < 1e-5);
    assert!((motion.total() - 2.0).abs() < 1e-5);
}
