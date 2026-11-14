// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A client's cache against the fake: identical calls in flight together sharing a request, answers kept while fresh and instant queries' times rounded to a bucket, range queries asking only for the steps they lack, partial answers never kept, and the cache's bounds.
use std::sync::mpsc::TryRecvError;

use super::*;

const MINUTE: Duration = Duration::from_secs(60);

/// A moment on a minute's boundary, from which the ranges' windows are counted.
fn base() -> i64 {
    1_700_000_040_000
}

fn minutes(count: i64) -> Timestamp {
    Timestamp::from_millis(base() + count * 60_000)
}

/// A range of one point a minute, from `start` to `end` minutes past [`base`].
fn window(start: i64, end: i64) -> Range {
    Range::new(minutes(start), minutes(end), MINUTE)
}

/// A matrix of `series`, each its labels and its values, one a minute from `start` minutes past [`base`].
fn matrix(series: &[(Value, i64, &[f64])]) -> Value {
    let result: Vec<Value> = series
        .iter()
        .map(|(metric, start, values)| {
            let values: Vec<Value> = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    json!([minutes(start + index as i64).seconds(), value.to_string()])
                })
                .collect();
            json!({"metric": metric, "values": values})
        })
        .collect();
    json!({"resultType": "matrix", "result": result})
}

/// The values of each series, by its labels' `pod`, and the minute past [`base`] of its first.
fn values(series: &[Series]) -> Vec<(String, i64, Vec<f64>)> {
    series
        .iter()
        .map(|series| {
            let first = (series.points[0].at.millis() - base()) / 60_000;
            for (index, point) in series.points.iter().enumerate() {
                assert_eq!(
                    point.at,
                    minutes(first + index as i64),
                    "every step present"
                );
            }
            (
                series.metric.get("pod").cloned().unwrap_or_default(),
                first,
                series.points.iter().map(|point| point.value).collect(),
            )
        })
        .collect()
}

/// The `start` and `end` a range query asked for, in minutes past [`base`].
fn asked(seen: &Seen) -> (i64, i64) {
    let fields = form(&seen.body);
    let field = |name: &str| {
        let seconds: f64 = fields
            .iter()
            .find(|(field, _)| field == name)
            .unwrap()
            .1
            .parse()
            .unwrap();
        ((seconds * 1000.0).round() as i64 - base()) / 60_000
    };
    (field("start"), field("end"))
}

fn time(seen: &Seen) -> String {
    form(&seen.body)
        .into_iter()
        .find(|(field, _)| field == "time")
        .unwrap()
        .1
}

fn partial(fake: &Fake, data: Value) {
    fake.reply(
        200,
        "",
        &json!({"status": "success", "data": data, "warnings": ["a store could not be reached"]})
            .to_string(),
    );
}

fn nothing_sent(fake: &Fake) {
    assert_eq!(fake.seen.try_recv().err(), Some(TryRecvError::Empty));
}

fn pod(name: &str) -> Value {
    json!({"pod": name})
}

/// Wait until `client`'s cache has had `count` calls follow another's request.
fn until_deduped(client: &Client, count: u64) {
    while client.cache_stats().deduped < count {
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn identical_calls_in_flight_together_share_one_request() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let callers: Vec<_> = (0..3)
        .map(|_| {
            let client = client.clone();
            thread::spawn(move || client.query("up", at()))
        })
        .collect();
    fake.next();
    until_deduped(&client, 2);
    fake.success(vector());
    for caller in callers {
        let samples = caller.join().unwrap().unwrap().data.into_vector().unwrap();
        assert_eq!(samples.len(), 2);
    }
    nothing_sent(&fake);
    let stats = client.cache_stats();
    assert_eq!((stats.misses, stats.deduped, stats.hits), (1, 2, 0));
    // Without a fresh time, nothing is kept: the next call asks again.
    assert_eq!(stats.entries, 0);
    fake.success(vector());
    client.query("up", at()).unwrap();
    fake.next();

    // An error is shared with the calls that waited for it, and never kept.
    let callers: Vec<_> = (0..2)
        .map(|_| {
            let client = client.clone();
            thread::spawn(move || {
                client.query_with("up", at(), Call::new().fresh(Duration::from_secs(60)))
            })
        })
        .collect();
    fake.next();
    until_deduped(&client, 3);
    fake.reply(503, "", "unavailable");
    for caller in callers {
        assert!(matches!(
            caller.join().unwrap(),
            Err(Error::Status { status: 503, .. })
        ));
    }
    fake.success(vector());
    client
        .query_with("up", at(), Call::new().fresh(Duration::from_secs(60)))
        .unwrap();
    fake.next();
}

#[test]
fn a_range_query_in_flight_is_shared_with_an_identical_one() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let callers: Vec<_> = (0..2)
        .map(|_| {
            let client = client.clone();
            thread::spawn(move || client.query_range("up", window(0, 2)))
        })
        .collect();
    fake.next();
    until_deduped(&client, 1);
    fake.success(matrix(&[(pod("a"), 0, &[1.0, 2.0, 3.0])]));
    for caller in callers {
        let series = caller.join().unwrap().unwrap().data;
        assert_eq!(values(&series), [("a".into(), 0, vec![1.0, 2.0, 3.0])]);
    }
    nothing_sent(&fake);
}

