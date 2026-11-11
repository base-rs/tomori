// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The space's timers, drawn as dials above the agent's home, counting down by this client's clock, and pointed at as
//! words begin, as a worker is. See `design/timers.md`.

use super::*;
use tomori_protocol::{Resource, Timer, TimerState};

const NOW: f64 = 1_800_000_000.0;

fn status() -> Timer {
    Timer::new(
        "status",
        "every hour",
        "when present",
        Some(NOW as u64 + 12 * 60 + 5),
        TimerState::Waiting,
    )
}

#[test]
fn a_dial_counts_down_by_this_client_s_clock_and_is_pointed_at_as_words_begin() {
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| state.timers = vec![status()]));
    let mut step = Step::default();
    let start = Instant::now();
    // The system's clock, which a test moves: it keeps time with the instants each frame is given, until it jumps.
    let system = std::sync::Arc::new(std::sync::Mutex::new(NOW));
    let read = system.clone();
    step.set_clock(move || *read.lock().unwrap());
    let mut feedback = felt();
    let mut senses = Senses {
        beam: true,
        ..senses()
    };
    let mut pill = String::new();
    let mut frame = 0u64;
    let mut step_to = |step: &mut Step,
                       conversation: &mut Conversation,
                       senses: &Senses,
                       seconds: f64,
                       jump: f64| {
        let mut output_pill = String::new();
        let frames = (seconds / 0.05).round() as u64;
        for _ in 0..frames.max(1) {
            frame += 1;
            *system.lock().unwrap() += 0.05;
            let now = start + Duration::from_millis(frame * 50);
            let output = step.update(senses, conversation, &mut felt(), now, FRAME);
            output_pill.clone_from(&output.dials[0].pill);
        }
        *system.lock().unwrap() += jump;
        output_pill
    };
    for _ in 0..40 {
        pill = step_to(&mut step, &mut conversation, &senses, 0.05, 0.0);
    }
    // Two seconds on, ticking by the instants each frame is given.
    assert_eq!(pill, "12:03");
    // The device sleeps for ten minutes: the steady clock stood still, and the system's clock moved on. The countdown
    // follows the system's clock at once.
    step_to(&mut step, &mut conversation, &senses, 0.05, 600.0);
    assert_eq!(
        step_to(&mut step, &mut conversation, &senses, 0.05, 0.0),
        "2:03"
    );
    // A drift under a second is left to the steady clock, so that it never stutters.
    step_to(&mut step, &mut conversation, &senses, 0.05, 0.5);
    assert_eq!(
        step_to(&mut step, &mut conversation, &senses, 0.05, 0.0),
        "2:03"
    );
    // The system's clock is set back half a minute: the countdown follows it back.
    step_to(&mut step, &mut conversation, &senses, 0.05, -30.0);
    assert_eq!(
        step_to(&mut step, &mut conversation, &senses, 0.05, 0.0),
        "2:33"
    );
    let output = step.update(
        &senses,
        &mut conversation,
        &mut feedback,
        start + Duration::from_millis(frame * 50),
        FRAME,
    );
    let later = start + Duration::from_millis(frame * 50);
    let dial = Vec3::from_array(output.dials[0].position);
    // The ray on the dial: it ends there, the hover ticks, and words begun are about the timer.
    senses.ray = Ray::new(arrival::HOME, dial - arrival::HOME);
    let output = step.update(&senses, &mut conversation, &mut feedback, later, FRAME);
    assert!(output.dials[0].pointed);
    let beam = output.arrival.and_then(|arrival| arrival.ray).unwrap();
    assert!(beam.hit && beam.end.distance(dial) < 0.01);
    conversation.take_actions();
    conversation.talk(true);
    let Some(crate::recording::Phase::Recording { about, .. }) =
        conversation.recording_phase(Instant::now())
    else {
        unreachable!("recording")
    };
    assert_eq!(about.as_deref(), Some("timer status"));
    conversation.talk(false);
    let actions = conversation.take_actions();
    let Some(Action::BeginUtterance(utterance)) = actions.first() else {
        unreachable!("recording begins, not {actions:?}")
    };
    assert_eq!(utterance.attention, [Resource::timer("status")]);
    // A timer the space no longer has is not attended to, nor drawn.
    conversation.apply(space(|state| state.timers = Vec::new()));
    conversation.talk(true);
    conversation.talk(false);
    let actions = conversation.take_actions();
    let Some(Action::BeginUtterance(utterance)) = actions.first() else {
        unreachable!("recording begins, not {actions:?}")
    };
    assert!(utterance.attention.is_empty());
    // Off the dial, nothing is pointed at.
    senses.ray = Ray::new(arrival::HOME, Vec3::Y);
    let output = step.update(&senses, &mut conversation, &mut feedback, later, FRAME);
    assert!(output.dials.iter().all(|dial| !dial.pointed));
}
