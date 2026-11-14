// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_environment::chart::{Chart, Colour, Series, SeriesStyle};
use tomori_environment::{ContextParameter, Panel};
use tomori_protocol::Pose;
use tomori_protocol::context::{ContextSetting, ContextValue};

use super::*;
use crate::stage::published::Publishing;

const DECLARED: [crate::EnvironmentWall; 2] = [
    crate::EnvironmentWall::new(Pose {
        position: [3.0, 1.5, -4.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }),
    crate::EnvironmentWall::new(Pose {
        position: [-3.0, 1.5, -4.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }),
];

fn console() -> Walls<'static> {
    Walls {
        environment: "console",
        tomori: crate::stage::default_stage(),
        declared: &DECLARED,
        status: crate::stage::Status::Beneath,
    }
}

fn floating() -> Target {
    Target::Free(Pose::at([0.5, 1.4, -1.2]))
}

fn fixed(value: &str) -> Setting {
    Setting::Fixed {
        value: value.to_owned(),
    }
}

/// A console selecting US-EAST1, showing a live latency chart of it on its first wall, published at 1000 s, ten
/// points 6 s apart, a list of pods on its second, depending on the region and the pod, and a rota of nothing: the
/// stage and their identities.
fn console_stage() -> (Stage, String, String, String) {
    let walls = console();
    let mut stage = Stage::default();
    stage.declare_parameters(
        "console",
        crate::stage::context::declared(vec![
            ContextParameter::new("region", "Region", "US-EAST1"),
            ContextParameter::new("pod", "Pod", "pod-7"),
        ])
        .unwrap(),
    );
    let chart = Chart::new(10)
        .with_time(TimeAxis::Live { step_ms: 6_000 })
        .with_series(Series::new(
            "p99",
            Colour::Amber,
            SeriesStyle::Line,
            vec![Some(41.0); 10],
        ));
    for panel in [
        Panel::chart("latency", "Latency", chart).depends_on(["region"]),
        Panel::text("pods", "Pods", "- pod-7")
            .depends_on(["region", "pod"])
            .on_wall(1, None),
        Panel::text("rota", "Rota", "Feed"),
    ] {
        stage
            .publish(&walls, Publishing::new(panel).unwrap(), 1_000)
            .unwrap();
    }
    let shown = stage.view("s", &walls, 0);
    let id = |title: &str| {
        shown
            .panels
            .iter()
            .find(|panel| panel.title == title)
            .unwrap()
            .id
            .clone()
    };
    (stage, id("Latency"), id("Pods"), id("Rota"))
}

fn chips(
    stage: &Stage,
    panel: &str,
) -> Vec<(String, ContextValue, ContextSetting, Option<ChipPress>)> {
    stage
        .view("s", &console(), 0)
        .panel(panel)
        .unwrap()
        .context
        .iter()
        .map(|chip| {
            (
                chip.parameter.clone(),
                chip.value.clone(),
                chip.setting,
                chip.press,
            )
        })
        .collect()
}

#[test]
fn pulled_off_its_wall_a_panel_is_pinned_as_a_copy_and_its_wall_keeps_it_following() {
    let walls = console();
    let (mut stage, latency, pods, rota) = console_stage();
    // The pods' wall fixes the pod: pulled off, the copy is pinned to the region selected and the wall's pod.
    stage
        .set_wall_context(&walls, "environment-1", "pod", Some(fixed("pod-9")))
        .unwrap();
    let (changed, copy) = stage.place_pulled(&walls, &pods, floating(), None).unwrap();
    assert!(changed);
    let (copy, instance) = copy.expect("pinned as a copy");
    assert!(tomori_environment::is_instance(&instance));
    let shown = stage.view("s", &walls, 0);
    assert!(matches!(
        shown.panel(&copy).unwrap().place,
        tomori_protocol::Place::Free { .. }
    ));
    assert!(
        matches!(&shown.panel(&pods).unwrap().place, tomori_protocol::Place::Wall { wall, .. } if wall == "environment-1")
    );
    let text = |text: &str| ContextValue::Text { text: text.into() };
    assert_eq!(
        chips(&stage, &copy),
        [
            (
                "region".into(),
                text("US-EAST1"),
                ContextSetting::Fixed,
                None
            ),
            ("pod".into(), text("pod-9"), ContextSetting::Fixed, None),
        ]
    );
    // The panel on the wall goes on following the selection as it changes; the copy keeps what it showed.
    stage.declare_parameters(
        "console",
        crate::stage::context::declared(vec![
            ContextParameter::new("region", "Region", "EU-WEST1"),
            ContextParameter::new("pod", "Pod", "pod-7"),
        ])
        .unwrap(),
    );
    assert_eq!(chips(&stage, &pods)[0].1, text("EU-WEST1"));
    assert_eq!(chips(&stage, &copy)[0].1, text("US-EAST1"));
    // Its side is told the copy, pinned so.
    let contexts = stage.contexts("console");
    assert_eq!(contexts[&instance].of(), Some("pods"));
    assert_eq!(
        contexts[&instance].binding("region"),
        Some(&tomori_environment::Binding::Fixed("US-EAST1".into()))
    );
    // The copy, an instance already, pulled off another wall is pinned in place, and moved itself.
    assert!(
        stage
            .place_sized(&walls, &copy, Target::Wall("environment-0".into()), None)
            .unwrap()
    );
    stage.set_panel_context(&walls, &copy, "pod", None).unwrap();
    let (_, again) = stage.place_pulled(&walls, &copy, floating(), None).unwrap();
    assert!(again.is_none());
    assert_eq!(
        chips(&stage, &copy)[1],
        ("pod".into(), text("pod-7"), ContextSetting::Fixed, None)
    );
    // Put on another wall, a panel takes that wall's context, unpinned; floating already, or depending on nothing, it
    // moves.
    let (_, moved) = stage
        .place_pulled(&walls, &latency, Target::Wall("environment-1".into()), None)
        .unwrap();
    assert!(moved.is_none());
    let (pulled, _) = stage
        .place_pulled(&walls, &latency, floating(), None)
        .unwrap()
        .1
        .expect("pulled off that wall, pinned");
    assert!(
        stage
            .place_pulled(&walls, &pulled, floating(), None)
            .unwrap()
            .1
            .is_none()
    );
    assert!(
        stage
            .place_pulled(&walls, &rota, floating(), None)
            .unwrap()
            .1
            .is_none()
    );
    // The agent's panel moves as ever, and a panel not here is refused as ever.
    let notes = stage.project(
        &walls,
        crate::stage::Content::parse("title: Notes\n---\nx").unwrap(),
        None,
        0,
    );
    assert!(
        stage
            .place_pulled(&walls, &notes, floating(), None)
            .unwrap()
            .1
            .is_none()
    );
    assert!(
        stage
            .place_pulled(&walls, "panel-999", floating(), None)
            .is_err()
    );
    let mut empty = Stage::default();
    assert!(
        empty
            .place_pulled(&walls, "panel-1", floating(), None)
            .is_err()
    );
}

#[test]
fn a_copy_that_cannot_go_where_it_is_pulled_is_taken_away_again() {
    let walls = console();
    let (mut stage, latency, ..) = console_stage();
    let far = Target::Free(Pose::at([90.0, 1.4, 0.0]));
    assert!(stage.place_pulled(&walls, &latency, far, None).is_err());
    assert_eq!(stage.view("s", &walls, 0).panels.len(), 3);
    assert!(
        stage
            .contexts("console")
            .keys()
            .all(|id| !tomori_environment::is_instance(id))
    );
    // Past the most copies, pulling it off is refused, saying why.
    let to = Destination::Free(Pose::at([0.0, 1.5, -1.5]));
    for region in [
        "EU-WEST1",
        "AP-SOUTH1",
        "EU-WEST1",
        "AP-SOUTH1",
        "EU-WEST1",
        "AP-SOUTH1",
        "EU-WEST1",
        "AP-SOUTH1",
    ] {
        stage
            .copy_panel(
                &walls,
                &latency,
                vec![("region".into(), fixed(region))],
                &to,
            )
            .unwrap();
    }
    assert!(
        stage
            .place_pulled(&walls, &latency, floating(), None)
            .unwrap_err()
            .contains("at most 8")
    );
}

#[test]
fn the_agent_pins_a_panel_as_pulling_it_off_would() {
    let walls = console();
    let (mut stage, latency, pods, rota) = console_stage();
    let to = Destination::Free(Pose::at([0.0, 1.5, -1.5]));
    stage
        .set_wall_context(
            &walls,
            "environment-0",
            "time",
            Some(Setting::Live { minutes: 30 }),
        )
        .unwrap();
    let ((copy, _), pinned) = stage.pin_copy(&walls, &latency, &to).unwrap();
    assert_eq!(
        pinned,
        [
            ("region".to_owned(), fixed("US-EAST1")),
            ("time".to_owned(), Setting::Live { minutes: 30 })
        ]
    );
    assert_eq!(stage.instance_of("console", &copy), Some("latency"));
    assert!(
        stage
            .pin_copy(&walls, &latency, &Destination::Tomori)
            .unwrap_err()
            .contains("Tomori wall")
    );
    // One fixing all it depends on is refused, as is one depending on nothing, the agent's and one not here.
    stage
        .set_panel_context(&walls, &pods, "region", Some(fixed("EU-WEST1")))
        .unwrap();
    stage
        .set_panel_context(&walls, &pods, "pod", Some(fixed("pod-9")))
        .unwrap();
    assert!(
        stage
            .pin_copy(&walls, &pods, &to)
            .unwrap_err()
            .contains("follows nothing")
    );
    assert!(
        stage
            .pin_copy(&walls, &rota, &to)
            .unwrap_err()
            .contains("follows nothing")
    );
    let notes = stage.project(
        &walls,
        crate::stage::Content::parse("title: Notes\n---\nx").unwrap(),
        None,
        0,
    );
    assert!(
        stage
            .pin_copy(&walls, &notes, &to)
            .unwrap_err()
            .contains("Only a panel")
    );
    assert_eq!(
        stage.pin_copy(&walls, "panel-999", &to),
        Err("That panel is not here.")
    );
}

/// The instant `copy`'s chart was last published, in milliseconds since the epoch, which its live axis ends at.
fn published_at(stage: &Stage, copy: &str) -> i64 {
    stage
        .kept("console", copy)
        .unwrap()
        .published
        .as_ref()
        .unwrap()
        .at_ms
}

fn chart_of(stage: &Stage, panel: &str) -> tomori_protocol::chart::Chart {
    *stage
        .view("s", &console(), 0)
        .panel(panel)
        .unwrap()
        .chart
        .clone()
        .unwrap()
}

fn latency_for(instance: &str, time: TimeAxis, points: u32) -> Publishing {
    let chart = Chart::new(points).with_time(time).with_series(Series::new(
        "p99",
        Colour::Amber,
        SeriesStyle::Line,
        vec![Some(7.0); points as usize],
    ));
    Publishing::new(Panel::chart(instance, "Latency", chart).depends_on(["region"])).unwrap()
}

#[test]
fn a_copy_is_frozen_to_the_window_its_chart_shows_and_unfrozen_to_follow_again() {
    let walls = console();
    let (mut stage, latency, _, _) = console_stage();
    let (copy, instance) = stage
        .place_pulled(&walls, &latency, floating(), None)
        .unwrap()
        .1
        .unwrap();
    // Its time chip freezes it, pressed; the panel on the wall offers nothing.
    assert_eq!(chips(&stage, &copy)[1].3, Some(ChipPress::Freeze));
    assert_eq!(chips(&stage, &latency)[1].3, None);
    // Frozen to the window it shows: nine steps of 6 s, ending at the instant its last point stood for, as it was
    // published, its chart kept for exactly that window.
    let before = chart_of(&stage, &copy);
    assert!(published_at(&stage, &copy) > 1_700_000_000_000);
    // Published, as if, at 02:30 UTC on 14 November 2026, so that its window is within that day, whatever the clock.
    let to_ms = 1_794_623_400_000;
    stage
        .here_mut("console", &copy)
        .and_then(|kept| kept.published.as_mut())
        .unwrap()
        .at_ms = to_ms;
    let (from_ms, to_ms) = (to_ms - 54_000, to_ms);
    assert_eq!(stage.freeze(&walls, &copy, true), Ok(true));
    assert_eq!(
        chips(&stage, &copy)[1],
        (
            "time".into(),
            ContextValue::Window { from_ms, to_ms },
            ContextSetting::Fixed,
            Some(ChipPress::Unfreeze)
        )
    );
    let frozen = chart_of(&stage, &copy);
    // Its body, what the agent reads, says its newest value at the end of the window it is frozen to, as its legend
    // does, rather than `now`; as participants are sent it, dated against today.
    assert_eq!(before.summary(), "p99: 41 now");
    let body = &stage
        .here("console")
        .find(|kept| kept.id == copy)
        .unwrap()
        .content
        .body;
    assert_eq!(body, "p99: 41 at its end, 11-14 02:30 UTC");
    let sent = |now: u64| {
        stage
            .view("s", &walls, now)
            .panel(&copy)
            .unwrap()
            .body
            .clone()
    };
    assert_eq!(sent((to_ms / 1000) as u64), "p99: 41 at its end, 02:30 UTC");
    assert_eq!(sent(0), *body);
    assert_eq!(frozen.series, before.series);
    assert_eq!(frozen.time, Some(TimeAxis::Fixed { from_ms, to_ms }));
    assert!(frozen.labels.last().unwrap().text.ends_with(" UTC"));
    assert_eq!(stage.freeze(&walls, &copy, true), Ok(false));
    // Its side is told the window, and live data no longer reaches it.
    assert_eq!(
        stage.contexts("console")[&instance].time(),
        tomori_environment::Time::Window { from_ms, to_ms }
    );
    stage
        .publish(
            &walls,
            latency_for(&instance, TimeAxis::Live { step_ms: 6_000 }, 10),
            2_000,
        )
        .unwrap();
    assert_eq!(chart_of(&stage, &copy).series, before.series);
    // Unfrozen, it follows again, and takes live data.
    assert_eq!(stage.freeze(&walls, &copy, false), Ok(true));
    assert_eq!(stage.freeze(&walls, &copy, false), Ok(false));
    assert_eq!(chips(&stage, &copy)[1].3, Some(ChipPress::Freeze));
    assert!(
        stage
            .panel_settings("console", &copy)
            .unwrap()
            .get(TIME)
            .is_none()
    );
    stage
        .publish(
            &walls,
            latency_for(&instance, TimeAxis::Live { step_ms: 6_000 }, 10),
            3_000,
        )
        .unwrap();
    assert_eq!(
        chart_of(&stage, &copy).time,
        Some(TimeAxis::Live { step_ms: 6_000 })
    );
}

#[test]
fn only_a_copy_of_a_chart_is_frozen_on_every_path() {
    let walls = console();
    let (mut stage, latency, pods, _) = console_stage();
    // The panel on its wall, a list, the agent's own and one not here are refused, frozen or unfrozen, saying why.
    for frozen in [true, false] {
        assert_eq!(stage.freeze(&walls, &latency, frozen), Err(ONLY_COPIES));
        assert_eq!(stage.freeze(&walls, &pods, frozen), Err(ONLY_COPIES));
    }
    let notes = stage.project(
        &walls,
        crate::stage::Content::parse("title: Notes\n---\nx").unwrap(),
        None,
        0,
    );
    assert_eq!(stage.freeze(&walls, &notes, true), Err(ONLY_COPIES));
    assert_eq!(
        stage.freeze(&walls, "panel-999", true),
        Err("That panel is not here.")
    );
    assert!(
        stage
            .panel_settings("console", &latency)
            .unwrap()
            .is_empty()
    );
    // A copy of the list is no chart; a copy of a chart whose panel was withdrawn is not frozen either.
    let (pinned, _) = stage
        .place_pulled(&walls, &pods, floating(), None)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(stage.freeze(&walls, &pinned, true), Err(ONLY_COPIES));
    let (copy, _) = stage
        .place_pulled(&walls, &latency, floating(), None)
        .unwrap()
        .1
        .unwrap();
    assert!(stage.retract("console", "latency"));
    stage.withdraw("console", "latency", 0);
    assert_eq!(stage.freeze(&walls, &copy, true), Err(ONLY_COPIES));
    assert_eq!(chips(&stage, &copy)[1].3, None);
}

#[test]
fn unfreezing_a_copy_gives_back_its_own_live_window() {
    let walls = console();
    let (mut stage, latency, _, _) = console_stage();
    let to = Destination::Free(Pose::at([0.0, 1.5, -1.5]));
    let (copy, _) = stage
        .copy_panel(
            &walls,
            &latency,
            vec![("time".into(), Setting::Live { minutes: 60 })],
            &to,
        )
        .unwrap();
    assert_eq!(stage.freeze(&walls, &copy, true), Ok(true));
    assert!(matches!(
        stage.panel_settings("console", &copy).unwrap().get(TIME),
        Some(Setting::Window { .. })
    ));
    assert_eq!(stage.freeze(&walls, &copy, false), Ok(true));
    assert_eq!(
        stage.panel_settings("console", &copy).unwrap().get(TIME),
        Some(&Setting::Live { minutes: 60 })
    );
}

#[test]
fn a_frozen_copy_and_its_settings_are_kept_across_a_restart_and_its_window_asked_for_again() {
    let walls = console();
    let (mut stage, latency, _, _) = console_stage();
    stage
        .set_wall_context(&walls, "environment-1", "pod", Some(fixed("pod-9")))
        .unwrap();
    let to = Destination::Free(Pose::at([0.0, 1.5, -1.5]));
    let (copy, instance) = stage
        .copy_panel(
            &walls,
            &latency,
            vec![("time".into(), Setting::Live { minutes: 60 })],
            &to,
        )
        .unwrap();
    stage.freeze(&walls, &copy, true).unwrap();
    let window = stage
        .panel_settings("console", &copy)
        .unwrap()
        .get(TIME)
        .cloned()
        .unwrap();
    let Setting::Window { from_ms, to_ms } = window.clone() else {
        panic!("frozen to a window");
    };
    // Written and read back, as the space is across a restart: the panels its side published rest, without what they
    // showed, the copy with its settings, frozen, and the wall's.
    let mut kept: Stage = serde_json::from_value(serde_json::to_value(&stage).unwrap()).unwrap();
    assert!(kept.view("s", &walls, 0).panel(&copy).is_none());
    let walled: Vec<_> = kept.wall_contexts("console").collect();
    assert_eq!(walled[0].1.get("pod"), Some(&fixed("pod-9")));
    // Its side started again is told the copy, frozen, asking it for that window's data again.
    kept.declare_parameters(
        "console",
        crate::stage::context::declared(vec![ContextParameter::new(
            "region", "Region", "US-EAST1",
        )])
        .unwrap(),
    );
    let told = kept.contexts("console");
    assert_eq!(told[&instance].of(), Some("latency"));
    assert_eq!(
        told[&instance].time(),
        tomori_environment::Time::Window { from_ms, to_ms }
    );
    // Publishing it live, it comes back waiting for its window; publishing that window, it shows it, frozen still.
    kept.publish(
        &walls,
        latency_for(&instance, TimeAxis::Live { step_ms: 6_000 }, 10),
        0,
    )
    .unwrap();
    let waiting = chart_of(&kept, &copy);
    assert!(!waiting.has_values());
    assert!(
        waiting
            .status
            .unwrap()
            .text
            .starts_with(crate::stage::held::WAITING)
    );
    kept.publish(
        &walls,
        latency_for(&instance, TimeAxis::Fixed { from_ms, to_ms }, 10),
        0,
    )
    .unwrap();
    assert!(chart_of(&kept, &copy).has_values());
    assert_eq!(chips(&kept, &copy)[1].3, Some(ChipPress::Unfreeze));
    // Unfrozen, it has its own live window back, kept across the restart too.
    assert_eq!(kept.freeze(&walls, &copy, false), Ok(true));
    assert_eq!(
        kept.panel_settings("console", &copy).unwrap().get(TIME),
        Some(&Setting::Live { minutes: 60 })
    );
}

#[test]
fn a_copy_of_a_fixed_chart_keeps_its_window_and_one_too_wide_or_saying_no_time_is_refused() {
    let walls = console();
    let (mut stage, latency, _, _) = console_stage();
    let (copy, instance) = stage
        .place_pulled(&walls, &latency, floating(), None)
        .unwrap()
        .1
        .unwrap();
    let republish = |stage: &mut Stage, time: Option<TimeAxis>, points: u32| {
        stage.freeze(&walls, &copy, false).unwrap();
        let chart = match time {
            Some(time) => Chart::new(points).with_time(time),
            None => Chart::new(points),
        };
        stage
            .publish(
                &walls,
                Publishing::new(
                    Panel::chart(instance.as_str(), "Latency", chart).depends_on(["region"]),
                )
                .unwrap(),
                0,
            )
            .unwrap();
    };
    republish(
        &mut stage,
        Some(TimeAxis::Fixed {
            from_ms: 0,
            to_ms: 60_000,
        }),
        10,
    );
    assert_eq!(stage.freeze(&walls, &copy, true), Ok(true));
    assert_eq!(
        stage.panel_settings("console", &copy).unwrap().get(TIME),
        Some(&Setting::Window {
            from_ms: 0,
            to_ms: 60_000
        })
    );
    // A live chart wider than 31 days is not frozen; nor one whose time it does not say, or says of a kind unknown.
    republish(
        &mut stage,
        Some(TimeAxis::Live {
            step_ms: 86_400_000,
        }),
        40,
    );
    assert!(stage.freeze(&walls, &copy, true).is_err());
    republish(&mut stage, None, 10);
    assert_eq!(stage.freeze(&walls, &copy, true), Err(NO_WINDOW));
    republish(&mut stage, Some(TimeAxis::Live { step_ms: 1_000 }), 10);
    stage
        .here_mut("console", &copy)
        .and_then(|kept| kept.published.as_mut())
        .and_then(|published| published.chart.as_mut())
        .unwrap()
        .time = Some(TimeAxis::Unknown);
    assert_eq!(stage.freeze(&walls, &copy, true), Err(NO_WINDOW));
}