#[test]
fn a_follower_waits_no_longer_than_its_own_time_out() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let leader = {
        let client = client.clone();
        thread::spawn(move || client.query("up", at()))
    };
    fake.next();
    let quick = Call::new().timeout(Duration::from_millis(50));
    assert_eq!(
        client.query_with("up", at(), quick).unwrap_err(),
        Error::Timeout(Duration::from_millis(50))
    );
    fake.success(vector());
    leader.join().unwrap().unwrap();
}

#[test]
fn an_answer_is_kept_while_it_is_fresh() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    fake.success(vector());
    let first = client.query("up", at()).unwrap();
    fake.next();
    let second = client.query("up", at()).unwrap();
    // NaN is no value's equal, its own included.
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    nothing_sent(&fake);
    // Another time or expression is another answer.
    fake.success(vector());
    client
        .query("up", Timestamp::from_millis(at().millis() + 1))
        .unwrap();
    fake.next();
    let stats = client.cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.entries), (1, 2, 2));
    assert!(stats.bytes > 0);

    // A call may say its answer is fresh for less, here not at all.
    fake.success(vector());
    client
        .query_with("up", at(), Call::new().fresh(Duration::ZERO))
        .unwrap();
    fake.next();

    // An answer older than a call's fresh time is asked for again.
    let brief = Call::new().fresh(Duration::from_millis(20));
    thread::sleep(Duration::from_millis(40));
    fake.success(vector());
    client.query_with("up", at(), brief).unwrap();
    fake.next();
}

#[test]
fn an_instant_query_in_a_bucket_is_evaluated_at_its_start_and_read_once_a_bucket() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let bucket = Call::new().bucket(Duration::from_secs(15));
    let start = Timestamp::from_millis(1_700_000_010_000);
    fake.success(vector());
    client
        .query_with(
            "up",
            Timestamp::from_millis(start.millis() + 2_500),
            bucket.clone(),
        )
        .unwrap();
    assert_eq!(time(&fake.next()), "1700000010.000");
    for offset in [0, 7_000, 14_999] {
        client
            .query_with(
                "up",
                Timestamp::from_millis(start.millis() + offset),
                bucket.clone(),
            )
            .unwrap();
    }
    nothing_sent(&fake);
    fake.success(vector());
    client
        .query_with(
            "up",
            Timestamp::from_millis(start.millis() + 15_000),
            bucket.clone(),
        )
        .unwrap();
    assert_eq!(time(&fake.next()), "1700000025.000");
    assert_eq!(client.cache_stats().hits, 3);

    // A client without a cache still rounds the time down, though it keeps nothing.
    let plain = fake.client().build().unwrap();
    for _ in 0..2 {
        fake.success(vector());
        plain
            .query_with(
                "up",
                Timestamp::from_millis(start.millis() + 4_000),
                bucket.clone(),
            )
            .unwrap();
        assert_eq!(time(&fake.next()), "1700000010.000");
    }
    assert_eq!(plain.cache_stats(), CacheStats::default());
}

#[test]
fn a_moving_window_asks_only_for_its_newest_steps_with_an_overlap() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    fake.success(matrix(&[
        (pod("a"), 0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]),
        (pod("b"), 2, &[20.0, 30.0, 40.0, 50.0]),
    ]));
    let first = client.query_range("up", window(0, 5)).unwrap();
    assert_eq!(asked(&fake.next()), (0, 5));
    assert_eq!(values(&first.data).len(), 2);

    // Two minutes on: the last minute kept, which a late sample changed, and the two new ones. `b` stopped reporting, and `c` began.
    fake.success(matrix(&[
        (pod("a"), 5, &[5.5, 6.0, 7.0]),
        (pod("c"), 6, &[60.0, 70.0]),
    ]));
    let moved = client.query_range("up", window(2, 7)).unwrap();
    assert_eq!(asked(&fake.next()), (5, 7));
    assert_eq!(
        values(&moved.data),
        [
            ("a".into(), 2, vec![2.0, 3.0, 4.0, 5.5, 6.0, 7.0]),
            ("b".into(), 2, vec![20.0, 30.0, 40.0]),
            ("c".into(), 6, vec![60.0, 70.0]),
        ]
    );
    assert!(!moved.partial);
    let stats = client.cache_stats();
    assert_eq!((stats.misses, stats.tails, stats.entries), (1, 1, 1));

    // The same window again, with no fresh time, asks for its overlap again.
    fake.success(matrix(&[(pod("a"), 6, &[6.0, 7.5])]));
    let again = client.query_range("up", window(2, 7)).unwrap();
    assert_eq!(asked(&fake.next()), (6, 7));
    // `c`, not in the newest steps' answer, has no points left.
    assert_eq!(
        values(&again.data),
        [
            ("a".into(), 2, vec![2.0, 3.0, 4.0, 5.5, 6.0, 7.5]),
            ("b".into(), 2, vec![20.0, 30.0, 40.0]),
        ]
    );

    // A narrower window within what is kept: points outside it are left out.
    fake.success(matrix(&[(pod("a"), 7, &[7.5, 8.0])]));
    let narrow = client.query_range("up", window(5, 8)).unwrap();
    assert_eq!(asked(&fake.next()), (7, 8));
    assert_eq!(
        values(&narrow.data),
        [("a".into(), 5, vec![5.5, 6.0, 7.5, 8.0])]
    );
}

