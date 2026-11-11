// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use serde_json::json;

fn tracker(events: &[(&str, Value, u64)]) -> Tracker {
    let mut tracker = Tracker::default();
    for (kind, data, at) in events {
        assert!(relevant(kind), "{kind}");
        tracker.apply(kind, data, *at);
    }
    tracker
}

fn enqueued(id: &str) -> Value {
    json!({"inboxID": id, "item": {"type": "user"}})
}

fn delivered(id: &str) -> Value {
    json!({"inboxID": id})
}

/// The message it has longest left unanswered, when it was sent and since when it counts.
fn unanswered(tracker: &Tracker) -> (Option<u64>, Option<u64>) {
    (tracker.current.message, tracker.current.unanswered_since)
}

/// This morning's wedge: a follow-up queued while the worker runs, which its current step streams past and its run
/// ends without taking up, is the worker's to answer from the run's end.
#[test]
fn a_follow_up_queued_behind_a_run_that_ends_without_taking_it_is_unanswered_from_the_end() {
    let mut tracker = tracker(&[
        ("session.execution.started", json!({}), 1_000),
        ("session.step.started", json!({}), 1_100),
        ("session.inbox.enqueued", enqueued("m1"), 2_000),
    ]);
    // Queued behind the run, it is not yet the worker's to answer.
    assert_eq!(unanswered(&tracker), (Some(2_000), None));
    // The current step streams and ends, and the run succeeds: none of that answers it.
    tracker.apply("session.text.delta", &json!({}), 3_000);
    tracker.apply("session.step.ended", &json!({"finish": "stop"}), 4_000);
    assert_eq!(unanswered(&tracker), (Some(2_000), None));
    tracker.apply("session.execution.succeeded", &json!({}), 5_000);
    assert_eq!(unanswered(&tracker), (Some(2_000), Some(5_000)));
    assert_eq!(tracker.current.progressed, 4_000);
}

#[test]
fn a_message_delivered_and_followed_by_the_run_ending_is_unanswered_from_its_delivery() {
    let tracker = tracker(&[
        ("session.execution.started", json!({}), 1_000),
        ("session.inbox.enqueued", enqueued("m1"), 1_500),
        ("session.inbox.delivered", delivered("m1"), 2_000),
        ("session.execution.succeeded", json!({}), 2_100),
    ]);
    assert_eq!(unanswered(&tracker), (Some(1_500), Some(2_000)));
    // A message delivered without being seen queued was sent as it was delivered; a second delivered while the first
    // waits leaves the first the longest unanswered.
    let tracker = self::tracker(&[
        ("session.inbox.delivered", delivered("m9"), 7_000),
        ("session.inbox.delivered", delivered("m10"), 8_000),
    ]);
    assert_eq!(unanswered(&tracker), (Some(7_000), Some(7_000)));
}

