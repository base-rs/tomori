// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's agent and its environment's context: `contexts` lists the parameters the environment declares, every
//! setting of its walls and panels, those not shown marked, and what each panel shows; `set_context` fixes a parameter,
//! or time as absolute instants in UTC, on a wall or a panel, or lets it follow, everyone seeing the chips and the side
//! told. See `design/panel-context.md` and `design/mcp.md`.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tomori_environment::chart::{Chart, TimeAxis};
use tomori_environment::{
    Binding, ContextParameter, Intent, Panel, Result, ServerSide, Space, Time,
};
use tomori_protocol::{ClientMessage, Pose, ServerMessage};
use tomori_server::test_support::{TestClient, TestServer, eventually};
use tomori_server::{Environment, EnvironmentWall};

pub mod mcp_support;
use mcp_support::{call, failure};

/// A console's side: it declares the parameters it is given, a latency chart of the selected region on its first wall
/// and a list of the selected region's pod on its second, and records each context it is told.
struct Console {
    declare: Arc<Mutex<Option<Vec<ContextParameter>>>>,
    told: Arc<Mutex<Vec<(u32, tomori_environment::Context)>>>,
}

impl ServerSide for Console {
    fn run(&self, space: &Space) -> Result<()> {
        let chart = Chart::new(181).with_time(TimeAxis::Live { step_ms: 5_000 });
        space.publish(Panel::chart("latency", "Latency", chart).depends_on(["region"]))?;
        space.publish(
            Panel::text("pods", "Pods", "- pod-7")
                .depends_on(["region", "pod"])
                .on_wall(1, None),
        )?;
        // A rota that depends on nothing, which contexts leaves out.
        space.publish(Panel::text("rota", "Rota", "Feed the hens").on_wall(1, None))?;
        while !space.cancellation().is_cancelled() {
            if let Some(parameters) = self.declare.lock().unwrap().take() {
                space.set_context(parameters)?;
            }
            if let Some(intent) = space.next_intent(Duration::from_millis(5))
                && let Some(context) = Intent::context(&intent)
            {
                self.told
                    .lock()
                    .unwrap()
                    .push((intent.participant(), context.clone()));
                // An instance it is told of, it shows in its context, under the id the server gave it.
                if let (Some(of), Some(Binding::Fixed(region))) =
                    (context.of(), context.binding("region"))
                {
                    let chart = Chart::new(181).with_time(TimeAxis::Live { step_ms: 5_000 });
                    space.publish(
                        Panel::chart(context.panel(), format!("{of} · {region}"), chart)
                            .depends_on(["region"]),
                    )?;
                }
            }
        }
        Ok(())
    }
}

fn parameters(pod: bool) -> Vec<ContextParameter> {
    let mut declared = vec![
        ContextParameter::new("region", "Region", "US-EAST1").with_choices([
            "US-EAST1",
            "EU-WEST1",
            "AP-SOUTH1",
        ]),
    ];
    if pod {
        declared.push(ContextParameter::new("pod", "Pod", "pod-7"));
    }
    declared
}