#[test]
fn a_window_kept_and_fresh_is_answered_without_a_request() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0, 3.0])]));
    client.query_range("up", window(0, 3)).unwrap();
    fake.next();
    // Within it, ending part way to the next step.
    let range = Range::new(
        minutes(1),
        Timestamp::from_millis(minutes(3).millis() + 30_000),
        MINUTE,
    );
    let kept = client.query_range("up", range).unwrap();
    nothing_sent(&fake);
    assert_eq!(values(&kept.data), [("a".into(), 1, vec![1.0, 2.0, 3.0])]);
    assert_eq!(client.cache_stats().hits, 1);
    // A read ending earlier leaves the newest points as fresh as they were.
    let brief = Call::new().fresh(Duration::from_millis(20));
    thread::sleep(Duration::from_millis(40));
    fake.success(matrix(&[(pod("a"), 1, &[1.0, 2.0])]));
    client
        .query_range_with("up", window(0, 2), brief.clone())
        .unwrap();
    assert_eq!(asked(&fake.next()), (1, 2));
    fake.success(matrix(&[(pod("a"), 2, &[2.0, 3.0])]));
    client.query_range_with("up", window(0, 3), brief).unwrap();
    assert_eq!(asked(&fake.next()), (2, 3));
    // A step it lacks is asked for, even while it is fresh.
    fake.success(matrix(&[(pod("a"), 3, &[3.0, 4.0])]));
    client.query_range("up", window(1, 4)).unwrap();
    assert_eq!(asked(&fake.next()), (3, 4));
}

#[test]
fn a_gap_an_earlier_start_another_step_or_phase_asks_for_the_whole_window() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let read = |range: Range, data: Value| {
        fake.success(data);
        let response = client.query_range("up", range).unwrap();
        (fake.next(), response)
    };
    read(
        window(0, 3),
        matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0, 3.0])]),
    );
    // A window starting after the step after the last kept leaves a gap: it is read whole.
    let (seen, gap) = read(window(5, 6), matrix(&[(pod("a"), 5, &[5.0, 6.0])]));
    assert_eq!(asked(&seen), (5, 6));
    assert_eq!(values(&gap.data), [("a".into(), 5, vec![5.0, 6.0])]);
    // One starting before what is kept is read whole too.
    let (seen, _) = read(window(4, 6), matrix(&[(pod("a"), 4, &[4.0, 5.0, 6.0])]));
    assert_eq!(asked(&seen), (4, 6));
    // Another step is kept apart.
    let half = Range::new(minutes(4), minutes(6), Duration::from_secs(30));
    fake.success(matrix(&[]));
    client.query_range("up", half).unwrap();
    let seen = fake.next();
    assert_eq!(asked(&seen), (4, 6));
    assert!(form(&seen.body).contains(&("step".into(), "30.000".into())));
    // As are steps that do not coincide with the kept ones.
    let shifted = Range::new(
        Timestamp::from_millis(minutes(4).millis() + 1_000),
        Timestamp::from_millis(minutes(7).millis() + 1_000),
        MINUTE,
    );
    fake.success(matrix(&[]));
    client.query_range("up", shifted).unwrap();
    let fields = form(&fake.next().body);
    assert!(fields.contains(&("start".into(), "1700000281.000".into())));
    // And the kept window at the minute step is still used.
    let (seen, kept) = read(window(4, 7), matrix(&[(pod("a"), 6, &[6.0, 7.0])]));
    assert_eq!(asked(&seen), (6, 7));
    assert_eq!(
        values(&kept.data),
        [("a".into(), 4, vec![4.0, 5.0, 6.0, 7.0])]
    );
    assert_eq!(client.cache_stats().entries, 3);
}

#[test]
fn a_partial_answer_is_returned_flagged_and_never_kept() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    partial(&fake, vector());
    let response = client.query("up", at()).unwrap();
    assert!(response.partial);
    assert_eq!(response.warnings, ["a store could not be reached"]);
    fake.next();
    fake.success(vector());
    assert!(!client.query("up", at()).unwrap().partial);
    fake.next();
    client.query("up", at()).unwrap();
    nothing_sent(&fake);

    // A range's tail answered partially: returned with the points kept, which are then dropped, so that the next read asks for the whole window.
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0])]));
    client.query_range("up", window(0, 2)).unwrap();
    fake.next();
    partial(&fake, matrix(&[]));
    let response = client.query_range("up", window(0, 4)).unwrap();
    assert_eq!(asked(&fake.next()), (2, 4));
    assert!(response.partial);
    assert_eq!(values(&response.data), [("a".into(), 0, vec![0.0, 1.0])]);
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0, 3.0, 4.0])]));
    let whole = client.query_range("up", window(0, 4)).unwrap();
    assert_eq!(asked(&fake.next()), (0, 4));
    assert!(!whole.partial);
    let stats = client.cache_stats();
    assert_eq!(stats.partial, 2);
}

#[test]
fn series_a_label_filter_leaves_alike_stay_apart_across_reads() {
    let fake = Fake::start();
    let client = fake
        .client()
        .labels(LabelFilter::only(["pod"]))
        .cache(Cache::new())
        .build()
        .unwrap();
    let a = json!({"pod": "a", "zone": "1"});
    let b = json!({"pod": "a", "zone": "2"});
    fake.success(matrix(&[
        (a.clone(), 0, &[1.0, 1.0]),
        (b.clone(), 0, &[2.0, 2.0]),
    ]));
    client.query_range("up", window(0, 1)).unwrap();
    fake.next();
    fake.success(matrix(&[(a, 1, &[1.0, 1.0]), (b, 1, &[2.0, 2.0])]));
    let read = client.query_range("up", window(0, 2)).unwrap();
    assert_eq!(
        values(&read.data),
        [
            ("a".into(), 0, vec![1.0, 1.0, 1.0]),
            ("a".into(), 0, vec![2.0, 2.0, 2.0]),
        ]
    );
}

