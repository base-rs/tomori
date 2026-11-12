// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A stick thrown and left to the server, said as a side says it: a world stepped every 50 ms, and sooner as intents
//! arrive, said at most 20 times a second as of when what moves was where it says, over a network whose latency
//! varies, to two clients drawing it at 72 Hz, the one who threw it and another; each required to draw it moving every
//! frame, never holding still and then jumping, its steps changing from frame to frame little more than the stick's
//! own, as the simulation moved it.
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
/// the simulation moved it, on time.
struct Drawn {
    holder: Vec<Placement>,
    other: Vec<Placement>,
    simulated: Vec<Placement>,
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
/// more, with an intent arriving between each of the side's ticks as often as `woken`; and what was drawn.
fn thrown(velocity: Vec3, spin: Vec3, jitter: f64, woken: f64, seed: u64) -> Drawn {
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
    let mut drawn = Drawn {
        holder: Vec::new(),
        other: Vec::new(),
        simulated: Vec::new(),
    };
    let mut arrived = 0;
    for frame in 0.. {
        let now = released + frame as f64 * 1000.0 / 72.0;
        while arrived < sent.len() && sent[arrived].0 <= now {
            let (_, at, update) = &sent[arrived];
            holder.apply(*at, update, clock(now));
            other.apply(*at, update, clock(now));
            arrived += 1;
        }
        drawn.holder.push(holder.placement(1, clock(now)).unwrap());
        // Another draws it in the holder's hand until the side says it was let go.
        drawn
            .other
            .push(other.placement(1, clock(now)).unwrap_or(hand));
        drawn.simulated.push(at(&simulated, now));
        let still = |drawn: &[Placement]| drawn[drawn.len() - 1] == drawn[drawn.len() - 2];
        if arrived == sent.len() && frame > 72 && still(&drawn.holder) && still(&drawn.other) {
            break;
        }
        assert!(frame < 72 * 10, "never rested");
    }
    drawn
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

#[test]
fn a_thrown_stick_is_drawn_moving_smoothly_every_frame_by_the_holder_and_everyone_else() {
    for (velocity, spin, jitter, woken, seed) in [
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
    ] {
        let drawn = thrown(velocity, spin, jitter, woken, seed);
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