#[test]
fn a_message_is_answered_by_progress_after_its_delivery_and_not_by_the_step_it_arrived_in() {
    // To a worker known to be idle: it counts from its sending, and the first progress after its delivery answers it.
    let mut tracker = tracker(&[
        ("session.execution.succeeded", json!({}), 900),
        ("session.inbox.enqueued", enqueued("m1"), 1_000),
    ]);
    assert_eq!(unanswered(&tracker), (Some(1_000), Some(1_000)));
    tracker.apply("session.execution.started", &json!({}), 1_010);
    tracker.apply("session.inbox.delivered", &delivered("m1"), 1_020);
    assert_eq!(unanswered(&tracker), (Some(1_000), Some(1_020)));
    tracker.apply("session.reasoning.delta", &json!({}), 1_500);
    assert_eq!(unanswered(&tracker), (None, None));
    // Steered into a step that is going: that step streaming on is no answer; its end, then progress, is.
    tracker.apply("session.step.started", &json!({}), 2_000);
    tracker.apply("session.inbox.delivered", &delivered("m2"), 2_100);
    tracker.apply("session.text.delta", &json!({}), 2_200);
    assert_eq!(unanswered(&tracker), (Some(2_100), Some(2_100)));
    tracker.apply(
        "session.step.ended",
        &json!({"finish": "tool-calls"}),
        2_300,
    );
    tracker.apply("session.tool.success", &json!({"id": "t9"}), 2_400);
    assert_eq!(unanswered(&tracker), (None, None));
    // Or a step begun after it.
    tracker.apply("session.step.started", &json!({}), 3_000);
    tracker.apply("session.inbox.delivered", &delivered("m3"), 3_100);
    tracker.apply("session.step.started", &json!({}), 3_200);
    assert_eq!(unanswered(&tracker), (None, None));
    // A cancelled message is not waiting; a message other than a user's is not one to answer; one queued twice is one.
    tracker.apply("session.execution.succeeded", &json!({}), 3_300);
    tracker.apply("session.inbox.enqueued", &enqueued("m4"), 4_000);
    tracker.apply("session.inbox.enqueued", &enqueued("m4"), 4_100);
    assert_eq!(tracker.queued.len(), 1);
    tracker.apply("session.inbox.cancelled", &json!({"inboxID": "m4"}), 4_200);
    tracker.apply(
        "session.inbox.enqueued",
        &json!({"inboxID": "m5", "item": {"type": "synthetic"}}),
        4_300,
    );
    tracker.apply(
        "session.inbox.enqueued",
        &json!({"item": {"type": "user"}}),
        4_400,
    );
    tracker.apply("session.inbox.cancelled", &json!({}), 4_500);
    assert_eq!(unanswered(&tracker), (None, None));
}

#[test]
fn queued_messages_are_bounded_and_the_longest_waiting_is_reported() {
    let mut tracker = Tracker::default();
    for index in 0..MOST + 2 {
        tracker.apply(
            "session.inbox.enqueued",
            &enqueued(&format!("m{index}")),
            1_000 + index as u64,
        );
    }
    assert_eq!(tracker.queued.len(), MOST);
    // Whether a run is going is not known, so none is yet the worker's to answer.
    assert_eq!(unanswered(&tracker), (Some(1_002), None));
}

#[test]
fn a_worker_first_seen_now_has_progressed_no_earlier_than_now() {
    let tracker = tracker(&[("permission.replied", json!({"requestID": "p1"}), 9_000)]);
    assert_eq!(tracker.current.progressed, 9_000);
    let tracker = self::tracker(&[("session.created", json!({}), 1_000)]);
    assert_eq!(tracker.current.progressed, 1_000);
}

#[test]
fn a_worker_waits_while_any_of_its_requests_is_pending() {
    let mut tracker = tracker(&[
        ("permission.asked", json!({"id": "p1"}), 1_000),
        ("permission.asked", json!({"id": "p2"}), 1_000),
        ("permission.asked", json!({"id": "p2"}), 1_000),
        ("form.created", json!({"form": {"id": "f1"}}), 1_000),
        ("permission.asked", json!({}), 1_000),
    ]);
    assert_eq!(tracker.requests, ["p1", "p2", "f1"]);
    tracker.apply("permission.replied", &json!({"requestID": "p1"}), 2_000);
    assert!(tracker.current.waiting);
    tracker.apply("permission.replied", &json!({"requestID": "p2"}), 2_000);
    tracker.apply("form.replied", &json!({"id": "f1"}), 2_000);
    assert!(!tracker.current.waiting);
    tracker.apply("form.created", &json!({"form": {"id": "f2"}}), 3_000);
    tracker.apply("form.cancelled", &json!({"form": {"id": "f2"}}), 3_000);
    tracker.apply("form.cancelled", &json!({}), 3_000);
    assert!(!tracker.current.waiting);
    for index in 0..MOST + 1 {
        tracker.apply(
            "permission.asked",
            &json!({"id": format!("p{index}")}),
            4_000,
        );
    }
    assert_eq!(tracker.requests.len(), MOST);
    // A run that ends has nothing left pending.
    tracker.apply("session.execution.interrupted", &json!({}), 5_000);
    assert!(!tracker.current.waiting);
}

