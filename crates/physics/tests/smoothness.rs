// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A stick thrown and left to the server, said as a side says it: a world stepped every 50 ms, and sooner as intents
//! arrive, said at most 20 times a second as of when what moves was where it says, over a network whose latency
//! varies, to two clients drawing it at 72 Hz, the one who threw it and another; each required to draw it moving every
//! frame, never holding still and then jumping, its steps changing from frame to frame little more than the stick's
//! own, as the simulation moved it. The other draws it in the thrower's hand as it swings, from the presence they send
//! each frame, relayed over the same network, then let go: required never to hold it still, nor move it back, and to
//! change how it moves from frame to frame little more than the hand and the stick themselves do.
use std::collections::VecDeque;

use glam::{Quat, Vec3};
use tomori_physics::message::{LetGo, Take, Update};
use tomori_physics::mirror::Mirror;
use tomori_physics::world::World;
use tomori_physics::{Body, Placement, Precision, SNAPSHOT_RATE, Seen, Settings, Side, Static};

/// A small, steady generator of numbers from 0 to 1.
struct Uniform(u64);

impl Uniform {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 1_000_000) as f64 / 1_000_000.0
    }
}

/// A stick of wood, 80 cm long and 3 cm across, lying on the ground.
fn stick() -> Body {
    let mut points = Vec::new();
    for x in [-0.4, 0.4] {
        for index in 0..6 {
            let turn = index as f32 / 6.0 * std::f32::consts::TAU;
            points.push(Vec3::new(x, turn.cos() * 0.015, turn.sin() * 0.015));
        }
    }
    Body::convex(1, points)
        .with_density(450.0)
        .at(Placement::at(Vec3::new(0.0, 0.02, 0.0)))
}

/// What was drawn, frame by frame, from the letting go until it rested: by the one who threw it, by another, and as
/// the simulation moved it, on time; and by the other from [`SWUNG`] before it, with where it truly was, in the hand
/// and then as the simulation moved it.
struct Drawn {
    holder: Vec<Placement>,
    other: Vec<Placement>,
    simulated: Vec<Placement>,
    watched: Vec<Placement>,
    truly: Vec<Placement>,
}

/// How the other draws what someone holds and lets go: from the hand as their presence last said, or between the
/// updates that brought it, as `tomori-scene`'s `relayed` does; and, let go, from the server's first snapshot, or on
/// from where it was drawn in the hand, as fast as it moved there (`Mirror::drawn_held`).
#[derive(Clone, Copy, Debug)]
struct Watching {
    between: bool,
    followed: bool,
}

/// How long the thrower swings the stick, from rest, before letting go, and how long their hand takes to slow after,
/// in seconds; and how long before the letting go the other's frames are measured from, in frames.
const SWING: f32 = 0.3;
const SLOWING: f32 = 0.06;
const SWUNG: i64 = 22;

/// Where the thrower's hand is `seconds` after letting go at `hand`, at `velocity`, spinning at `spin`: still, then
/// swinging faster and faster for [`SWING`], then slowing.
fn swung(hand: &Placement, velocity: Vec3, spin: Vec3, seconds: f32) -> Placement {
    let along = if seconds < -SWING {
        -SWING / 2.0
    } else if seconds <= 0.0 {
        (seconds + SWING).powi(2) / (2.0 * SWING) - SWING / 2.0
    } else {
        SLOWING * (1.0 - (-seconds / SLOWING).exp())
    };
    Placement::new(
        hand.position + velocity * along,
        (Quat::from_scaled_axis(spin * along) * hand.rotation).normalize(),
    )
}

/// The thrower's hand as the other draws it, between the updates that brought it, an interval behind, as
/// `tomori-scene`'s `relayed` does, by the time each arrived, in milliseconds.
#[derive(Default)]
struct Between {
    updates: VecDeque<(f64, Placement)>,
    interval: Option<f64>,
}

impl Between {
    fn update(&mut self, now: f64, hand: Placement) -> Placement {
        match self.updates.back() {
            Some((_, was)) if *was == hand => {}
            Some((then, _)) => {
                let gap = now - then;
                self.interval = Some(
                    self.interval
                        .map_or(gap, |interval| interval + (gap - interval) / 8.0),
                );
                self.updates.push_back((now, hand));
            }
            None => self.updates.push_back((now, hand)),
        }
        while self.updates.len() > 8 {
            self.updates.pop_front();
        }
        let track: Vec<(f64, Placement)> = self.updates.iter().copied().collect();
        at(&track, now - self.interval.unwrap_or(0.0).min(50.0))
    }
}