#[test]
fn the_agent_reads_every_setting_and_sets_a_wall_s_or_a_panel_s_context_in_utc() {
    let declare = Arc::new(Mutex::new(Some(parameters(true))));
    let told = Arc::new(Mutex::new(Vec::new()));
    let side = Console {
        declare: declare.clone(),
        told: told.clone(),
    };
    let server = TestServer::with_tokens_and_tools(false, |server| {
        // Named as Sky, which the test client creates its spaces in.
        server.with_environments([Environment::new("sky", "Console")
            .with_walls([
                EnvironmentWall::new(Pose::at([2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])),
                EnvironmentWall::new(Pose::at([-2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])),
            ])
            .with_server_side(side)])
    });
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {owner}"))],
    );
    ada.hello_projected("ada", "Ada");
    let space = ada.create_space("Studio").id;
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    let tool = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);
    let contexts = || tool("contexts", json!({}))["structuredContent"].clone();
    // The parameters as declared, no wall set, and each panel following.
    let read = eventually(|| {
        let read = contexts();
        (read["panels"].as_array()?.len() == 2 && read["parameters"].as_array()?.len() == 2)
            .then_some(read)
    });
    assert_eq!(read["space"], json!(space));
    assert_eq!(
        read["parameters"][0],
        json!({"id": "region", "label": "Region", "value": "US-EAST1", "choices": ["US-EAST1", "EU-WEST1", "AP-SOUTH1"]})
    );
    assert_eq!(read["walls"], json!([]));
    let latency = read["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|panel| panel["title"] == "Latency")
        .unwrap()
        .clone();
    assert_eq!(latency["settings"], json!([]));
    assert_eq!(
        latency["shows"],
        json!([
            {"parameter": "region", "label": "Region", "value": "US-EAST1", "setting": "follows"},
            {"parameter": "time", "label": "Time", "live_minutes": 15, "setting": "follows"},
        ])
    );
    // What it takes, as its schema declares, and nothing else.
    for (arguments, refused) in [
        (
            json!({"parameter": "region", "value": "EU-WEST1"}),
            "Say either wall or panel",
        ),
        (
            json!({"wall": "environment wall 1", "panel": "Latency", "parameter": "region", "value": "EU-WEST1"}),
            "Say either wall or panel",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "region"}),
            "Say one of value",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "live_minutes": 30, "follow": true}),
            "Say one of value",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "from": "2026-11-12T02:20:00"}),
            "Say one of value",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "from": "02:20", "to": "02:50"}),
            "from is an instant with its offset from UTC",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "live_minutes": -3}),
            "live_minutes",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "region", "value": "MARS-1"}),
            "That is not one of the parameter's choices.",
        ),
        (
            json!({"wall": "status wall", "parameter": "region", "value": "EU-WEST1"}),
            "The Tomori wall holds no environment's panels.",
        ),
        (
            json!({"panel": "Pods", "parameter": "time", "live_minutes": 30}),
            "That panel does not depend on that parameter.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "region", "value": "EU-WEST1", "colour": "red"}),
            "colour",
        ),
        (
            json!({"wall": 5, "parameter": "region", "value": "EU-WEST1"}),
            "wall is text.",
        ),
        (
            json!({"panel": 5, "parameter": "region", "value": "EU-WEST1"}),
            "panel is text.",
        ),
        (
            json!({"wall": "environment wall 1", "value": "EU-WEST1"}),
            "parameter",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "r".repeat(33), "value": "EU-WEST1"}),
            "parameter",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "region", "value": 5}),
            "value is text.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "from": 5, "to": "2026-11-12T02:50:00Z"}),
            "from is text.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "from": "2026-11-12T02:20:00Z", "to": 5}),
            "to is text.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "from": "2026-11-12T02:20:00Z", "to": "02:50"}),
            "to is an instant with its offset from UTC",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "region", "follow": "yes"}),
            "follow is true or false.",
        ),
        (
            json!({"wall": "the moon", "parameter": "region", "value": "EU-WEST1"}),
            "There is no wall called \"the moon\" here: name one as presence does",
        ),
        (
            json!({"panel": "Nothing like it", "parameter": "region", "value": "EU-WEST1"}),
            "There is no such panel here.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "live_minutes": 0}),
            "A live window is from a minute to a week.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "live_minutes": 10_081}),
            "A live window is from a minute to a week.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "from": "10000-01-01T00:00:00Z", "to": "10000-01-01T00:30:00Z"}),
            "from is past the year 9999: a time window is within the years 0 to 9999.",
        ),
        (
            json!({"wall": "environment wall 1", "parameter": "time", "from": "-000001-12-31T23:00:00Z", "to": "0000-01-01T00:30:00Z"}),
            "A time window ends after it begins, within the years 0 to 9999, and spans at most 31 days.",
        ),
    ] {
        let failed = failure(&tool("set_context", arguments.clone()));
        assert!(failed.contains(refused), "{arguments}: {failed}");
    }
    // The first wall fixed to EU-WEST1: changed once, its chart inherits it, and the side is told, by nobody.
    let set = tool(
        "set_context",
        json!({"wall": "environment wall 1", "parameter": "region", "value": "EU-WEST1"}),
    )["structuredContent"]
        .clone();
    assert_eq!(set["changed"], true, "{set}");
    assert_eq!(set["id"], "environment-0");
    assert_eq!(
        set["setting"],
        json!({"parameter": "region", "value": "EU-WEST1"})
    );
    assert_eq!(
        set["next"],
        "Everyone sees it on the chips, and a notice on the status wall says what you set; tell everyone in the space in a sentence what you set."
    );
    // A notice on the status wall says what the agent set.
    let listed = tool("panels", json!({}))["structuredContent"]["panels"].clone();
    let notice = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|panel| panel["title"] == "Context")
        .cloned()
        .unwrap_or_else(|| panic!("{listed}"));
    assert!(
        notice
            .to_string()
            .contains("The agent set region to EU-WEST1 of environment wall 1."),
        "{notice}"
    );
    let again = tool(
        "set_context",
        json!({"wall": "environment wall 1", "parameter": "region", "value": "EU-WEST1"}),
    );
    assert_eq!(again["structuredContent"]["changed"], false);
    let fixed = tomori_environment::Context::new(
        "latency",
        vec![("region".into(), Binding::Fixed("EU-WEST1".into()))],
        Time::Follows,
    );
    eventually(|| {
        told.lock()
            .unwrap()
            .contains(&(0, fixed.clone()))
            .then_some(())
    });
    // Everyone sees the wall's chip.
    ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .wall("environment-0")
            .is_some_and(|wall| wall.context.len() == 1)
            .then_some(()),
        ServerMessage::ProjectionPatch { .. } => None,
        _ => None,
    });
    // The chart's own window, in UTC from an instant given in another zone, over its wall's.
    let set = tool(
        "set_context",
        json!({"panel": "Latency", "parameter": "time", "from": "2026-11-12T13:20:00+11:00", "to": "2026-11-12T02:50:00Z"}),
    )["structuredContent"]
        .clone();
    assert_eq!(
        set["setting"],
        json!({"parameter": "time", "from": "2026-11-12T02:20:00Z", "to": "2026-11-12T02:50:00Z"})
    );
    let pods = tool(
        "set_context",
        json!({"panel": "Pods", "parameter": "pod", "value": "pod-9"}),
    );
    assert_eq!(pods["structuredContent"]["changed"], true, "{pods}");
    assert_eq!(pods["structuredContent"]["title"], "Pods");
    let read = contexts();
    assert_eq!(
        read["walls"],
        json!([{"wall": "environment wall 1", "id": "environment-0", "settings": [
            {"parameter": "region", "value": "EU-WEST1", "shown": true},
        ]}])
    );
    let panel = |read: &Value, title: &str| {
        read["panels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|panel| panel["title"] == title)
            .unwrap()
            .clone()
    };
    assert_eq!(
        panel(&read, "Latency")["shows"],
        json!([
            {"parameter": "region", "label": "Region", "value": "EU-WEST1", "setting": "inherited"},
            {"parameter": "time", "label": "Time", "from": "2026-11-12T02:20:00Z", "to": "2026-11-12T02:50:00Z", "setting": "fixed"},
        ])
    );
    // The side no longer declaring a pod, its setting is kept, listed as not shown.
    *declare.lock().unwrap() = Some(parameters(false));
    let read = eventually(|| {
        let read = contexts();
        (read["parameters"].as_array()?.len() == 1).then_some(read)
    });
    assert_eq!(
        panel(&read, "Pods")["settings"],
        json!([{"parameter": "pod", "value": "pod-9", "shown": false, "note": "not shown: the environment doesn't declare it now"}])
    );
    // Following again, the wall holds nothing and is listed no more.
    let follow = tool(
        "set_context",
        json!({"wall": "environment wall 1", "parameter": "region", "follow": true}),
    )["structuredContent"]
        .clone();
    assert_eq!(
        follow["setting"],
        json!({"parameter": "region", "follows": true})
    );
    assert_eq!(follow["changed"], true);
    assert_eq!(contexts()["walls"], json!([]));
    // A session in no space reads and sets nothing.
    for (name, arguments) in [
        ("contexts", json!({})),
        (
            "set_context",
            json!({"wall": "environment wall 1", "parameter": "region", "follow": true}),
        ),
    ] {
        let result = call(&server, &token, "nobody", name, arguments);
        assert_eq!(result["isError"], true, "{result}");
    }
    // Live minutes, for time, on a wall.
    let live = tool(
        "set_context",
        json!({"wall": "environment wall 2", "parameter": "time", "live_minutes": 60}),
    )["structuredContent"]
        .clone();
    assert_eq!(
        live["setting"],
        json!({"parameter": "time", "live_minutes": 60})
    );
}