#[test]
fn a_running_tool_is_seen_until_it_ends() {
    let mut tracker = tracker(&[("session.step.started", json!({}), 4_000)]);
    // A call whose input is being written is not running; once called, it is, by name, until it ends.
    tracker.apply(
        "session.tool.input.started",
        &json!({"id": "t1", "name": "shell"}),
        5_000,
    );
    assert_eq!(tracker.current.tool, None);
    tracker.apply("session.tool.called", &json!({"id": "t1"}), 6_000);
    assert_eq!(
        tracker.current.tool,
        Some(RunningTool {
            name: "shell".into(),
            since: 6_000
        })
    );
    // Called again, a call keeps when it began.
    tracker.apply("session.tool.called", &json!({"id": "t1"}), 6_500);
    assert_eq!(tracker.current.tool.as_ref().unwrap().since, 6_000);
    // A call whose input names no tool, or a call never announced, is a tool all the same.
    tracker.apply("session.tool.input.started", &json!({"id": "t0"}), 5_500);
    tracker.apply("session.tool.called", &json!({"id": "t0"}), 5_600);
    assert_eq!(tracker.current.tool.as_ref().unwrap().name, "a tool");
    tracker.apply("session.tool.success", &json!({"id": "t0"}), 5_700);
    tracker.apply("session.tool.called", &json!({"id": "t2"}), 7_000);
    tracker.apply("session.tool.success", &json!({"id": "t1"}), 8_000);
    assert_eq!(
        tracker.current.tool,
        Some(RunningTool {
            name: "a tool".into(),
            since: 7_000
        })
    );
    tracker.apply("session.tool.failed", &json!({"id": "t2"}), 9_000);
    assert_eq!(tracker.current.tool, None);
    // An end for a call not known, or events without an id, change nothing.
    for kind in [
        "session.tool.input.started",
        "session.tool.called",
        "session.tool.success",
    ] {
        tracker.apply(kind, &json!({}), 9_500);
    }
    tracker.apply("session.tool.success", &json!({"id": "nobody"}), 9_600);
    assert!(tracker.calls.is_empty());
    tracker.apply(
        "session.step.ended",
        &json!({"finish": "tool-calls"}),
        10_000,
    );
    assert_eq!(tracker.current.finish.as_deref(), Some("tool-calls"));
    tracker.apply(
        "session.step.ended",
        &json!({"finish": "tool-calls"}),
        10_100,
    );
    // The calls a run made end with it, and are bounded.
    tracker.apply("session.tool.called", &json!({"id": "t3"}), 11_000);
    tracker.apply("session.execution.succeeded", &json!({}), 12_000);
    assert_eq!(tracker.current.tool, None);
    for index in 0..MOST + 4 {
        tracker.apply(
            "session.tool.input.started",
            &json!({"id": format!("i{index}"), "name": "read"}),
            13_000,
        );
    }
    assert_eq!(tracker.calls.len(), MOST);
}

