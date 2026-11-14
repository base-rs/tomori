// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A frame with others in the space allocates nothing once everyone is known: their presence heard, whose voices play,
//! each drawn between their updates as a swarm with their rays, and their hands given to an environment's client side,
//! each into buffers kept from frame to frame, so that people cost the headset's frame no time in the allocator. Counted
//! on the test's thread only, so that what the test harness allocates on its own threads meanwhile is not. See
//! `design/people.md`.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use tomori_client::Event;
use tomori_client::connection::{Report, Session};
use tomori_protocol::{Hand, JOINT_COUNT, Joint, Participant, Pose, Ray, ServerMessage};
use tomori_scene::anchors::Anchor;
use tomori_scene::conversation::Conversation;
use tomori_scene::people::People;
use tomori_scene::relayed::{Others, others};
use tomori_scene::scene::fixtures::{joined, space};

/// The system's allocator, counting what each thread allocates.
struct Counting;

thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    // Without a thread's locals, as it ends, nothing is counted.
    let _ = ALLOCATED.try_with(|allocated| allocated.set(allocated.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// How many allocations this thread has made.
fn allocated() -> usize {
    ALLOCATED.with(Cell::get)
}

/// The presence of `source` at `frame`, walking slowly along x, a controller in each hand but for the first, whose hands
/// are tracked, and the second pointing at something.
fn presence(source: u32, frame: usize) -> Event {
    let x = source as f32 - 3.0 + frame as f32 * 0.002;
    let head = Pose::upright_facing([x, 1.6, -2.0], [x, 1.6, 0.0]);
    let at = |dx: f32| Pose::at([x + dx, 1.2, -1.7]);
    let tracked = source == 2;
    let hand = |dx: f32| {
        let mut joints = [None; JOINT_COUNT];
        joints[0] = Some(Joint {
            position: [x + dx, 1.25, -1.7],
            radius: 0.02,
        });
        Hand { joints }
    };
    Event::Message(ServerMessage::Presence {
        source,
        head,
        left: (!tracked).then(|| at(-0.2)),
        right: (!tracked).then(|| at(0.2)),
        hands: tracked.then(|| Box::new([Some(hand(-0.2)), Some(hand(0.2))])),
        pointing: (source == 3).then(|| {
            Box::new([
                None,
                Some(Ray {
                    aim: Pose::at([x + 0.2, 1.25, -1.75]),
                    length: 1.5,
                    meets: true,
                }),
            ])
        }),
    })
}

#[test]
fn a_frame_with_others_in_the_space_allocates_nothing_once_everyone_is_known() {
    let others_here = [2, 3, 4, 5];
    let mut conversation: Conversation = joined();
    conversation.apply(space(|state| {
        for (name, source) in [("Bob", 2), ("Cleo", 3), ("Dan", 4), ("Eve", 5)] {
            state.participants.push(Participant {
                name: name.into(),
                source,
                engagement: None,
            });
        }
    }));
    let frames = 240;
    // Each frame's presence, as it arrives over the network, made before anything is counted.
    let mut arriving: Vec<Vec<Event>> = (0..frames)
        .map(|frame| {
            others_here
                .iter()
                .map(|source| presence(*source, frame))
                .collect()
        })
        .collect();
    arriving.reverse();
    let carriage = Anchor {
        position: Vec3::new(0.0, 0.0, -1.0),
        orientation: Quat::from_rotation_y(0.3),
    };
    let start = Instant::now();
    let mut people = People::default();
    let mut hands = Others::default();
    let mut cubes = 0;
    let eyes = Some(Vec3::new(0.0, 1.6, 0.0));
    for frame in 0..frames {
        let now = start + Duration::from_secs_f32(frame as f32 / 72.0);
        let events = arriving.pop().expect("a frame's presence");
        // Once everyone has been drawn for a second, nothing more is allocated.
        let before = allocated();
        for event in events {
            conversation.apply(event);
        }
        let voices = [2, 4].into_iter().collect();
        conversation.report(Report::Voices(voices));
        people.hear(conversation.voices());
        people.update(
            now,
            1.0 / 72.0,
            conversation.hands_forgotten(),
            conversation.people(),
            eyes,
            None,
        );
        others(people.drawn(), carriage, &mut hands);
        cubes = people
            .figures()
            .iter()
            .map(|figure| figure.cubes.len())
            .sum::<usize>();
        let during = allocated() - before;
        if frame >= 72 {
            assert_eq!(during, 0, "frame {frame} allocated");
        }
    }
    // Everyone was drawn, in full, with their rays and hands.
    assert_eq!(people.figures().len(), 4);
    assert!(cubes > 4 * 250, "{cubes}");
    assert_eq!((hands.0.len(), hands.1.len()), (3, 3));
}
