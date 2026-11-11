// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The audio callback allocates nothing to play an ambience: its thread decodes into blocks that go round, and the
//! callback only reads them. Counted with an allocator of the test's own, on the callback's thread only, in a binary of
//! its own. See `design/spatial-audio.md`.
use std::alloc::{GlobalAlloc, Layout as Memory, System};
use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use tomori_client::playback::ambience::{Ambience, Bed, Call, Loop, Slot};
use tomori_client::playback::spatial::Listener;
use tomori_client::playback::{Layout, MixerOptions, Player};

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

const TONE: &[u8] = include_bytes!("../src/playback/ambience/fixtures/tone-mono.ogg");
const LEFT: &[u8] = include_bytes!("../src/playback/ambience/fixtures/tone-left.ogg");

#[test]
fn playing_an_ambience_allocates_nothing_in_the_audio_callback() {
    let (_frames, audio) = std::sync::mpsc::channel();
    let (mut player, control) = Player::new(audio, MixerOptions::default());
    player.prepare(48_000);
    control.set_scene(|scene| {
        scene.listener = Some(Listener {
            position: [0.0, 1.6, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        });
    });
    // A bed, two loops and calls, as often as calls can come, so that voices begin and end throughout.
    let ambience = Ambience::new()
        .with_bed(Bed::new(Slot::embedded("bed", LEFT)))
        .with_loop(Loop::at(
            Slot::embedded("waterfall", TONE),
            [3.0, 1.0, -4.0],
        ))
        .with_loop(Loop::along(
            Slot::embedded("river", TONE),
            &[[-10.0, 0.0, -8.0], [10.0, 0.0, -8.0]],
        ))
        .with_call(
            Call::new(vec![Slot::embedded("tui", TONE)])
                .every(Duration::ZERO, Duration::from_millis(200))
                .around(&[[-4.0, 4.0, -6.0], [5.0, 3.0, -7.0]], 2.0)
                .pitch(0.9, 1.1),
        );
    control.set_ambience(Some(Arc::new(ambience)));
    let mut output = vec![0i16; 960];
    // Render 10 ms at a time, about as often, as a device would, until the ambience is heard and has settled.
    let mut render = |player: &mut Player| {
        player.render(48_000, Layout::Stereo, &mut output);
        std::thread::sleep(Duration::from_millis(10));
        output.iter().any(|sample| sample.unsigned_abs() > 500)
    };
    assert!(
        (0..1_000).any(|_| render(&mut player)),
        "the ambience was never heard"
    );
    for _ in 0..50 {
        render(&mut player);
    }
    // Two seconds more of it, counting every allocation and free in the callback.
    let mut heard = 0;
    for _ in 0..200 {
        COUNTING.with(|counting| counting.set(true));
        player.render(48_000, Layout::Stereo, &mut output);
        COUNTING.with(|counting| counting.set(false));
        heard += usize::from(output.iter().any(|sample| sample.unsigned_abs() > 500));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(heard > 150, "{heard}");
    assert_eq!(COUNTED.with(Cell::get), 0);
}