/// Where `track`, placements by the time they were so, puts a body at `at`.
fn at(track: &[(f64, Placement)], at: f64) -> Placement {
    let after = track.partition_point(|(then, _)| *then <= at);
    if after == 0 || after == track.len() {
        return track[after.min(track.len() - 1)].1;
    }
    let ((from, was), (to, will)) = (track[after - 1], track[after]);
    let t = ((at - from) / (to - from)) as f32;
    Placement::new(
        was.position.lerp(will.position, t),
        was.rotation.slerp(will.rotation, t),
    )
}

/// Throw the stick at `velocity`, spinning at `spin`, over a network whose one-way latency is 20 ms and up to `jitter`
/// more, with an intent arriving between each of the side's ticks as often as `woken`, the other `watching` it so; and
/// what was drawn.
fn thrown(
    velocity: Vec3,
    spin: Vec3,
    jitter: f64,
    woken: f64,
    seed: u64,
    watching: Watching,
) -> Drawn {
    let mut random = Uniform(seed);
    let ground = Static::cuboid(
        Vec3::new(-20.0, -1.0, -20.0),
        Vec3::new(20.0, 0.0, 20.0),
        Quat::IDENTITY,
    );
    let mut world =
        World::new(Settings::default(), vec![ground], Vec::new(), vec![stick()]).unwrap();
    assert!(world.settle(5.0));
    let state = world.shared();
    let (mut holder, mut other) = (Mirror::new(), Mirror::new());
    holder.share(&state);
    other.share(&state);
    let hand = Placement::at(Vec3::new(0.0, 1.1, 0.2)).snapped(Precision::Rest);
    let seen = Seen {
        head: Vec3::new(0.0, 1.6, 0.4),
        hands: [None, Some(hand)],
    };
    let take = Take {
        body: 1,
        epoch: 0,
        hand: Side::Right,
        offset: Placement::default().encoded(Precision::Rest),
    };
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    let taken = world.step(0.05);
    holder.apply(0, &taken, 0);
    other.apply(0, &taken, 0);
    // Real time, in milliseconds, is the server's clock, which both clients' lag by the latency their state arrived
    // with; the letting go arrives that much after it is made.
    let latency = 20.0;
    let released = 1000.0;
    let clock = |now: f64| (now - latency) as u64;
    holder.let_go(1, hand, velocity, spin, clock(released));
    let let_go = LetGo {
        body: 1,
        epoch,
        at: hand.encoded(Precision::Rest),
        velocity: velocity.to_array(),
        spin: spin.to_array(),
        clock: clock(released),
    };
    world.let_go(3, &let_go, Some(&seen)).unwrap();
    // The side, stepping its world every 50 ms, and as intents arrive between.
    let interval = 1000.0 / SNAPSHOT_RATE as f64;
    let (mut tick, mut stepped) = (released + latency, released + latency);
    let (mut said, mut moved) = (None::<f64>, stepped);
    let mut pending = Update::default();
    let mut sent: Vec<(f64, u64, Update)> = Vec::new();
    let mut simulated = vec![(stepped, hand)];
    let mut arrives = 0.0f64;
    let mut woken_at = None;
    while stepped < released + 8000.0 {
        let now = match woken_at.take() {
            Some(woken) => woken,
            None => {
                tick += interval + 0.3;
                if random.next() < woken {
                    woken_at = Some(tick);
                    tick - random.next() * interval
                } else {
                    tick
                }
            }
        };
        let update = world.step(((now - stepped) / 1000.0) as f32);
        stepped = now;
        let reached = now - world.behind() as f64 * 1000.0;
        simulated.push((reached, world.placement(1).unwrap()));
        if !update.moving.is_empty() {
            moved = reached;
        }
        pending.merge(update);
        if !pending.is_empty() && said.is_none_or(|said| now - said >= interval) {
            said = Some(now);
            let when = if pending.moving.is_empty() {
                now
            } else {
                moved
            };
            // In order, however late each is.
            arrives = arrives.max(now + latency + random.next() * jitter);
            sent.push((arrives, when as u64, std::mem::take(&mut pending)));
        }
    }
    // The thrower's presence, each of their frames at 72 Hz, in their own time, of their hand as it swings, relayed to
    // the server and on to the other, each way 20 ms and up to `jitter` more, in order.
    let mut network = Uniform(seed + 100);
    let frame = 1000.0 / 72.0;
    let mut presence = Vec::new();
    let (mut relayed, mut reached) = (0.0f64, 0.0f64);
    let mut then = released - 1000.0 + network.next() * frame;
    while then < released + 1000.0 {
        relayed = relayed.max(then + latency + network.next() * jitter);
        reached = reached.max(relayed + latency + network.next() * jitter);
        let seconds = ((then - released) / 1000.0) as f32;
        presence.push((reached, swung(&hand, velocity, spin, seconds)));
        then += frame;
    }
    let mut drawn = Drawn {
        holder: Vec::new(),
        other: Vec::new(),
        simulated: Vec::new(),
        watched: Vec::new(),
        truly: Vec::new(),
    };
    let (mut arrived, mut heard) = (0, 0);
    let mut between = Between::default();
    let mut presented = swung(&hand, velocity, spin, -1.0);
    for frame in -SWUNG.. {
        let now = released + frame as f64 * 1000.0 / 72.0;
        // The latest of the thrower's presence to have arrived, each frame.
        while heard < presence.len() && presence[heard].0 <= now {
            presented = presence[heard].1;
            heard += 1;
        }
        let in_hand = if watching.between {
            between.update(now, presented)
        } else {
            presented
        };
        while arrived < sent.len() && sent[arrived].0 <= now {
            let (_, at, update) = &sent[arrived];
            holder.apply(*at, update, clock(now));
            other.apply(*at, update, clock(now));
            arrived += 1;
        }
        // The other draws it in the thrower's hand until the side says it was let go.
        let watched = match other.held(1) {
            Some(held) => {
                let placement = held.placement(&in_hand);
                if watching.followed {
                    other.drawn_held(1, placement, clock(now))
                } else {
                    placement
                }
            }
            None => other.placement(1, clock(now)).unwrap(),
        };
        drawn.watched.push(watched);
        drawn.truly.push(if now <= released {
            swung(&hand, velocity, spin, ((now - released) / 1000.0) as f32)
        } else {
            // The server simulates it from where it was let go as the letting go arrives.
            at(&simulated, now + latency)
        });
        if frame < 0 {
            continue;
        }
        drawn.holder.push(holder.placement(1, clock(now)).unwrap());
        drawn.other.push(watched);
        drawn.simulated.push(at(&simulated, now));
        let still = |drawn: &[Placement]| drawn[drawn.len() - 1] == drawn[drawn.len() - 2];
        if arrived == sent.len() && frame > 72 && still(&drawn.holder) && still(&drawn.other) {
            break;
        }
        assert!(frame < 72 * 10, "never rested");
    }
    drawn
}

