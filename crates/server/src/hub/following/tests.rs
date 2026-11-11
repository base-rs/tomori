// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn a_glide_through_its_waypoint_is_followed_along_its_way_at_its_speed() {
    let now = Instant::now();
    let mut following = Following::default();
    let home = Pose::at([0.0, 1.4, -2.3]);
    // Through a waypoint 3 m up and then 4 m across, at its glide's speed.
    following.start(
        [0.0, 0.0, 0.0],
        Some(home),
        [4.0, 3.0, 0.0],
        Route::Via([0.0, 3.0, 0.0]),
        now,
    );
    assert_eq!(following.via, Some([0.0, 3.0, 0.0]));
    assert!(!following.stream);
    let glide = following.glide.clone().expect("gliding");
    assert!((glide.length() - 7.0).abs() < 1e-5);
    assert_eq!(glide.ends(), now + Duration::from_secs_f32(7.0 / GLIDE));
    let at = |seconds: f32| glide.at(now + Duration::from_secs_f32(seconds));
    assert!(follow::distance(at(0.0), [0.0, 0.0, 0.0]) < 1e-5);
    assert!(follow::distance(at(1.0 / GLIDE), [0.0, 1.0, 0.0]) < 1e-3);
    assert!(follow::distance(at(4.0 / GLIDE), [1.0, 3.0, 0.0]) < 1e-3);
    assert!(follow::distance(at(100.0), [4.0, 3.0, 0.0]) < 1e-5);
    // Where it stands follows the glide, facing as it will.
    let to = Pose::at([4.0, 3.0, 0.0]);
    let along = following.at(Some(to), home, now + Duration::from_secs_f32(1.0 / GLIDE));
    assert!(follow::distance(along.position, [0.0, 1.0, 0.0]) < 1e-3);
    assert_eq!(along.orientation, to.orientation);
    // Streamed, faster, with no waypoint.
    following.start([0.0; 3], None, [12.0, 0.0, 0.0], Route::Stream, now);
    assert_eq!((following.via, following.stream), (None, true));
    assert_eq!(
        following.deadline(),
        following.glide.as_ref().map(Glide::ends)
    );
    assert_eq!(
        following.glide.as_ref().map(Glide::ends),
        Some(now + Duration::from_secs_f32(12.0 / STREAMING))
    );
    // Without a glide it stands where it was summoned, or at home.
    let still = Following::default();
    assert_eq!(still.at(None, home, now), home);
    assert_eq!(still.deadline(), None);
}

#[test]
fn it_faces_whomever_it_glances_at_before_whoever_it_turned_to_and_speaks_until_its_last_frame_ends()
 {
    let now = Instant::now();
    let mut following = Following {
        facing: Some([1.0, 1.6, 0.0]),
        ..Following::default()
    };
    assert_eq!(following.faces(), Some([1.0, 1.6, 0.0]));
    following.glance = Some(([2.0, 1.6, 0.0], now + GLANCE));
    assert_eq!(following.faces(), Some([2.0, 1.6, 0.0]));
    assert_eq!(following.deadline(), Some(now + GLANCE));
    following.spoke(now, Duration::from_secs(2));
    following.spoke(now, Duration::from_secs(1));
    assert_eq!(
        following.speaking,
        Some(now + Duration::from_secs(2) + SPOKEN)
    );
}

#[test]
fn speech_still_to_come_counts_until_it_is_sent_or_nothing_of_it_moves_for_a_minute() {
    let now = Instant::now();
    let mut following = Following::default();
    assert!(!following.unspoken(now));
    following.queued(2, now);
    assert!(following.unspoken(now));
    // Synthesis that hangs ends the reply all the same, once nothing of its speech has moved for `STALLED`.
    assert!(following.unspoken(now + STALLED - Duration::from_millis(1)));
    assert!(!following.unspoken(now + STALLED));
    // A frame sent, or a segment sent whole, is progress, from which it counts again.
    let later = now + Duration::from_secs(50);
    following.spoke(later, Duration::from_millis(20));
    assert!(following.unspoken(later + Duration::from_secs(50)));
    following.sent(later + Duration::from_secs(55));
    assert!(following.unspoken(later + Duration::from_secs(100)));
    following.sent(later + Duration::from_secs(56));
    assert!(!following.unspoken(later + Duration::from_secs(56)));
}
