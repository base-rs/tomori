// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tests of the echo test.
use super::*;

/// An echo test for a live microphone opened at `opened`, as it starts.
fn echo_test(routed: bool) -> EchoTest {
    EchoTest {
        label: "test",
        routed,
        armed: None,
        speaker: None,
        opened: None,
        probed: None,
        floor: Heard::default(),
        echo: Heard::default(),
    }
}

#[test]
fn the_echo_test_hears_the_room_then_the_probe_and_reports_once() {
    let mut test = echo_test(false);
    let start = Instant::now();
    let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);
    let heard = |level: f64| Heard {
        energy: level * level * 320.0,
        samples: 320,
    };
    // Before the live microphone opens, nothing.
    assert_eq!(test.update(heard(1.0), at(0)), (None, false));
    test.opened = Some(start);
    // The first 300 ms are ignored, then the room is heard until the probe plays at one second.
    assert_eq!(test.update(heard(1.0), at(100)), (None, false));
    assert_eq!(test.update(heard(0.001), at(500)), (None, false));
    assert_eq!(test.update(heard(0.001), at(900)), (None, false));
    assert_eq!(test.update(Heard::default(), at(1_000)), (None, true));
    assert_eq!(test.update(heard(1.0), at(1_100)), (None, false));
    assert_eq!(test.update(heard(0.01), at(1_400)), (None, false));
    assert_eq!(test.update(heard(0.01), at(2_200)), (None, false));
    let (result, probe) = test.update(Heard::default(), at(2_400));
    assert!(!probe);
    assert_eq!(
        result.as_deref(),
        Some(
            "Echo test (test, the speaker as media): the speaker played a probe at -20.0 dBFS; the microphone heard -40.0 dBFS during it, 20.0 dB below the probe, 20.0 dB above the room; the room was -60.0 dBFS"
        )
    );
    // Once only, until the microphone opens again.
    assert_eq!(test.update(heard(1.0), at(3_000)), (None, false));
    assert_eq!(Heard::default().level(), None);
}

#[test]
fn the_echo_test_waits_for_the_speaker_to_play_as_it_should() {
    crate::test_support::log_everything();
    let start = Instant::now();
    // Not armed, as before the live microphone opens: nothing starts.
    let mut test = echo_test(true);
    test.arm(Some(Some(7)), start);
    assert!(test.opened.is_none());
    // Armed, needing the microphone's session: not while the speaker is closed or plays as media.
    test.armed = Some(start);
    test.arm(None, start);
    test.arm(Some(None), start + Duration::from_millis(100));
    assert!(test.opened.is_none() && test.armed.is_some());
    // It gives up after a while, measuring nothing.
    test.arm(Some(None), start + ECHO_WAIT);
    assert!(test.opened.is_none() && test.armed.is_none());
    // As voice communication, it starts, saying in which session.
    test.armed = Some(start);
    test.arm(Some(Some(7)), start);
    assert_eq!((test.opened, test.speaker), (Some(start), Some(7)));
    // A test that needs no session starts once the speaker is open, as media, and gives up too if it never opens.
    let mut test = echo_test(false);
    test.armed = Some(start);
    test.arm(Some(None), start);
    assert_eq!((test.opened, test.speaker), (Some(start), None));
    let mut test = echo_test(false);
    test.armed = Some(start);
    test.arm(None, start + ECHO_WAIT);
    assert!(test.opened.is_none() && test.armed.is_none());
}

/// The result of an echo test that heard `floor` from the room and `echo` from the probe, played by a speaker in session `speaker`.
fn echo_result(speaker: Option<i32>, floor: Heard, echo: Heard) -> String {
    let opened = Instant::now();
    let mut test = EchoTest {
        speaker,
        opened: Some(opened),
        probed: Some(opened),
        floor,
        echo,
        ..echo_test(false)
    };
    test.update(Heard::default(), opened + ECHO_HEARD.end)
        .0
        .expect("a result")
}

#[test]
fn the_echo_test_says_what_it_could_not_hear_and_when_the_echo_is_lost_in_the_room() {
    let heard = |level: f64| Heard {
        energy: 10_f64.powf(level / 10.0) * 100.0,
        samples: 100,
    };
    let nothing = echo_result(None, Heard::default(), Heard::default());
    assert!(nothing.contains("the speaker as media"));
    assert!(nothing.contains("the microphone heard nothing during it; the room was nothing"));
    let lost = echo_result(Some(7), heard(-50.0), heard(-49.0));
    assert!(lost.contains("as voice communication in session 7"));
    assert!(lost.contains("within 1.0 dB of the room, so at most this"));
    assert!(lost.contains("dB below the probe"));
}