/// Of what the other drew, from the swing until the stick came down near the ground, by `direction`, which way the
/// throw went across the ground: on how many frames it held still, having moved and moving again, and how many it moved
/// back against the throw by more than a millimetre; and the most its step changed from one frame to the next, in
/// millimetres, and that most of where it truly was.
fn watched(drawn: &Drawn, direction: Vec3) -> (usize, usize, f32, f32) {
    let direction = Vec3::new(direction.x, 0.0, direction.z).normalize();
    let aloft = |placements: &[Placement]| {
        let down = placements
            .iter()
            .position(|placement| placement.position.y < 0.15)
            .unwrap_or(placements.len());
        placements[..down]
            .windows(2)
            .map(|pair| (pair[1].position - pair[0].position) * 1000.0)
            .collect::<Vec<Vec3>>()
    };
    let most = |steps: &[Vec3]| {
        steps
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .fold(0.0f32, f32::max)
    };
    let steps = aloft(&drawn.watched);
    let moved = |steps: &[Vec3]| steps.iter().any(|step| step.length() > 1.0);
    let held = (0..steps.len())
        .filter(|&frame| {
            steps[frame].length() < 0.05 && moved(&steps[..frame]) && moved(&steps[frame + 1..])
        })
        .count();
    let back = steps
        .iter()
        .filter(|step| step.dot(direction) < -1.0)
        .count();
    (held, back, most(&steps), most(&aloft(&drawn.truly)))
}

/// How far it moved and turned from each frame to the next, in millimetres and degrees.
fn steps(drawn: &[Placement]) -> Vec<(f32, f32)> {
    drawn
        .windows(2)
        .map(|pair| {
            (
                pair[0].position.distance(pair[1].position) * 1000.0,
                pair[0]
                    .rotation
                    .angle_between(pair[1].rotation)
                    .to_degrees(),
            )
        })
        .collect()
}

