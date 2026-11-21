// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::smoothing::{
    CINEMATIC, LIGHT,
    traces::{orientation, report},
};

fn at(x: f32) -> Vec3 {
    Vec3::new(x, 1.6, 0.0)
}

fn still(track: &mut Track, second: f64) {
    track.push(second, report(at(0.0), Quat::IDENTITY));
}

#[test]
fn the_interval_between_reports_is_observed_and_the_delay_follows_it_within_bounds() {
    for (tuning, interval, delay) in [
        // A report every 10 ms is a delay of the least; every 50 ms is one and a half intervals; every 100 ms and more is the most.
        (&LIGHT, 0.01, LIGHT.min_delay),
        (&LIGHT, 0.05, 0.075),
        (&LIGHT, 0.1, LIGHT.max_delay),
        (&LIGHT, 0.4, LIGHT.max_delay),
        (&CINEMATIC, 0.01, CINEMATIC.min_delay),
        (&CINEMATIC, 0.05, 0.1),
        (&CINEMATIC, 0.08, 0.16),
        (&CINEMATIC, 0.4, CINEMATIC.max_delay),
    ] {
        let mut track = Track::default();
        for report in 0..60 {
            still(&mut track, f64::from(report) * interval);
        }
        assert!(
            (track.delay(tuning) - delay).abs() < 0.002,
            "{interval}: {}",
            track.delay(tuning)
        );
    }
}

#[test]
fn reports_cut_by_a_teleport_a_gap_or_a_lost_connection_are_not_the_interval() {
    let mut track = Track::default();
    for report in 0..40 {
        still(&mut track, f64::from(report) * 0.05);
    }
    let interval = track.interval;
    // Three seconds on; the head 5 m away; and, right after, a connection lost and a report 0.3 m from the last.
    track.push(5.0, report(at(0.0), Quat::IDENTITY));
    track.push(5.05, report(at(5.0), Quat::IDENTITY));
    track.interrupt();
    track.push(5.1, report(at(5.3), Quat::IDENTITY));
    track.push(5.15, report(at(5.3), Quat::IDENTITY));
    assert_eq!(track.cuts(), 3);
    assert!((track.interval - interval).abs() < 0.01);
    // The silence before a cut is left out of the times, but for one interval: the cut follows the report before it by about an interval.
    let times: Vec<f64> = track.samples.iter().map(|sample| sample.at).collect();
    assert!(
        times.windows(2).all(|pair| pair[1] - pair[0] < 0.06),
        "{times:?}"
    );
    // And the next report is not a cut.
    track.push(5.2, report(at(5.3), Quat::IDENTITY));
    assert_eq!(track.cuts(), 3);
}

#[test]
fn only_so_many_reports_are_kept_and_nothing_grows() {
    let mut track = Track::default();
    let capacity = track.samples.capacity();
    for report in 0..1000 {
        still(&mut track, f64::from(report) * 0.05);
    }
    assert_eq!(track.samples.len(), CAPACITY);
    assert_eq!(track.samples.capacity(), capacity);
    // A view after a long stall is of the oldest kept, at the least, and of the newest at the most.
    let base = Instant::now();
    let view = track.view(base, &LIGHT).unwrap();
    assert_eq!(view.head.position, at(0.0));
    let view = track
        .view(base + std::time::Duration::from_secs(60), &LIGHT)
        .unwrap();
    assert_eq!(view.head.position, at(0.0));
}

#[test]
fn a_view_is_the_same_for_the_same_instant_and_a_stall_moves_it_by_no_more_than_the_longest_frame()
{
    let mut track = Track::default();
    for report in 0..20 {
        track.push(f64::from(report) * 0.05, report_at(report));
    }
    let base = Instant::now();
    let first = track.view(base, &LIGHT).unwrap();
    // The same instant again: the same view, rather than a division by nothing.
    assert_eq!(track.view(base, &LIGHT), Some(first));
    // A minute later the playhead has moved by the longest frame, not a minute.
    let playhead = track.playhead.unwrap();
    track
        .view(base + std::time::Duration::from_secs(60), &LIGHT)
        .unwrap();
    assert!(track.playhead.unwrap() - playhead <= LONGEST_FRAME * LIGHT.max_rate + 1e-9);
}

fn report_at(count: i32) -> Presence {
    report(
        at(count as f32 * 0.05),
        orientation(count as f32 * 0.05, 0.0, 0.0),
    )
}

#[test]
fn reports_at_the_same_second_are_played_without_dividing_by_nothing() {
    let mut track = Track::default();
    let base = Instant::now();
    track.push(0.0, report(at(0.0), Quat::IDENTITY));
    track.push(0.0, report(at(0.5), Quat::IDENTITY));
    track.push(0.0, report(at(0.5), Quat::IDENTITY));
    for frame in 0..10 {
        let view = track
            .view(base + std::time::Duration::from_millis(frame * 16), &LIGHT)
            .unwrap();
        assert!(view.head.position.is_finite() && view.head.orientation.is_finite());
    }
}

#[test]
fn a_track_with_nothing_usable_has_no_view_but_has_its_latest_report() {
    let mut track = Track::default();
    assert!(track.view(Instant::now(), &LIGHT).is_none());
    let unusable = report(Vec3::NAN, Quat::IDENTITY);
    track.push(0.0, unusable);
    assert!(track.view(Instant::now(), &LIGHT).is_none());
    assert!(
        track
            .latest()
            .head
            .position
            .iter()
            .all(|value| value.is_nan())
    );
    assert!(track.latest_view().head.position.is_nan());
}

#[test]
fn restarting_starts_the_view_over_where_the_participant_is() {
    let mut track = Track::default();
    let base = Instant::now();
    for count in 0..40 {
        track.push(f64::from(count) * 0.05, report_at(count));
    }
    for frame in 0..30 {
        track.view(base + std::time::Duration::from_millis(frame * 16), &LIGHT);
    }
    assert!(track.playhead.is_some() && track.filter.is_some() && track.drawn.is_some());
    track.restart();
    assert!(track.playhead.is_none() && track.filter.is_none() && track.drawn.is_none());
}