#[test]
fn answers_read_with_different_label_filters_are_kept_apart() {
    let fake = Fake::start();
    let client = fake
        .client()
        .labels(LabelFilter::only(["pod"]))
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    let zone = Call::new().labels(LabelFilter::only(["pod", "zone"]));
    let metric = json!({"pod": "a", "zone": "1"});
    fake.success(matrix(&[(metric.clone(), 0, &[1.0, 2.0])]));
    let plain = client.query_range("up", window(0, 1)).unwrap();
    fake.next();
    // The same range with another filter is not answered from what the client's filter kept.
    fake.success(matrix(&[(metric.clone(), 0, &[1.0, 2.0])]));
    let zoned = client
        .query_range_with("up", window(0, 1), zone.clone())
        .unwrap();
    assert_eq!(asked(&fake.next()), (0, 1));
    assert_eq!(plain.data[0].metric.len(), 1);
    assert_eq!(zoned.data[0].metric.len(), 2);
    // Each is kept under its own filter, and a call naming the client's own filter shares the client's answers.
    let again = client
        .query_range_with("up", window(0, 1), zone.clone())
        .unwrap();
    assert_eq!(again.data[0].metric.len(), 2);
    let same = Call::new().labels(LabelFilter::only(["pod"]));
    let again = client.query_range_with("up", window(0, 1), same).unwrap();
    assert_eq!(again.data[0].metric.len(), 1);
    nothing_sent(&fake);
    // A moving window asks only for the steps its own filter's answer lacks.
    fake.success(matrix(&[(metric.clone(), 1, &[2.0, 3.0])]));
    let moved = client
        .query_range_with("up", window(0, 2), zone.clone())
        .unwrap();
    assert_eq!(asked(&fake.next()), (1, 2));
    assert_eq!(moved.data[0].metric.len(), 2);
    // Instant queries too.
    fake.success(vector());
    client.query("up", at()).unwrap();
    fake.next();
    fake.success(vector());
    client.query_with("up", at(), zone.clone()).unwrap();
    fake.next();
    client.query_with("up", at(), zone).unwrap();
    client.query("up", at()).unwrap();
    nothing_sent(&fake);
    let stats = client.cache_stats();
    assert_eq!((stats.entries, stats.misses, stats.tails), (4, 4, 1));

    // A filter's names count towards what an answer weighs.
    let weighed = |filter: LabelFilter| {
        let cache = Cache::new().fresh(Duration::from_secs(60));
        let client = fake.client().cache(cache).build().unwrap();
        fake.success(vector());
        client
            .query_with("up", at(), Call::new().labels(filter))
            .unwrap();
        fake.next();
        client.cache_stats().bytes
    };
    let long = (0..16).map(|index| format!("label_{index}_{}", "x".repeat(64)));
    assert!(weighed(LabelFilter::only(long)) > weighed(LabelFilter::only(["pod"])) + 16 * 64);
}

#[test]
fn identical_calls_in_flight_together_with_different_filters_send_their_own_requests() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let call = |name: &'static str| {
        let client = client.clone();
        thread::spawn(move || {
            client.query_with("up", at(), Call::new().labels(LabelFilter::only([name])))
        })
    };
    let pods = call("pod");
    fake.next();
    // Asked while the first is in flight, the second is not answered by it: it sends its own.
    let urls = call("url");
    fake.next();
    fake.success(vector());
    fake.success(vector());
    let pods = pods.join().unwrap().unwrap().data.into_vector().unwrap();
    let urls = urls.join().unwrap().unwrap().data.into_vector().unwrap();
    assert!(pods[0].metric.contains_key("pod") && !pods[0].metric.contains_key("url"));
    assert!(urls[0].metric.contains_key("url") && !urls[0].metric.contains_key("pod"));
    let stats = client.cache_stats();
    assert_eq!((stats.misses, stats.deduped), (2, 0));
}

#[test]
fn points_remembered_with_a_call_s_label_filter_are_kept_under_it() {
    let fake = Fake::start();
    let client = fake
        .client()
        .labels(LabelFilter::only(["pod"]))
        .cache(Cache::new())
        .build()
        .unwrap();
    let zone = Call::new().labels(LabelFilter::only(["pod", "zone"]));
    fake.success(matrix(&[(
        json!({"pod": "a", "zone": "1"}),
        0,
        &[0.0, 1.0, 2.0],
    )]));
    let series = client
        .query_range_with("up", window(0, 2), zone.clone())
        .unwrap()
        .data;
    fake.next();
    let restarted = fake
        .client()
        .labels(LabelFilter::only(["pod"]))
        .cache(Cache::new())
        .build()
        .unwrap();
    restarted
        .remember_with("up", window(0, 2), &series, zone.clone())
        .unwrap();
    // Read with the filter it was remembered under, only the steps after it are asked for.
    fake.success(matrix(&[(
        json!({"pod": "a", "zone": "1"}),
        2,
        &[2.0, 3.0],
    )]));
    restarted
        .query_range_with("up", window(0, 3), zone)
        .unwrap();
    assert_eq!(asked(&fake.next()), (2, 3));
    // With the client's own filter, the whole window.
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0, 3.0])]));
    restarted.query_range("up", window(0, 3)).unwrap();
    assert_eq!(asked(&fake.next()), (0, 3));
}