/// How many frames it was drawn holding still, though it moved a millimetre or turned half a degree in the frames
/// either side; and how much its steps changed from frame to frame, in position and in rotation, against how large
/// they were, over the frames it moved.
fn stutter(drawn: &[Placement]) -> (usize, f32, f32) {
    let steps = steps(drawn);
    let held = steps
        .windows(3)
        .filter(|three| {
            let moved = |step: &(f32, f32)| step.0 > 1.0 || step.1 > 0.5;
            three[1].0 < 0.05 && three[1].1 < 0.01 && moved(&three[0]) && moved(&three[2])
        })
        .count();
    let moving: Vec<&(f32, f32)> = steps
        .iter()
        .filter(|step| step.0 > 0.5 || step.1 > 0.1)
        .collect();
    let change = |pick: fn(&(f32, f32)) -> f32| {
        let size = moving.iter().map(|step| pick(step)).sum::<f32>();
        let change = moving
            .windows(2)
            .map(|pair| (pick(pair[1]) - pick(pair[0])).abs())
            .sum::<f32>();
        change / size.max(1e-6)
    };
    (held, change(|step| step.0), change(|step| step.1))
}

/// The throws: at what velocity, spinning how fast, over how much jitter, woken how often, from which seed.
const THROWS: [(Vec3, Vec3, f64, f64, u64); 4] = [
    (
        Vec3::new(0.5, 2.5, -2.0),
        Vec3::new(4.0, 0.0, 6.0),
        0.0,
        0.0,
        1,
    ),
    (
        Vec3::new(0.5, 2.5, -2.0),
        Vec3::new(4.0, 0.0, 6.0),
        40.0,
        0.3,
        2,
    ),
    (
        Vec3::new(-1.0, 1.0, -4.0),
        Vec3::new(0.0, 8.0, 2.0),
        60.0,
        0.5,
        3,
    ),
    (
        Vec3::new(0.2, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        40.0,
        0.3,
        4,
    ),
];

#[test]
fn a_thrown_stick_is_drawn_moving_smoothly_every_frame_by_the_holder_and_everyone_else() {
    let watching = Watching {
        between: true,
        followed: true,
    };
    for (velocity, spin, jitter, woken, seed) in THROWS {
        let drawn = thrown(velocity, spin, jitter, woken, seed, watching);
        let (_, moving, turning) = stutter(&drawn.simulated);
        println!(
            "Thrown at {velocity} m/s spinning at {spin} rad/s, {jitter} ms of jitter, woken {woken}: simulated, \
             steps changing {:.0}% moving and {:.0}% turning",
            moving * 100.0,
            turning * 100.0,
        );
        for (who, frames) in [("holder", &drawn.holder), ("other", &drawn.other)] {
            let (held, position, rotation) = stutter(frames);
            println!(
                "  {who}: {} frames, {held} held still, steps changing {:.0}% moving and {:.0}% turning",
                frames.len(),
                position * 100.0,
                rotation * 100.0
            );
            assert_eq!(held, 0, "{who} held it still");
            assert!(
                position < moving + 0.15 && rotation < turning + 0.2,
                "{who} stuttered"
            );
        }
    }
}

#[test]
fn another_draws_a_stick_swung_and_let_go_moving_on_every_frame_never_back() {
    for watching in [
        Watching {
            between: false,
            followed: false,
        },
        Watching {
            between: true,
            followed: false,
        },
        Watching {
            between: false,
            followed: true,
        },
        Watching {
            between: true,
            followed: true,
        },
    ] {
        println!("{watching:?}");
        for (velocity, spin, jitter, woken, seed) in THROWS {
            let drawn = thrown(velocity, spin, jitter, woken, seed, watching);
            let (held, back, most, truly) = watched(&drawn, velocity);
            println!(
                "  at {velocity} m/s, {jitter} ms of jitter: {} frames, {held} held still, {back} back, its step \
                 changing at most {most:.1} mm, truly {truly:.1} mm",
                drawn.watched.len(),
            );
            // Drawn between the hand's updates and on from it, it never holds still nor moves back, and its step
            // changes at most a few times as much as the hand's and the stick's own most.
            if watching.between && watching.followed {
                assert_eq!(held, 0, "held it still");
                assert_eq!(back, 0, "moved it back");
                assert!(most < 3.5 * truly, "jumped");
            }
        }
    }
}
