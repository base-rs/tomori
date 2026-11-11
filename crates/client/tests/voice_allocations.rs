// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The audio callback allocates nothing to place the agent's voice and a person's, as the listener walks through the
//! agent, from beyond its reference distance, through its near field and out the other side: placing and rendering
//! allocate nothing, and the mixer allocates nothing more than it does unplaced, the frees of frames once played. Counted with an allocator
//! of the test's own, on the callback's thread only, in a binary of its own. See `design/spatial-audio.md`.
use std::alloc::{GlobalAlloc, Layout as Memory, System};
use std::cell::Cell;

use tomori_client::playback::spatial::{Ears, Falloff, Listener, Placement, Scene, place};
use tomori_client::playback::{Mixer, MixerOptions};
use tomori_protocol::{AGENT_SOURCE, AudioFrame, Gate};

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static COUNTED: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    // Without a thread's locals, as as it ends, nothing is counted.
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            COUNTED.with(|counted| counted.set(counted.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Memory) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Memory) {
        count();
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Memory, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const RATE: u32 = 48_000;
/// A person, standing 1 m to the agent's right.
const PERSON: u32 = 1;

fn frame(source: u32, sequence: u32) -> AudioFrame {
    AudioFrame {
        gate: Gate::Live,
        source,
        sequence,
        timestamp: 0,
        sample_rate: RATE,
        samples: (0..960)
            .map(|index| ((index as f32 * 0.05).sin() * 8_000.0) as i16)
            .collect(),
    }
}

/// What the callback allocates and frees, counted, while `walk` runs.
fn counted(walk: impl FnOnce()) -> usize {
    COUNTED.with(|counted| counted.set(0));
    COUNTING.with(|counting| counting.set(true));
    walk();
    COUNTING.with(|counting| counting.set(false));
    COUNTED.with(Cell::get)
}

fn listener(x: f32) -> Listener {
    Listener {
        position: [x, 1.6, -0.3],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

#[test]
fn placing_and_rendering_the_agent_as_the_listener_walks_through_it_allocates_nothing() {
    let mut ears = Ears::new(RATE);
    let input = vec![0.25; 480];
    let mut output = vec![0.0; 960];
    ears.render(&input, Placement::CENTRE, (1.0, 1.0), &mut output);
    // Six seconds of 10 ms callbacks, walking 2 m/s from 6 m to the agent's left to 6 m to its right, 0.3 m in front of it.
    let allocations = counted(|| {
        for call in 0..600 {
            let listener = listener(-6.0 + 12.0 * call as f32 / 600.0);
            for mono in [false, true] {
                let placement = place(&listener, [0.0, 1.6, 0.0], Falloff::AGENT, RATE, mono);
                ears.render(&input, placement, (1.0, 1.0), &mut output);
            }
            let placement = place(&listener, [1.0, 1.6, 0.0], Falloff::PERSON, RATE, false);
            ears.render(&input, placement, (1.0, 1.0), &mut output);
        }
    });
    assert_eq!(allocations, 0);
}

/// Six seconds of 20 ms mixes of the agent and a person, standing 1 m to its right, with frames arriving between mixes as the application pushes them, the listener walking through the agent with `walking`, or nowhere placed without: the allocations and frees in the mixes, and how many were heard.
fn mixed(walking: bool) -> (usize, usize) {
    let mut mixer = Mixer::new(MixerOptions::default());
    assert_eq!(mixer.sample_rate(), RATE);
    if walking {
        *mixer.scene_mut() = Scene {
            listener: Some(listener(-6.0)),
            sources: vec![(AGENT_SOURCE, [0.0, 1.6, 0.0]), (PERSON, [1.0, 1.6, 0.0])],
            mono: false,
        };
    }
    let mut output = vec![0i16; 1_920];
    let mut sequence = 0;
    let mut push = |mixer: &mut Mixer| {
        for source in [AGENT_SOURCE, PERSON] {
            mixer.push(frame(source, sequence));
        }
        sequence += 1;
    };
    // Enough buffered to start playing, and the mix settled at each callback's size.
    for _ in 0..4 {
        push(&mut mixer);
    }
    for _ in 0..10 {
        mixer.mix_stereo(&mut output);
        push(&mut mixer);
    }
    let mut heard = 0;
    let mut allocations = 0;
    for call in 0..300 {
        if let Some(listener) = mixer.scene_mut().listener.as_mut() {
            listener.position[0] = -6.0 + 12.0 * call as f32 / 300.0;
        }
        allocations += counted(|| mixer.mix_stereo(&mut output));
        heard += usize::from(output.iter().any(|sample| sample.unsigned_abs() > 1_000));
        push(&mut mixer);
    }
    (allocations, heard)
}

#[test]
fn walking_through_the_agent_allocates_nothing_more_in_the_mixer() {
    // The mixer frees each frame's samples once played, as `Player` says; placing the voices adds nothing to that.
    let (placed, heard) = mixed(true);
    let (unplaced, _) = mixed(false);
    assert!(heard > 290, "{heard}");
    assert_eq!(placed, unplaced);
}