#[test]
fn the_cache_keeps_within_its_bounds_dropping_the_least_recently_used() {
    let fake = Fake::start();
    let fresh = Cache::new().fresh(Duration::from_secs(60));
    let client = fake.client().cache(fresh.max_entries(2)).build().unwrap();
    let read = |expression: &str, sent: bool| {
        if sent {
            fake.success(vector());
        }
        client.query(expression, at()).unwrap();
        if sent {
            fake.next();
        } else {
            nothing_sent(&fake);
        }
    };
    read("one", true);
    read("two", true);
    read("one", false);
    read("three", true);
    let stats = client.cache_stats();
    assert_eq!((stats.entries, stats.evictions), (2, 1));
    read("one", false);
    read("two", true);
    read("one", false);

    // Bounded by bytes: an answer larger than the cache is never kept, and drops nothing.
    let scalar = |value: &str| json!({"resultType": "scalar", "result": [1, value]});
    let probe = fake.client().cache(fresh).build().unwrap();
    fake.success(scalar("1"));
    probe.query("1", at()).unwrap();
    fake.next();
    let one = probe.cache_stats().bytes;
    let small = fake
        .client()
        .cache(fresh.max_bytes(one * 3 / 2))
        .build()
        .unwrap();
    fake.success(scalar("1"));
    small.query("1", at()).unwrap();
    fake.next();
    let samples: Vec<Value> = (0..100)
        .map(|index| json!({"metric": {"pod": format!("web-{index}")}, "value": [1, "1"]}))
        .collect();
    fake.success(json!({"resultType": "vector", "result": samples}));
    small.query("up", at()).unwrap();
    fake.next();
    let stats = small.cache_stats();
    assert_eq!((stats.entries, stats.evictions, stats.bytes), (1, 0, one));
    // Two that fit only one at a time: the older is dropped.
    fake.success(scalar("2"));
    small.query("2", at()).unwrap();
    fake.next();
    let stats = small.cache_stats();
    assert_eq!((stats.entries, stats.evictions, stats.bytes), (1, 1, one));
    fake.success(scalar("1"));
    small.query("1", at()).unwrap();
    fake.next();

    // A cache of no entries keeps nothing.
    let none = fake.client().cache(fresh.max_entries(0)).build().unwrap();
    fake.success(vector());
    none.query("up", at()).unwrap();
    fake.next();
    assert_eq!(none.cache_stats().entries, 0);
}

#[test]
fn a_window_kept_holds_no_more_than_the_longest_asked_for() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0])]));
    client.query_range("up", window(0, 2)).unwrap();
    fake.next();
    let before = client.cache_stats().bytes;
    for end in 3..10 {
        fake.success(matrix(&[(pod("a"), end - 1, &[0.0, 0.0])]));
        client.query_range("up", window(end - 2, end)).unwrap();
        assert_eq!(asked(&fake.next()), (end - 1, end));
    }
    assert_eq!(client.cache_stats().bytes, before);
    // Overlapping nothing kept, the oldest points are gone: a window back in time is read whole.
    fake.success(matrix(&[]));
    client.query_range("up", window(3, 9)).unwrap();
    assert_eq!(asked(&fake.next()), (3, 9));
}

#[test]
fn a_cached_client_describes_its_cache() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::default().overlap(Duration::ZERO))
        .build()
        .unwrap();
    assert!(format!("{client:?}").contains("cache: Some(Cache { max_entries: 1024"));
    // Without an overlap, only the steps lacking are asked for.
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0])]));
    client.query_range("up", window(0, 1)).unwrap();
    fake.next();
    fake.success(matrix(&[(pod("a"), 2, &[2.0])]));
    client.query_range("up", window(0, 2)).unwrap();
    assert_eq!(asked(&fake.next()), (2, 2));
    // An overlap of part of a step is a whole step.
    let partly = fake
        .client()
        .cache(Cache::new().overlap(Duration::from_secs(1)))
        .build()
        .unwrap();
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0])]));
    partly.query_range("up", window(0, 1)).unwrap();
    fake.next();
    fake.success(matrix(&[(pod("a"), 1, &[1.0, 2.0])]));
    partly.query_range("up", window(0, 2)).unwrap();
    assert_eq!(asked(&fake.next()), (1, 2));
}

#[test]
fn a_leader_that_panics_fails_its_followers_rather_than_leave_them_waiting() {
    let fake = Fake::start();
    let (entered, entering) = channel();
    let (go, going) = channel::<()>();
    let (entered, going) = (Mutex::new(entered), Mutex::new(going));
    let client = fake
        .client()
        .cache(Cache::new())
        .token_source(move || -> Result<Token, String> {
            entered.lock().unwrap().send(()).unwrap();
            going.lock().unwrap().recv().unwrap();
            panic!("the token source failed");
        })
        .build()
        .unwrap();
    let leader = {
        let client = client.clone();
        thread::spawn(move || client.query("up", at()))
    };
    entering.recv().unwrap();
    let follower = {
        let client = client.clone();
        thread::spawn(move || client.query("up", at()))
    };
    until_deduped(&client, 1);
    go.send(()).unwrap();
    assert!(leader.join().is_err());
    assert_eq!(
        follower.join().unwrap().unwrap_err(),
        Error::Transport("the identical request this one waited for failed".into())
    );
    nothing_sent(&fake);
}