#[test]
fn a_failure_says_why_until_the_worker_runs_again() {
    let mut tracker = tracker(&[
        (
            "session.step.failed",
            json!({"error": {"type": "BadRequestError", "message": "insufficient credit"}}),
            1_000,
        ),
        ("session.execution.failed", json!({}), 2_000),
    ]);
    assert_eq!(
        tracker.current.error.as_deref(),
        Some("BadRequestError: insufficient credit")
    );
    tracker.apply(
        "session.execution.failed",
        &json!({"error": {"message": "out of\ncredit"}}),
        3_000,
    );
    assert_eq!(tracker.current.error.as_deref(), Some("out of credit"));
    tracker.apply(
        "session.execution.failed",
        &json!({"error": {"type": "APIError", "message": "APIError: down"}}),
        3_500,
    );
    assert_eq!(tracker.current.error.as_deref(), Some("APIError: down"));
    tracker.apply("session.execution.started", &json!({}), 4_000);
    assert_eq!(tracker.current.error, None);
    // A step that fails without saying why keeps the reason known.
    tracker.apply("session.step.failed", &json!({}), 4_500);
    assert_eq!(tracker.current.error, None);
    tracker.apply("session.execution.failed", &json!({"error": {}}), 5_000);
    assert_eq!(
        tracker.current.error.as_deref(),
        Some("the agent's run failed")
    );
    for (error, said) in [
        (json!({"type": "Overloaded"}), Some("Overloaded")),
        (json!("boom"), Some("boom")),
        (json!({}), None),
    ] {
        assert_eq!(describe_error(Some(&error)).as_deref(), said);
    }
    assert_eq!(
        describe_error(Some(&json!({"message": "x".repeat(1_000)})))
            .unwrap()
            .chars()
            .count(),
        ERROR_CHARS
    );
    assert!(!relevant("session.usage.updated") && !relevant("server.connected"));
}

#[test]
fn the_records_say_what_a_worker_was_doing() {
    let messages = [
        json!({"type": "user", "time": {"created": 100}}),
        json!({"type": "assistant", "time": {"created": 200, "streamed": 250, "completed": 300}, "finish": "stop"}),
        json!({"type": "idle", "time": {"created": 310}}),
        json!({"type": "synthetic", "time": {"created": 320}}),
        // A follow-up delivered, and never answered.
        json!({"type": "user", "time": {"created": 400}}),
    ];
    let inbox = [
        json!({"id": "m9", "type": "user", "time": {"created": 500}}),
        json!({"type": "user", "time": {"created": 600}}),
        json!({"type": "synthetic", "time": {"created": 700}}),
    ];
    let tracker = Tracker::from_records((&messages, &inbox, &[]));
    assert_eq!(tracker.current.progressed, 300);
    assert_eq!(tracker.current.finish.as_deref(), Some("stop"));
    assert_eq!(unanswered(&tracker), (Some(400), Some(400)));
    assert_eq!(tracker.queued.len(), 2);
    assert_eq!(tracker.queued[0].since, Some(500));
    // Running a tool, in a message not yet completed, a message queued behind the run waits; one that failed says why.
    let messages = [
        json!({"type": "assistant", "time": {"created": 100}, "error": {"message": "overloaded"}, "content": [
            {"type": "text"},
            {"type": "tool", "name": "shell", "state": {"status": "running"}, "time": {"created": 110, "ran": 120}},
            {"type": "tool", "state": {"status": "running"}},
            {"type": "tool", "name": "read", "state": {"status": "completed"}},
        ]}),
    ];
    let tracker = Tracker::from_records((
        &messages,
        &[json!({"type": "user", "time": {"created": 150}})],
        &[],
    ));
    assert_eq!(
        tracker.current.tool,
        Some(RunningTool {
            name: "a tool".into(),
            since: 100
        })
    );
    assert_eq!(tracker.calls.len(), 2);
    assert_eq!(tracker.current.error.as_deref(), Some("overloaded"));
    assert_eq!(unanswered(&tracker), (Some(150), None));
    // Records with no times say nothing of progress; of two messages unanswered, the first is the longest waiting.
    let tracker = Tracker::from_records((
        &[
            json!({"type": "user"}),
            json!({"type": "user", "time": {"created": 9}}),
        ],
        &[],
        &[],
    ));
    assert_eq!(tracker.current.progressed, 0);
    assert_eq!(unanswered(&tracker), (Some(0), Some(0)));
}

