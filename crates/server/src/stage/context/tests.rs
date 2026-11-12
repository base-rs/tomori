// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_environment::chart::{Chart, TimeAxis};
use tomori_environment::{ContextParameter, Panel};
use tomori_protocol::Pose;

use super::*;
use crate::stage::Destination;
use crate::stage::published::Publishing;

/// The console's two walls.
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

/// 12 November 2026, 02:20 to 02:50 UTC.
const WINDOW: Setting = Setting::Window {
    from_ms: 1_794_450_000_000,
    to_ms: 1_794_451_800_000,
};

fn region(value: &str) -> Setting {
    Setting::Fixed {
        value: value.to_owned(),
    }
}

/// The console's region and pod, the region choosing among three.
fn parameters() -> Vec<Parameter> {
    declared(vec![
        ContextParameter::new("region", "Region", "US-EAST1").with_choices([
            "US-EAST1",
            "EU-WEST1",
            "AP-SOUTH1",
        ]),
        ContextParameter::new("pod", "Pod", "pod-7"),
    ])
    .unwrap()
}

/// A live latency chart of the selected region, 15 minutes at 5 s, on the console's first wall, and a list of pods of
/// the selected region on its second, and a rota that depends on nothing, with the console's parameters declared.
fn stage() -> (Stage, String, String, String) {
    let walls = console();
    let mut stage = Stage::default();
    stage.declare_parameters("console", parameters());
    let chart = Chart::new(181).with_time(TimeAxis::Live { step_ms: 5_000 });
    let published = [
        Panel::chart("latency", "Latency", chart).depends_on(["region"]),
        Panel::text("pods", "Pods", "- pod-7")
            .depends_on(["region", "pod"])
            .on_wall(1, None),
        Panel::text("rota", "Rota", "Feed"),
    ];
    for panel in published {
        stage
            .publish(&walls, Publishing::new(panel).unwrap(), 0)
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

fn chip(parameter: &str, label: &str, value: ContextValue, setting: ContextSetting) -> ContextChip {
    ContextChip::new(parameter, label, value, setting)
}

fn text(value: &str) -> ContextValue {
    ContextValue::Text {
        text: value.to_owned(),
    }
}

#[test]
fn a_side_declares_at_most_three_parameters_cleaned_by_safe_ids_never_time() {
    let parameters = declared(vec![
        ContextParameter::new(
            "region",
            "  A label far too long to keep  ",
            "US-EAST1\n(selected)",
        )
        .with_choices(["US-EAST1", "EU-WEST1"]),
        ContextParameter::new("pod", "\u{202e}", "pod-7"),
    ])
    .unwrap();
    assert_eq!(parameters[0].label, "A label far too …");
    assert_eq!(parameters[0].value, "US-EAST1 (selected)");
    assert_eq!(parameters[0].choices, ["US-EAST1", "EU-WEST1"]);
    // A label empty once cleaned takes its id.
    assert_eq!(parameters[1].label, "pod");
    let many = |count: usize| {
        (0..count)
            .map(|index| ContextParameter::new(format!("p{index}"), "P", "v"))
            .collect::<Vec<_>>()
    };
    assert!(declared(many(3)).is_ok());
    assert!(declared(many(4)).unwrap_err().contains("at most 3"));
    for id in ["time", "a region", "", &"r".repeat(33)] {
        assert!(
            declared(vec![ContextParameter::new(id, "P", "v")]).is_err(),
            "{id}"
        );
    }
    let twice = vec![
        ContextParameter::new("region", "R", "a"),
        ContextParameter::new("region", "R", "b"),
    ];
    assert!(declared(twice).unwrap_err().contains("same id"));
    let choices = ContextParameter::new("region", "R", "a")
        .with_choices((0..17).map(|index| format!("c{index}")));
    assert!(declared(vec![choices]).unwrap_err().contains("16 choices"));
}

#[test]
fn a_panel_depends_on_at_most_three_parameters_and_time_each_once() {
    assert!(dependencies(&["region".into(), "pod".into(), "node".into(), "time".into()]).is_ok());
    let five: Vec<String> = ["a", "b", "c", "d", "time"].map(String::from).to_vec();
    assert!(dependencies(&five).is_err());
    assert!(dependencies(&["a region".into()]).is_err());
    assert!(dependencies(&["region".into(), "region".into()]).is_err());
    assert!(
        Publishing::new(Panel::text("pods", "Pods", "").depends_on(["region", "region"])).is_err()
    );
}

#[test]
fn settings_are_checked_time_absolute_and_bounded_and_values_among_the_choices() {
    let declared = parameters();
    let check = |parameter: &str, setting: Setting| setting.checked(parameter, &declared);
    assert_eq!(check("time", WINDOW), Ok(WINDOW));
    assert_eq!(
        check("time", Setting::Live { minutes: 30 }),
        Ok(Setting::Live { minutes: 30 })
    );
    assert!(
        check(
            "time",
            Setting::Live {
                minutes: LIVE_MINUTES
            }
        )
        .is_ok()
    );
    assert!(check("time", Setting::Live { minutes: 0 }).is_err());
    assert!(
        check(
            "time",
            Setting::Live {
                minutes: LIVE_MINUTES + 1
            }
        )
        .is_err()
    );
    let window = |from_ms: i64, to_ms: i64| Setting::Window { from_ms, to_ms };
    assert!(check("time", window(5, 5)).is_err());
    assert!(check("time", window(9, 5)).is_err());
    assert!(check("time", window(0, WINDOW_MS)).is_ok());
    assert!(check("time", window(0, WINDOW_MS + 1)).is_err());
    assert!(check("time", window(CHART_EARLIEST_MS - 1, 0)).is_err());
    assert!(check("time", window(CHART_EARLIEST_MS, CHART_EARLIEST_MS + 1)).is_ok());
    assert!(check("time", window(CHART_INSTANT_MS - 1, CHART_INSTANT_MS + 1)).is_err());
    assert!(check("time", region("US-EAST1")).is_err());
    // A value of a declared parameter, cleaned, among its choices where it has any.
    assert_eq!(
        check("region", region(" EU-WEST1 ")),
        Ok(region("EU-WEST1"))
    );
    assert!(
        check("region", region("MARS-1"))
            .unwrap_err()
            .contains("choices")
    );
    assert_eq!(check("pod", region("pod-9")), Ok(region("pod-9")));
    assert!(check("pod", region(" \n ")).is_err());
    assert!(check("pod", WINDOW).is_err());
    assert!(
        check("node", region("n-1"))
            .unwrap_err()
            .contains("no such parameter")
    );
    // What a side is told of each.
    assert_eq!(
        region("EU-WEST1").binding(),
        Binding::Fixed("EU-WEST1".into())
    );
    assert_eq!(WINDOW.binding(), Binding::Follows);
    assert_eq!(
        WINDOW.time(),
        Time::Window {
            from_ms: 1_794_450_000_000,
            to_ms: 1_794_451_800_000
        }
    );
    assert_eq!(
        Setting::Live { minutes: 30 }.time(),
        Time::Live { minutes: 30 }
    );
    assert_eq!(region("EU-WEST1").time(), Time::Follows);
}

#[test]
fn a_panel_inherits_its_wall_s_context_where_it_does_not_fix_it_itself() {
    let walls = console();
    let (mut stage, latency, pods, rota) = stage();
    let chips = |stage: &Stage, panel: &str| {
        stage
            .view("s", &walls, 0)
            .panel(panel)
            .unwrap()
            .context
            .clone()
    };
    // Following: the side's values now, and the time its chart shows, 15 minutes rounded up from 180 points at 5 s.
    assert_eq!(
        chips(&stage, &latency),
        [
            chip(
                "region",
                "Region",
                text("US-EAST1"),
                ContextSetting::Follows
            ),
            chip(
                "time",
                "Time",
                ContextValue::Live { minutes: 15 },
                ContextSetting::Follows
            ),
        ]
    );
    // A list depending on two parameters and not on time has their chips alone, in the order the side declares them.
    assert_eq!(
        chips(&stage, &pods),
        [
            chip(
                "region",
                "Region",
                text("US-EAST1"),
                ContextSetting::Follows
            ),
            chip("pod", "Pod", text("pod-7"), ContextSetting::Follows),
        ]
    );
    assert!(chips(&stage, &rota).is_empty());
    assert!(
        stage
            .view("s", &walls, 0)
            .walls
            .iter()
            .all(|wall| wall.context.is_empty())
    );
    // The first wall fixed to EU-WEST1 and a window: its chips, and its panels inherit them.
    assert_eq!(
        stage.set_wall_context(&walls, "environment-0", "region", Some(region("EU-WEST1"))),
        Ok(true)
    );
    assert_eq!(
        stage.set_wall_context(&walls, "environment-0", "region", Some(region("EU-WEST1"))),
        Ok(false)
    );
    assert_eq!(
        stage.set_wall_context(&walls, "environment-0", "time", Some(WINDOW)),
        Ok(true)
    );
    let window = ContextValue::Window {
        from_ms: 1_794_450_000_000,
        to_ms: 1_794_451_800_000,
    };
    let shown = stage.view("s", &walls, 0);
    assert_eq!(
        shown.wall("environment-0").unwrap().context,
        [
            chip("region", "Region", text("EU-WEST1"), ContextSetting::Fixed),
            chip("time", "Time", window.clone(), ContextSetting::Fixed),
        ]
    );
    assert!(shown.wall("environment-1").unwrap().context.is_empty());
    assert_eq!(
        chips(&stage, &latency),
        [
            chip(
                "region",
                "Region",
                text("EU-WEST1"),
                ContextSetting::Inherited
            ),
            chip("time", "Time", window.clone(), ContextSetting::Inherited),
        ]
    );
    // Its own setting over its wall's.
    assert_eq!(
        stage.set_panel_context(&walls, &latency, "region", Some(region("AP-SOUTH1"))),
        Ok(true)
    );
    assert_eq!(
        chips(&stage, &latency)[0],
        chip("region", "Region", text("AP-SOUTH1"), ContextSetting::Fixed)
    );
    // Moved to the other wall, it keeps its own and follows time there.
    assert_eq!(
        stage.relocate(
            &walls,
            &latency,
            &Destination::Wall {
                wall: "environment-1".into(),
                slot: None
            }
        ),
        Ok(true)
    );
    assert_eq!(
        chips(&stage, &latency),
        [
            chip("region", "Region", text("AP-SOUTH1"), ContextSetting::Fixed),
            chip(
                "time",
                "Time",
                ContextValue::Live { minutes: 15 },
                ContextSetting::Follows
            ),
        ]
    );
    // Following again, it follows its wall's, or the side's.
    assert_eq!(
        stage.set_panel_context(&walls, &latency, "region", None),
        Ok(true)
    );
    assert_eq!(
        stage.set_panel_context(&walls, &latency, "region", None),
        Ok(false)
    );
    assert_eq!(chips(&stage, &latency)[0].setting, ContextSetting::Follows);
    // The side's selection changing changes what following panels show, and nothing else.
    assert!(
        stage.declare_parameters(
            "console",
            declared(vec![
                ContextParameter::new("region", "Region", "EU-WEST1"),
                ContextParameter::new("pod", "Pod", "pod-7")
            ])
            .unwrap()
        )
    );
    assert!(
        !stage.declare_parameters(
            "console",
            declared(vec![
                ContextParameter::new("region", "Region", "EU-WEST1"),
                ContextParameter::new("pod", "Pod", "pod-7")
            ])
            .unwrap()
        )
    );
    assert_eq!(
        chips(&stage, &pods)[0],
        chip(
            "region",
            "Region",
            text("EU-WEST1"),
            ContextSetting::Follows
        )
    );
    // A wall left following shows nothing.
    assert_eq!(
        stage.set_wall_context(&walls, "environment-0", "region", None),
        Ok(true)
    );
    assert_eq!(
        stage.set_wall_context(&walls, "environment-0", "time", None),
        Ok(true)
    );
    assert_eq!(
        stage.set_wall_context(&walls, "environment-0", "time", None),
        Ok(false)
    );
    assert!(
        stage
            .view("s", &walls, 0)
            .walls
            .iter()
            .all(|wall| wall.context.is_empty())
    );
}

#[test]
fn settings_are_refused_where_they_mean_nothing_and_bounded() {
    let walls = console();
    let (mut stage, latency, pods, rota) = stage();
    assert!(
        stage
            .set_wall_context(&walls, TOMORI_WALL, "region", Some(region("EU-WEST1")))
            .is_err()
    );
    assert!(
        stage
            .set_wall_context(&walls, "wall-9", "region", Some(region("EU-WEST1")))
            .is_err()
    );
    assert!(
        stage
            .set_wall_context(&walls, "environment-0", "region", Some(region("MARS-1")))
            .is_err()
    );
    assert!(
        stage
            .set_panel_context(&walls, "panel-999", "region", Some(region("EU-WEST1")))
            .is_err()
    );
    // A panel that does not depend on a parameter, or time, takes no setting for it, nor does the agent's.
    assert!(
        stage
            .set_panel_context(&walls, &rota, "region", Some(region("EU-WEST1")))
            .is_err()
    );
    assert!(
        stage
            .set_panel_context(&walls, &pods, "time", Some(WINDOW))
            .is_err()
    );
    assert!(
        stage
            .set_panel_context(&walls, &latency, "pod", Some(region("pod-9")))
            .is_err()
    );
    let notes = stage.project(
        &walls,
        crate::stage::Content::parse("title: Notes\n---\nx").unwrap(),
        None,
        0,
    );
    assert!(
        stage
            .set_panel_context(&walls, &notes, "region", Some(region("EU-WEST1")))
            .is_err()
    );
    let tomori = stage.project_to(
        &walls,
        crate::stage::Content::parse("title: Notice\n---\nx").unwrap(),
        &Destination::Tomori,
        0,
    );
    assert!(
        stage
            .set_panel_context(&walls, &tomori, "region", Some(region("EU-WEST1")))
            .is_err()
    );
    // An added wall takes a setting for each parameter its side declares and time, labelled as declared; those of
    // parameters it no longer declares are kept, shown nowhere.
    let added = stage
        .add_wall(&walls, &Pose::at([0.0, 1.5, -2.0]), None)
        .unwrap();
    stage.declare_parameters(
        "console",
        declared(
            ["a", "b", "c"]
                .map(|id| ContextParameter::new(id, id.to_uppercase(), "v"))
                .to_vec(),
        )
        .unwrap(),
    );
    for id in ["a", "b", "c"] {
        assert_eq!(
            stage.set_wall_context(&walls, &added, id, Some(region("x"))),
            Ok(true)
        );
    }
    assert_eq!(
        stage.set_wall_context(&walls, &added, "time", Some(WINDOW)),
        Ok(true)
    );
    let labels = |stage: &Stage| -> Vec<String> {
        stage
            .wall_chips("console", &added)
            .into_iter()
            .map(|chip| chip.label)
            .collect()
    };
    assert_eq!(labels(&stage), ["A", "B", "C", "Time"]);
    assert!(stage.declare_parameters("console", parameters()));
    assert_eq!(labels(&stage), ["Time"]);
    assert_eq!(stage.rooms["console"].contexts[&added].len(), 4);
    // Removed, its settings go with it.
    stage
        .set_wall_context(&walls, &added, "region", Some(region("EU-WEST1")))
        .unwrap();
    stage.remove_wall(&walls, &added).unwrap();
    assert!(!stage.rooms["console"].contexts.contains_key(&added));
    assert!(stage.wall_chips("elsewhere", &added).is_empty());
    assert!(stage.contexts("elsewhere").is_empty());
}

#[test]
fn each_panel_is_told_how_it_is_bound_its_own_setting_over_its_wall_s() {
    let walls = console();
    let (mut stage, latency, pods, _) = stage();
    let contexts = stage.contexts("console");
    // Those that depend on something, by the side's ids, following everything at first.
    assert_eq!(contexts.keys().collect::<Vec<_>>(), ["latency", "pods"]);
    assert!(contexts.values().all(tomori_environment::Context::follows));
    stage
        .set_wall_context(&walls, "environment-1", "region", Some(region("EU-WEST1")))
        .unwrap();
    stage
        .set_panel_context(&walls, &pods, "pod", Some(region("pod-9")))
        .unwrap();
    stage
        .set_panel_context(&walls, &latency, "time", Some(WINDOW))
        .unwrap();
    let contexts = stage.contexts("console");
    assert_eq!(
        contexts["pods"],
        tomori_environment::Context::new(
            "pods",
            vec![
                ("region".into(), Binding::Fixed("EU-WEST1".into())),
                ("pod".into(), Binding::Fixed("pod-9".into())),
            ],
            Time::Follows,
        )
    );
    assert_eq!(
        contexts["latency"],
        tomori_environment::Context::new(
            "latency",
            vec![("region".into(), Binding::Follows)],
            WINDOW.time(),
        )
    );
}

#[test]
fn settings_are_kept_across_a_restart_and_a_panel_comes_back_so_set() {
    let walls = console();
    let (mut stage, latency, _, _) = stage();
    stage
        .set_wall_context(&walls, "environment-1", "region", Some(region("EU-WEST1")))
        .unwrap();
    stage
        .set_panel_context(&walls, &latency, "time", Some(WINDOW))
        .unwrap();
    let mut kept: Stage = serde_json::from_value(serde_json::to_value(&stage).unwrap()).unwrap();
    // What the side declared belongs to its run, and is never written; the wall's settings and the panel's are, and
    // a parameter's show nowhere until its side declares it again.
    assert!(kept.parameters("console").is_empty());
    assert!(kept.wall_chips("console", "environment-1").is_empty());
    kept.declare_parameters("console", parameters());
    let chart = Chart::new(181).with_time(TimeAxis::Live { step_ms: 5_000 });
    kept.publish(
        &walls,
        Publishing::new(Panel::chart("latency", "Latency", chart).depends_on(["region"])).unwrap(),
        0,
    )
    .unwrap();
    assert_eq!(kept.contexts("console")["latency"].time(), WINDOW.time());
    assert_eq!(
        kept.wall_chips("console", "environment-1")[0].label,
        "Region"
    );
    // A run ending forgets what its side declared.
    kept.rest("console");
    assert!(kept.parameters("console").is_empty());
}

#[test]
fn time_follows_what_a_chart_shows_or_says_nothing_where_its_chart_says_nothing() {
    let walls = console();
    let mut stage = Stage::default();
    let charts = [
        Panel::chart(
            "fixed",
            "Fixed",
            Chart::new(181).with_time(TimeAxis::Fixed {
                from_ms: 0,
                to_ms: 1_800_000,
            }),
        ),
        Panel::chart("untimed", "Untimed", Chart::new(181)),
        Panel::text("events", "Events", "").depends_on(["time"]),
    ];
    for panel in charts {
        stage
            .publish(&walls, Publishing::new(panel).unwrap(), 0)
            .unwrap();
    }
    let shown = stage.view("s", &walls, 0);
    let chips = |title: &str| {
        shown
            .panels
            .iter()
            .find(|panel| panel.title == title)
            .unwrap()
            .context
            .clone()
    };
    assert_eq!(
        chips("Fixed"),
        [chip(
            "time",
            "Time",
            ContextValue::Window {
                from_ms: 0,
                to_ms: 1_800_000
            },
            ContextSetting::Follows
        )]
    );
    assert!(chips("Untimed").is_empty());
    assert!(chips("Events").is_empty());
    // A list of events takes a time setting, and is told it.
    let events = shown
        .panels
        .iter()
        .find(|panel| panel.title == "Events")
        .unwrap()
        .id
        .clone();
    assert_eq!(
        stage.set_panel_context(&walls, &events, "time", Some(Setting::Live { minutes: 60 })),
        Ok(true)
    );
    assert_eq!(
        stage.contexts("console")["events"].time(),
        Time::Live { minutes: 60 }
    );
    let shown = stage.view("s", &walls, 0);
    assert_eq!(
        shown.panel(&events).unwrap().context,
        [chip(
            "time",
            "Time",
            ContextValue::Live { minutes: 60 },
            ContextSetting::Fixed
        )]
    );
    // A time axis of a kind this server does not know says nothing.
    let mut chart = Chart::new(181);
    chart.time = Some(TimeAxis::Unknown);
    assert_eq!(shown_time(&chart), None);
}

#[test]
fn a_setting_of_a_parameter_no_longer_declared_is_kept_shown_nowhere_and_binds_again_once_declared()
{
    let walls = console();
    let (mut stage, latency, pods, _) = stage();
    stage
        .set_wall_context(&walls, "environment-0", "region", Some(region("EU-WEST1")))
        .unwrap();
    stage
        .set_panel_context(&walls, &latency, "region", Some(region("AP-SOUTH1")))
        .unwrap();
    stage
        .set_panel_context(&walls, &pods, "pod", Some(region("pod-9")))
        .unwrap();
    // The side no longer declares a pod, and no longer offers AP-SOUTH1: the pod's setting is kept but shown nowhere and
    // its side is told no binding for it, while a value no longer among the choices is still shown and bound, alike.
    let narrower = declared(vec![
        ContextParameter::new("region", "Region", "US-EAST1")
            .with_choices(["US-EAST1", "EU-WEST1"]),
    ])
    .unwrap();
    assert!(stage.declare_parameters("console", narrower.clone()));
    assert!(!stage.declare_parameters("console", narrower));
    let shown = stage.view("s", &walls, 0);
    assert_eq!(
        shown.panel(&latency).unwrap().context[0],
        chip("region", "Region", text("AP-SOUTH1"), ContextSetting::Fixed)
    );
    assert_eq!(
        shown.panel(&pods).unwrap().context,
        [chip(
            "region",
            "Region",
            text("US-EAST1"),
            ContextSetting::Follows
        )]
    );
    let contexts = stage.contexts("console");
    assert_eq!(
        contexts["latency"].binding("region"),
        Some(&Binding::Fixed("AP-SOUTH1".into()))
    );
    assert_eq!(contexts["pods"].binding("pod"), None);
    assert_eq!(contexts["pods"].bindings().len(), 1);
    assert!(
        stage.rooms["console"]
            .panels
            .iter()
            .any(|kept| kept.context.contains_key("pod"))
    );
    // Declaring nothing, nothing but time is shown on the wall, and nothing is lost.
    stage.declare_parameters("console", Vec::new());
    assert!(stage.wall_chips("console", "environment-0").is_empty());
    assert_eq!(stage.rooms["console"].contexts["environment-0"].len(), 1);
    // Declared again, each binds and shows as before.
    stage.declare_parameters("console", parameters());
    assert_eq!(
        stage.contexts("console")["pods"].binding("pod"),
        Some(&Binding::Fixed("pod-9".into()))
    );
    assert_eq!(stage.wall_chips("console", "environment-0").len(), 1);
    // A run that has not declared yet, as after a restart, binds and shows only time until it does.
    stage.rest("console");
    let mut kept: Stage = serde_json::from_value(serde_json::to_value(&stage).unwrap()).unwrap();
    kept.publish(
        &walls,
        Publishing::new(
            Panel::text("pods", "Pods", "- pod-7")
                .depends_on(["region", "pod"])
                .on_wall(1, None),
        )
        .unwrap(),
        0,
    )
    .unwrap();
    assert!(kept.contexts("console")["pods"].bindings().is_empty());
    let pods = kept.view("s", &walls, 0).panels[0].id.clone();
    assert!(
        kept.view("s", &walls, 0)
            .panel(&pods)
            .unwrap()
            .context
            .is_empty()
    );
    kept.declare_parameters("console", parameters());
    assert_eq!(
        kept.contexts("console")["pods"].binding("pod"),
        Some(&Binding::Fixed("pod-9".into()))
    );
}

#[test]
fn settings_read_back_from_the_state_file_are_held_to_what_the_server_would_set() {
    let (stage, _, _, _) = stage();
    let mut json = serde_json::to_value(&stage).unwrap();
    let room = &mut json["rooms"]["console"];
    room["contexts"] = serde_json::json!({
        "environment-0": {
            "time": {"kind": "live", "minutes": 0},
            "region": {"kind": "window", "from_ms": 0, "to_ms": 1},
            "a region": {"kind": "fixed", "value": "EU-WEST1"},
            "season": {"kind": "solstice"},
            "pod": {"kind": "fixed", "value": "  pod-9\n "},
        },
        "environment-1": {"time": {"kind": "window", "from_ms": 5, "to_ms": 5}},
    });
    let kept: Stage = serde_json::from_value(json).unwrap();
    let contexts = &kept.rooms["console"].contexts;
    assert_eq!(contexts.len(), 1);
    assert_eq!(
        contexts["environment-0"].iter().collect::<Vec<_>>(),
        [(&"pod".to_owned(), &region("pod-9"))]
    );
    // At most a setting for each parameter and time, of a panel as of a wall.
    let settings = serde_json::json!({
        "a": {"kind": "fixed", "value": "1"},
        "b": {"kind": "fixed", "value": "2"},
        "c": {"kind": "fixed", "value": "3"},
        "d": {"kind": "fixed", "value": "4"},
        "time": {"kind": "live", "minutes": 30},
    });
    let parsed = read(settings).unwrap();
    assert_eq!(parsed.len(), SETTINGS);
    assert!(!parsed.contains_key("time"));
    let walled =
        read_walls(serde_json::json!({"wall-1": {"e": {"kind": "fixed", "value": ""}}})).unwrap();
    assert!(walled.is_empty());
    // What is not settings at all is refused, as any malformed state file is.
    assert!(read(serde_json::json!(5)).is_err());
    assert!(read_walls(serde_json::json!("wall")).is_err());
}

#[test]
fn a_chip_pointed_at_is_described_with_what_it_shows_and_where_that_comes_from() {
    let walls = console();
    let (mut stage, latency, pods, _) = stage();
    let pointed = |stage: &Stage, panel: &str, chip: &str| {
        stage
            .describe_at(
                &walls,
                panel,
                crate::stage::Pointed {
                    chip: Some(chip),
                    ..crate::stage::Pointed::default()
                },
                0,
            )
            .unwrap()
    };
    assert!(
        pointed(&stage, &pods, "region")
            .contains("; pointing at its chip: \"Region: US-EAST1\", following the environment"),
        "{}",
        pointed(&stage, &pods, "region")
    );
    assert!(pointed(&stage, &latency, "time").contains("\"Time: the last 15 minutes\", following"));
    stage
        .set_wall_context(&walls, "environment-0", "region", Some(region("EU-WEST1")))
        .unwrap();
    stage
        .set_panel_context(&walls, &latency, "time", Some(WINDOW))
        .unwrap();
    assert!(
        pointed(&stage, &latency, "region")
            .contains("\"Region: EU-WEST1\", inherited from its wall")
    );
    assert!(pointed(&stage, &latency, "time").contains(
        "\"Time: 2026-11-12T02:20:00Z to 2026-11-12T02:50:00Z, UTC\", fixed by its own setting"
    ));
    // A chip it does not show says nothing.
    assert!(!pointed(&stage, &pods, "time").contains("chip"));
}