#[test]
fn a_range_query_in_flight_shares_its_failure_with_an_identical_one() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let callers: Vec<_> = (0..2)
        .map(|_| {
            let client = client.clone();
            thread::spawn(move || client.query_range("up", window(0, 2)))
        })
        .collect();
    fake.next();
    until_deduped(&client, 1);
    fake.reply(503, "", "unavailable");
    for caller in callers {
        assert!(matches!(
            caller.join().unwrap(),
            Err(Error::Status { status: 503, .. })
        ));
    }
    assert_eq!(client.cache_stats().entries, 0);
}

#[test]
fn an_overlap_as_long_as_the_window_asks_for_it_whole() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().overlap(Duration::from_secs(600)))
        .build()
        .unwrap();
    for _ in 0..2 {
        fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0])]));
        client.query_range("up", window(0, 3)).unwrap();
        assert_eq!(asked(&fake.next()), (0, 3));
    }
    let stats = client.cache_stats();
    assert_eq!((stats.misses, stats.tails, stats.entries), (2, 0, 1));
}

#[test]
fn answers_no_longer_fresh_are_dropped_as_others_are_kept() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    fake.success(vector());
    client
        .query_with("up", at(), Call::new().fresh(Duration::from_millis(20)))
        .unwrap();
    fake.next();
    thread::sleep(Duration::from_millis(40));
    fake.success(vector());
    client
        .query_with("down", at(), Call::new().fresh(Duration::from_secs(60)))
        .unwrap();
    fake.next();
    let stats = client.cache_stats();
    assert_eq!((stats.entries, stats.evictions), (1, 0));
}

#[test]
fn every_kind_of_answer_is_kept_with_its_own_warnings() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    let answers = [
        json!({"resultType": "vector", "result": [
            {"metric": {"le": "a"}, "histogram": [1, {"count": "2"}]},
            {"metric": {"pod": "web-1"}, "value": [1, "1"]},
        ]}),
        json!({"resultType": "matrix", "result": [{"metric": {}, "values": [[1, "1"]]}]}),
        json!({"resultType": "string", "result": [1, "text"]}),
    ];
    for (index, answer) in answers.into_iter().enumerate() {
        let expression = format!("q{index}");
        fake.success(answer);
        let first = client.query(&expression, at()).unwrap();
        fake.next();
        let kept = client.query(&expression, at()).unwrap();
        nothing_sent(&fake);
        assert_eq!(kept, first);
        assert!(!kept.partial);
    }
    assert_eq!(
        client.query("q0", at()).unwrap().warnings.len(),
        1,
        "the native histograms' warning is kept with the answer"
    );
    // A range's own warnings are kept with its points too.
    fake.success(json!({"resultType": "matrix", "result": [
        {"metric": {"pod": "a"}, "values": [[minutes(0).seconds(), "1"]], "histograms": [[minutes(0).seconds(), {"count": "1"}]]},
    ]}));
    client.query_range("up", window(0, 0)).unwrap();
    fake.next();
    let kept = client.query_range("up", window(0, 0)).unwrap();
    nothing_sent(&fake);
    assert_eq!(kept.warnings.len(), 1);
}

#[test]
fn points_kept_by_another_call_meanwhile_are_left_as_they_are() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)).max_entries(1))
        .build()
        .unwrap();
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0])]));
    client.query_range("up", window(0, 2)).unwrap();
    fake.next();
    // An instant query in flight first, then the window's newest steps.
    let instant = {
        let client = client.clone();
        thread::spawn(move || client.query("other", at()))
    };
    fake.next();
    let range = {
        let client = client.clone();
        thread::spawn(move || client.query_range("up", window(0, 4)))
    };
    assert_eq!(asked(&fake.next()), (2, 4));
    // The instant query's answer is kept first, which drops the window kept.
    fake.success(vector());
    instant.join().unwrap().unwrap();
    fake.success(matrix(&[(pod("a"), 2, &[2.0, 3.0, 4.0])]));
    let read = range.join().unwrap().unwrap();
    assert_eq!(
        values(&read.data),
        [("a".into(), 0, vec![0.0, 1.0, 2.0, 3.0, 4.0])]
    );
    // The newest steps alone are not kept in its place.
    let stats = client.cache_stats();
    assert_eq!((stats.entries, stats.evictions), (1, 1));
    fake.success(matrix(&[]));
    client.query_range("up", window(0, 4)).unwrap();
    assert_eq!(asked(&fake.next()), (0, 4));
}

#[test]
fn points_the_application_kept_itself_are_remembered_but_never_fresh() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    let series: Vec<Series> = {
        fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0, 3.0, 4.0])]));
        let read = fake.client().build().unwrap();
        let series = read.query_range("up", window(0, 4)).unwrap().data;
        fake.next();
        series
    };
    // Remembered over a narrower window: the points outside it are left out.
    client.remember("up", window(1, 3), &series).unwrap();
    assert_eq!(client.cache_stats().entries, 1);
    // Though the cache keeps answers fresh for a minute, what was remembered is read past its last step, with the overlap.
    fake.success(matrix(&[(pod("a"), 3, &[3.5, 4.0])]));
    let read = client.query_range("up", window(1, 4)).unwrap();
    assert_eq!(asked(&fake.next()), (3, 4));
    assert_eq!(
        values(&read.data),
        [("a".into(), 1, vec![1.0, 2.0, 3.5, 4.0])]
    );
    // And once read, is fresh.
    client.query_range("up", window(1, 4)).unwrap();
    nothing_sent(&fake);

    // A range that cannot be asked for is refused; a client without a cache keeps nothing.
    let second = Duration::from_secs(1);
    assert!(matches!(
        client.remember("up", Range::new(at(), at() - second, second), &series),
        Err(Error::Invalid(_))
    ));
    let plain = fake.client().build().unwrap();
    plain.remember("up", window(1, 3), &series).unwrap();
    assert_eq!(plain.cache_stats(), CacheStats::default());
}