#[test]
fn after_the_stream_is_lost_nothing_is_reported_until_the_records_are_read_again() {
    let mut workers = Workers::default();
    workers.apply("w1", "session.execution.started", &json!({}), 1_000);
    workers.apply("w1", "session.tool.called", &json!({"id": "t1"}), 1_000);
    workers.apply("w2", "session.execution.started", &json!({}), 2_000);
    workers.apply("w3", "session.execution.started", &json!({}), 500);
    workers.report(|_| true, |_, _| true);
    let mut synced = workers.unsync();
    synced.sort();
    assert_eq!(synced, ["w1", "w2", "w3"]);
    assert!(!workers.is_dirty());
    // An event during the read is held back.
    workers.apply("w2", "session.text.delta", &json!({}), 3_000);
    assert!(!workers.is_dirty());
    // The success missed in the gap is in the records: the tool no longer runs.
    let messages = [
        json!({"type": "assistant", "time": {"created": 1_000, "completed": 1_500}, "finish": "stop"}),
    ];
    workers.sync("w1", Some((&messages, &[], &[])), 4_000);
    // w2's own events since are newer than its records, and keep the records' progress if later.
    let later = [json!({"type": "assistant", "time": {"created": 3_500}})];
    workers.sync("w2", Some((&later, &[], &[])), 4_000);
    // w3's records could not be read: it starts again from now.
    workers.sync("w3", None, 4_000);
    workers.sync("unknown", None, 4_000);
    let mut told = Vec::new();
    workers.report(
        |_| true,
        |id, activity| {
            told.push((id.to_owned(), activity));
            true
        },
    );
    told.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(told.len(), 3);
    assert_eq!(told[0].1.tool, None);
    assert_eq!(told[0].1.progressed, 1_500);
    assert_eq!(told[1].1.progressed, 3_500);
    assert_eq!(told[2].1.progressed, 4_000);
}

#[test]
fn only_the_workers_last_active_are_read_again() {
    let mut workers = Workers::default();
    for index in 0..MOST_SYNCED + 3 {
        workers.apply(
            &format!("w{index}"),
            "session.created",
            &json!({}),
            1_000 + index as u64,
        );
    }
    let synced = workers.unsync();
    assert_eq!(synced.len(), MOST_SYNCED);
    assert!(!synced.contains(&"w0".to_owned()));
    assert_eq!(workers.trackers.len(), MOST_SYNCED);
}

#[test]
fn a_worker_is_reported_as_it_changes_and_as_its_progress_moves_on() {
    let mut workers = Workers::default();
    workers.apply("w1", "session.created", &json!({}), 1_000);
    assert!(workers.is_dirty());
    let mut told = Vec::new();
    workers.report(
        |_| true,
        |id, activity| {
            told.push((id.to_owned(), activity));
            true
        },
    );
    assert_eq!(told.len(), 1);
    assert!(!workers.is_dirty());
    // Progress within the step is not reported again; half a minute more is.
    workers.apply("w1", "session.text.delta", &json!({}), 20_000);
    assert!(!workers.is_dirty());
    workers.apply("w1", "session.text.delta", &json!({}), 31_000);
    assert!(workers.is_dirty());
    workers.apply("w1", "session.text.delta", &json!({}), 32_000);
    // Refused, it waits, with every worker after it.
    workers.apply("w2", "session.created", &json!({}), 31_000);
    workers.report(|_| true, |_, _| false);
    assert!(workers.is_dirty());
    // A worker no longer known is forgotten.
    workers.report(|id| id != "w2", |_, _| true);
    assert!(!workers.is_dirty());
    assert!(!workers.trackers.contains_key("w2"));
    workers.apply("w1", "permission.asked", &json!({"id": "p1"}), 32_000);
    workers.forget("w1");
    assert!(!workers.is_dirty());
    // A tracker with nothing new to say, or none at all, is passed over.
    workers.dirty.insert("w3".into());
    workers.trackers.insert("w3".into(), {
        let mut tracker = Tracker::default();
        tracker.reported();
        tracker
    });
    workers.dirty.insert("w4".into());
    workers.apply("w5", "session.created", &json!({}), 40_000);
    let mut asked = Vec::new();
    workers.report(
        |_| true,
        |id, _| {
            asked.push(id.to_owned());
            true
        },
    );
    assert_eq!(asked, ["w5"]);
    assert!(!workers.is_dirty());
    assert!(now() > 0);
    assert_eq!(bounded("a  b\nc", 10), "a b c");
}