#[test]
fn the_agent_copies_a_panel_with_a_context_and_builds_walls_each_with_its_own() {
    let declare = Arc::new(Mutex::new(Some(parameters(true))));
    let told = Arc::new(Mutex::new(Vec::new()));
    let side = Console {
        declare: declare.clone(),
        told: told.clone(),
    };
    let server = TestServer::with_tokens_and_tools(false, |server| {
        server.with_environments([Environment::new("sky", "Console")
            .with_walls([
                EnvironmentWall::new(Pose::at([2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])),
                EnvironmentWall::new(Pose::at([-2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])),
            ])
            .with_server_side(side)])
    });
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {owner}"))],
    );
    ada.hello_projected("ada", "Ada");
    ada.create_space("Studio");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    let tool = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);
    let contexts = || tool("contexts", json!({}))["structuredContent"].clone();
    eventually(|| (contexts()["parameters"].as_array()?.len() == 2).then_some(()));
    // What it takes, as its schema declares.
    for (arguments, refused) in [
        (
            json!({"panel": "Latency", "context": "EU-WEST1"}),
            "context is a list of at most 4 settings",
        ),
        (
            json!({"panel": "Latency", "context": [{"parameter": "region", "value": "EU-WEST1"}, {"parameter": "region", "value": "AP-SOUTH1"}]}),
            "context sets each parameter once.",
        ),
        (
            json!({"panel": "Latency", "context": [{"parameter": "region"}]}),
            "Each setting of a context says one of value, live_minutes, or from and to together.",
        ),
        (
            json!({"panel": "Latency", "context": [{"parameter": "region", "follow": true}]}),
            "follow",
        ),
        (
            json!({"panel": "Rota", "context": []}),
            "That panel depends on no context",
        ),
        (
            json!({"panel": "Latency", "context": [{"parameter": "region", "value": "EU-WEST1"}], "place": "wall status wall"}),
            "Tomori wall",
        ),
        (json!({"panel": 5}), "This tool needs panel, as text."),
        (json!({"panel": "p".repeat(2000)}), "too long"),
        (json!({"panel": "Latency", "place": 5}), "place is text."),
        (
            json!({"panel": "Nothing like it"}),
            "There is no such panel here.",
        ),
        (
            json!({"panel": "Latency", "place": "upside down"}),
            "Tomori does not know where \"upside down\" is",
        ),
        (
            json!({"panel": "Latency", "place": "beside Nobody"}),
            "Nobody",
        ),
    ] {
        let failed = failure(&tool("copy_panel", arguments.clone()));
        eprintln!("COPY {arguments}: {failed}");
        assert!(failed.contains(refused), "{arguments}: {failed}");
    }
    // Copied for AP-SOUTH1 onto the other wall: an instance its side is told of, publishes and keeps updated.
    let copied = tool(
        "copy_panel",
        json!({"panel": "Latency", "context": [{"parameter": "region", "value": "AP-SOUTH1"}], "place": "wall environment wall 2"}),
    )["structuredContent"]
        .clone();
    let instance = copied["instance"].as_str().unwrap().to_owned();
    assert!(instance.starts_with('~'), "{copied}");
    assert_eq!(copied["title"], "Latency");
    let copy = copied["panel"].as_str().unwrap().to_owned();
    eventually(|| {
        told.lock()
            .unwrap()
            .iter()
            .any(|(by, context)| {
                *by == 0 && context.panel() == instance && context.of() == Some("latency")
            })
            .then_some(())
    });
    let titled = eventually(|| {
        let read = tool("panels", json!({"panel": copy}))["structuredContent"].clone();
        (read["panels"][0]["title"] == "latency · AP-SOUTH1").then_some(read)
    });
    assert_eq!(titled["panels"][0]["title"], "latency · AP-SOUTH1");
    let listed = contexts();
    let instanced = listed["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|panel| panel["panel"] == json!(copy))
        .cloned()
        .unwrap();
    assert_eq!(instanced["instance_of"], "latency");
    assert_eq!(
        instanced["settings"],
        json!([{"parameter": "region", "value": "AP-SOUTH1", "shown": true}])
    );
    // Without a context, or with one the panel shows already, a copy would show the same, and is refused; a session in
    // no space copies nothing.
    let plain = failure(&tool(
        "copy_panel",
        json!({"panel": "Latency", "place": "wall environment wall 1"}),
    ));
    assert_eq!(
        plain,
        "Say the context the copy shows, at least one setting."
    );
    let again = failure(&tool(
        "copy_panel",
        json!({"panel": copy, "context": [{"parameter": "region", "value": "AP-SOUTH1"}]}),
    ));
    assert!(again.contains("shows that context already"), "{again}");
    let nobody = call(
        &server,
        &token,
        "nobody",
        "copy_panel",
        json!({"panel": "Latency"}),
    );
    assert_eq!(nobody["isError"], true);
    // A notice says what was copied.
    let notices = tool("panels", json!({}))["structuredContent"]["panels"].to_string();
    assert!(notices.contains("The agent copied the panel"), "{notices}");
    // Ada stands at home, looking along −Z, for the layout to stand before her.
    ada.send(&ClientMessage::Presence {
        head: Pose::upright_facing([0.0, 1.6, 0.0], [0.0, 1.6, -1.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
        carried: Vec::new(),
        rays: None,
    });
    eventually(|| {
        tool("presence", json!({}))["structuredContent"]["people"][0]
            .get("head")
            .cloned()
    });
    // Two walls, each with its own window, built in one step; one refused builds nothing.
    let malformed = failure(&tool(
        "create_layout",
        json!({"place": "front Ada", "walls": [{"id": "first", "columns": 2, "rows": 1, "context": "x"}]}),
    ));
    assert!(malformed.starts_with("context is a list"), "{malformed}");
    let refused = failure(&tool(
        "create_layout",
        json!({"place": "front Ada", "walls": [{"id": "first", "columns": 2, "rows": 1, "context": [{"parameter": "region", "value": "MARS-1"}]}]}),
    ));
    assert_eq!(refused, "That is not one of the parameter's choices.");
    let built = tool(
        "create_layout",
        json!({"place": "front Ada", "walls": [
            {"id": "first", "columns": 2, "rows": 1, "context": [{"parameter": "time", "from": "2026-11-12T02:20:00Z", "to": "2026-11-12T02:50:00Z"}]},
            {"id": "second", "columns": 2, "rows": 1, "attach": {"to": "first", "edge": "right"}, "context": [{"parameter": "time", "from": "2026-11-12T04:10:00Z", "to": "2026-11-12T04:40:00Z"}]},
        ]}),
    );
    assert_ne!(built["isError"], true, "{built}");
    let walls = contexts()["walls"].clone();
    let windows: Vec<Value> = walls
        .as_array()
        .unwrap()
        .iter()
        .map(|wall| wall["settings"][0]["from"].clone())
        .collect();
    assert_eq!(
        windows,
        [json!("2026-11-12T02:20:00Z"), json!("2026-11-12T04:10:00Z")]
    );
}