#[test]
fn a_range_evaluated_at_its_start_or_end_is_read_whole_and_never_kept() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    let expression = "rate(x[5m] @ end())";
    for (start, end) in [(0, 3), (0, 3), (1, 4)] {
        fake.success(matrix(&[(pod("a"), start, &[1.0, 2.0, 3.0, 4.0])]));
        client.query_range(expression, window(start, end)).unwrap();
        assert_eq!(asked(&fake.next()), (start, end));
    }
    assert_eq!(client.cache_stats().entries, 0);
    // Nor remembered.
    let series = vec![Series {
        metric: Labels::new(),
        points: vec![Point {
            at: minutes(0),
            value: 1.0,
        }],
    }];
    client.remember(expression, window(0, 0), &series).unwrap();
    assert_eq!(client.cache_stats().entries, 0);
    // The same words in a label's value are no modifier.
    let plain = r#"up{note="@ end()"}"#;
    fake.success(matrix(&[(pod("a"), 0, &[1.0, 2.0])]));
    client.query_range(plain, window(0, 1)).unwrap();
    fake.next();
    client.query_range(plain, window(0, 1)).unwrap();
    nothing_sent(&fake);
}

#[test]
fn promql_s_own_annotations_leave_an_answer_kept() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    let annotated = |fake: &Fake, data: Value| {
        fake.reply(
            200,
            "",
            &json!({"status": "success", "data": data,
                "warnings": ["PromQL warning: metric might not be a counter, name does not end in _total/_sum/_count/_bucket: \"x\""],
                "infos": ["PromQL info: ignored a histogram"]})
            .to_string(),
        );
    };
    annotated(&fake, vector());
    let response = client.query("rate(x[5m])", at()).unwrap();
    assert!(!response.partial);
    assert_eq!(response.warnings.len(), 2);
    fake.next();
    assert_eq!(client.query("rate(x[5m])", at()).unwrap().warnings.len(), 2);
    nothing_sent(&fake);
    // A range's newest steps annotated: kept, the next read asking only for what is new.
    fake.success(matrix(&[(pod("a"), 0, &[0.0, 1.0, 2.0])]));
    client.query_range("rate(x[5m])", window(0, 2)).unwrap();
    fake.next();
    annotated(&fake, matrix(&[(pod("a"), 2, &[2.0, 3.0])]));
    let read = client.query_range("rate(x[5m])", window(0, 3)).unwrap();
    assert_eq!(asked(&fake.next()), (2, 3));
    assert!(!read.partial);
    client.query_range("rate(x[5m])", window(0, 3)).unwrap();
    nothing_sent(&fake);
    assert_eq!(client.cache_stats().partial, 0);
}

#[test]
fn remembered_points_must_be_on_the_steps_in_order_and_never_replace_those_read() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let series = |values: &[(i64, f64)]| {
        vec![Series {
            metric: Labels::from([("pod".to_owned(), "a".to_owned())]),
            points: values
                .iter()
                .map(|(minute, value)| Point {
                    at: minutes(*minute),
                    value: *value,
                })
                .collect(),
        }]
    };
    let off_step = vec![Series {
        metric: Labels::new(),
        points: vec![Point {
            at: Timestamp::from_millis(minutes(1).millis() + 30_000),
            value: 1.0,
        }],
    }];
    assert!(matches!(
        client.remember("up", window(0, 3), &off_step),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        client.remember("up", window(0, 3), &series(&[(2, 1.0), (1, 1.0)])),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        client.remember("up", window(0, 3), &series(&[(1, 1.0), (1, 1.0)])),
        Err(Error::Invalid(_))
    ));
    assert_eq!(client.cache_stats().entries, 0);
    // Read, then remembered over more: the steps read are kept as read, the others added.
    fake.success(matrix(&[(pod("a"), 2, &[2.0, 3.0])]));
    client.query_range("up", window(2, 3)).unwrap();
    fake.next();
    let saved: Vec<(i64, f64)> = (0..=5).map(|minute| (minute, -1.0)).collect();
    client
        .remember("up", window(0, 5), &series(&saved))
        .unwrap();
    fake.success(matrix(&[(pod("a"), 5, &[5.0, 6.0])]));
    let read = client.query_range("up", window(0, 6)).unwrap();
    assert_eq!(asked(&fake.next()), (5, 6));
    assert_eq!(
        values(&read.data),
        [("a".into(), 0, vec![-1.0, -1.0, 2.0, 3.0, -1.0, 5.0, 6.0])]
    );
    // Remembered within what was read: it stays as fresh as it was.
    let fresh = fake
        .client()
        .cache(Cache::new().fresh(Duration::from_secs(60)))
        .build()
        .unwrap();
    fake.success(matrix(&[(pod("a"), 2, &[2.0, 3.0])]));
    fresh.query_range("up", window(2, 3)).unwrap();
    fake.next();
    fresh
        .remember("up", window(0, 3), &series(&[(0, -1.0), (3, -1.0)]))
        .unwrap();
    let kept = fresh.query_range("up", window(2, 3)).unwrap();
    nothing_sent(&fake);
    assert_eq!(values(&kept.data), [("a".into(), 2, vec![2.0, 3.0])]);
    // Remembered apart from what is kept: what was read stays.
    client
        .remember("up", window(20, 21), &series(&[(20, 1.0)]))
        .unwrap();
    fake.success(matrix(&[(pod("a"), 6, &[6.0, 7.0])]));
    client.query_range("up", window(0, 7)).unwrap();
    assert_eq!(asked(&fake.next()), (6, 7));
}