/// A worker first seen after a restart by a message queued for it is not taken as idle: whether it is running is read
/// from its records, and the events held meanwhile apply over them.
#[test]
fn a_worker_first_seen_by_a_queued_message_is_read_before_it_is_judged() {
    let mut workers = Workers::default();
    assert!(workers.apply("w1", "session.inbox.enqueued", &enqueued("m1"), 1_000));
    assert!(!workers.apply("w1", "session.text.delta", &json!({}), 1_100));
    workers.apply("w1", "session.text.delta", &json!({}), 1_200);
    workers.apply("w1", "session.tool.called", &json!({"id": "t1"}), 1_300);
    // Created before Tomori's eyes, it is known from the start.
    assert!(!workers.apply("w2", "session.created", &json!({}), 1_000));
    assert!(!workers.trackers["w1"].due());
    assert_eq!(workers.trackers["w1"].held.as_ref().unwrap().len(), 3);
    // Its records say a run is going: the message waits behind it, the tool called meanwhile runs.
    let messages = [json!({"type": "assistant", "time": {"created": 500}})];
    workers.sync("w1", Some((&messages, &[], &[])), 2_000);
    let tracker = &workers.trackers["w1"];
    assert_eq!(unanswered(tracker), (Some(1_000), None));
    assert_eq!(tracker.current.progressed, 1_300);
    assert_eq!(tracker.current.tool.as_ref().unwrap().since, 1_300);
    assert!(tracker.due());
}

#[test]
fn too_many_events_held_keep_what_they_said() {
    let mut tracker = Tracker::default();
    tracker.unsync();
    tracker.apply("session.execution.started", &json!({}), 10);
    for index in 0..HELD + 1 {
        tracker.apply(
            "permission.asked",
            &json!({"id": format!("p{index}")}),
            20 + index as u64,
        );
    }
    assert!(tracker.held.is_none());
    tracker.apply("session.text.delta", &json!({}), 5_000);
    let messages = [json!({"type": "assistant", "time": {"created": 1, "completed": 9_000}})];
    tracker.sync(Some((&messages, &[], &[])), 10_000);
    assert_eq!(tracker.current.progressed, 9_000);
    assert!(tracker.current.waiting);
    assert_eq!(tracker.running, Some(true));
    // A worker whose records could not be read keeps what its events said.
    let mut tracker = Tracker::default();
    tracker.unsync();
    tracker.apply("session.text.delta", &json!({}), 300);
    tracker.sync(None, 10_000);
    assert_eq!(tracker.current.progressed, 300);
}

/// A worker waiting on a permission across a gap still waits once its records are read.
#[test]
fn the_records_say_which_requests_a_worker_waits_on() {
    let requests = [json!({"id": "p1"}), json!({"id": "f1"}), json!({})];
    let tracker = Tracker::from_records((&[], &[], &requests));
    assert_eq!(tracker.requests, ["p1", "f1"]);
    assert!(tracker.current.waiting);
    let many: Vec<Value> = (0..MOST + 2)
        .map(|index| json!({"id": format!("p{index}")}))
        .collect();
    assert_eq!(
        Tracker::from_records((&[], &[], &many)).requests.len(),
        MOST
    );
}

/// A delivery whose id the queue does not know delivers the oldest queued, as the inbox delivers in order.
#[test]
fn a_delivery_by_an_unknown_id_takes_the_oldest_queued() {
    let tracker = tracker(&[
        ("session.execution.started", json!({}), 100),
        ("session.inbox.enqueued", enqueued("m1"), 200),
        ("session.inbox.enqueued", enqueued("m2"), 300),
        ("session.inbox.delivered", delivered("other"), 400),
    ]);
    assert_eq!(tracker.queued.len(), 1);
    assert_eq!(tracker.queued[0].id, "m2");
    assert_eq!(unanswered(&tracker), (Some(200), Some(400)));
}