#[test]
fn an_answer_kept_again_outlives_the_freshness_it_was_first_kept_for() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    fake.success(vector());
    client
        .query_with("up", at(), Call::new().fresh(Duration::from_millis(300)))
        .unwrap();
    fake.next();
    thread::sleep(Duration::from_millis(600));
    // Fresh for less than it has aged, so read again, and kept again for as long.
    let again = Call::new().fresh(Duration::from_millis(500));
    fake.success(vector());
    client.query_with("up", at(), again).unwrap();
    fake.next();
    // Another answer kept drops what is no longer fresh, but not this one, kept again since.
    fake.success(vector());
    client
        .query_with("other", at(), Call::new().fresh(Duration::from_secs(60)))
        .unwrap();
    fake.next();
    assert_eq!(client.cache_stats().entries, 2);
    client
        .query_with("up", at(), Call::new().fresh(Duration::from_secs(60)))
        .unwrap();
    nothing_sent(&fake);
}

#[test]
fn a_window_kept_shrinks_once_no_longer_range_is_asked_of_it() {
    let fake = Fake::start();
    let client = fake
        .client()
        .cache(Cache::new().span_decay(Duration::from_millis(100)))
        .build()
        .unwrap();
    let values: Vec<f64> = (0..=10).map(f64::from).collect();
    fake.success(matrix(&[(pod("a"), 0, &values)]));
    client.query_range("up", window(0, 10)).unwrap();
    fake.next();
    let long = client.cache_stats().bytes;
    // A short range in the next period: the long one is still kept, asked for in the period before.
    thread::sleep(Duration::from_millis(150));
    fake.success(matrix(&[(pod("a"), 10, &[10.0, 11.0])]));
    client.query_range("up", window(10, 11)).unwrap();
    assert_eq!(asked(&fake.next()), (10, 11));
    fake.success(matrix(&[(pod("a"), 11, &[11.0, 12.0])]));
    client.query_range("up", window(2, 12)).unwrap();
    assert_eq!(asked(&fake.next()), (11, 12));
    // Two periods of short ranges alone: the window shrinks to them.
    thread::sleep(Duration::from_millis(150));
    fake.success(matrix(&[(pod("a"), 12, &[12.0, 13.0])]));
    client.query_range("up", window(12, 13)).unwrap();
    fake.next();
    thread::sleep(Duration::from_millis(150));
    fake.success(matrix(&[(pod("a"), 13, &[13.0, 14.0])]));
    client.query_range("up", window(13, 14)).unwrap();
    fake.next();
    assert!(client.cache_stats().bytes < long);
    fake.success(matrix(&[]));
    client.query_range("up", window(5, 14)).unwrap();
    assert_eq!(asked(&fake.next()), (5, 14));
}

#[test]
fn a_call_waits_only_for_a_request_of_its_priority_and_a_time_out_as_long() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let spawn = |call: Call| {
        let client = client.clone();
        thread::spawn(move || client.query_with("up", at(), call))
    };
    // A background request in flight: a foreground call sends its own.
    let background = spawn(Call::new().background());
    fake.next();
    let foreground = spawn(Call::new());
    fake.next();
    fake.success(vector());
    fake.success(vector());
    background.join().unwrap().unwrap();
    foreground.join().unwrap().unwrap();
    // A request with a shorter time-out in flight: a call with a longer one sends its own.
    let short = spawn(Call::new().timeout(Duration::from_secs(5)));
    fake.next();
    let long = spawn(Call::new().timeout(Duration::from_secs(20)));
    fake.next();
    // One with the same priority and no longer a time-out waits for it.
    let shorter = spawn(Call::new().timeout(Duration::from_secs(1)));
    until_deduped(&client, 1);
    for _ in 0..2 {
        fake.success(vector());
    }
    for caller in [short, long, shorter] {
        caller.join().unwrap().unwrap();
    }
    nothing_sent(&fake);
    let stats = client.cache_stats();
    assert_eq!((stats.misses, stats.deduped), (4, 1));
}

#[test]
fn what_is_kept_is_weighed_with_its_allocations() {
    let fake = Fake::start();
    let client = fake.client().cache(Cache::new()).build().unwrap();
    let values: Vec<f64> = (0..=100).map(f64::from).collect();
    fake.success(matrix(&[(pod("a"), 0, &values)]));
    client.query_range("up", window(0, 100)).unwrap();
    fake.next();
    let bytes = client.cache_stats().bytes;
    assert!(bytes >= 101 * std::mem::size_of::<Point>() + std::mem::size_of::<Series>());
    // Each newest step read leaves it the same size: the points it drops are given back.
    for end in 101..105 {
        fake.success(matrix(&[(pod("a"), end - 1, &[0.0, 0.0])]));
        client.query_range("up", window(end - 100, end)).unwrap();
        fake.next();
        assert_eq!(client.cache_stats().bytes, bytes);
    }
}
