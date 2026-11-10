// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels an environment's server side publishes onto its own walls, through the whole server: a small example
//! environment, Gauges, whose side keeps a live scaling-pressure chart of 15 minutes at 5 seconds, one new point a tick,
//! and the agent reading it back and inspecting it. Its data is invented. See `design/environments.md` and
//! `design/projection.md`.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tomori_environment::chart::{
    AxisLabel, Chart, Colour, Edge, Note, Reference, Series, SeriesStyle, Shade, Tone, ValueAxis,
};
use tomori_environment::{Panel, Result, Row, ServerSide, Space};
use tomori_protocol::{ClientMessage, Place, Projection, ServerMessage};
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber, eventually};
use tomori_server::{Environment, Server};

/// The points of the window: 15 minutes at 5 seconds.
const POINTS: usize = 181;

/// How often the example publishes its next point: once a second, as fast as the server shows an update of a panel, an
/// update that comes a little early waiting for the next sweep, the latest winning.
const TICK: Duration = Duration::from_secs(1);

/// The invented pressure at sample `index`: a mean that wanders below the target, its greatest rising above it now and
/// then, and every 37th sample missing, as a poll that failed.
fn sample(index: usize) -> (Option<f32>, Option<f32>) {
    if index % 37 == 36 {
        return (None, None);
    }
    let phase = index as f32 / 9.0;
    let mean = 0.7 + 0.15 * phase.sin();
    (Some(mean), Some(mean + 0.35 + 0.3 * (phase / 2.0).cos()))
}

/// The chart of the window ending at sample `newest`.
fn pressure(newest: usize) -> Chart {
    let first = (newest + 1).saturating_sub(POINTS);
    let (mean, greatest): (Vec<_>, Vec<_>) = (first..=newest).map(sample).unzip();
    Chart::new(POINTS as u32)
        .with_labels(vec![
            AxisLabel::new(0, "-15 min"),
            AxisLabel::new(60, "-10 min"),
            AxisLabel::new(120, "-5 min"),
            AxisLabel::new(180, "now"),
        ])
        .with_axis(ValueAxis::new(Some(0.0), None, ""))
        .with_series(Series::new("mean", Colour::Teal, SeriesStyle::Line, mean))
        .with_series(Series::new(
            "greatest",
            Colour::Amber,
            SeriesStyle::Line,
            greatest,
        ))
        .with_reference(Reference::new(1.0, "target", Colour::White))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 1 },
            true,
            Colour::Red,
        ))
        .with_status(Note::new("8 of 8 reporting", Tone::Neutral))
}

/// The example environment's side: it publishes its chart on its wall, and, each tick, the next point; and answers the agent's inspection of it with what the chart shows.
struct Gauges {
    /// The newest sample published, which the inspector reads.
    newest: Arc<Mutex<usize>>,
}

impl ServerSide for Gauges {
    fn run(&self, space: &Space) -> Result<()> {
        let newest = self.newest.clone();
        space.inspect_with(move |reference: &str| -> Result<Option<String>> {
            let newest = *newest.lock().unwrap();
            Ok((reference == "chart/pressure").then(|| {
                json!({"metric": "scaling pressure", "window_seconds": 900, "step_seconds": 5,
                    "newest_sample": newest})
                .to_string()
            }))
        });
        let mut next = POINTS - 1;
        while !space.cancellation().is_cancelled() {
            let panel = Panel::chart("pressure", "Scaling pressure", pressure(next))
                .on_wall(0, Some(1))
                .with_reference("chart/pressure");
            space.publish(panel)?;
            *self.newest.lock().unwrap() = next;
            next += 1;
            std::thread::sleep(TICK);
        }
        Ok(())
    }
}

/// An example environment's side that publishes a queue of pods as a list panel, highlighting one, and answers the
/// agent's inspection of a pod. See `design/panel-actions.md`.
struct Queue;

impl ServerSide for Queue {
    fn run(&self, space: &Space) -> Result<()> {
        space.inspect_with(|reference: &str| -> Result<Option<String>> {
            Ok(reference
                .strip_prefix("pod/")
                .map(|pod| json!({"pod": pod, "waiting": 41}).to_string()))
        });
        let rows = vec![
            Row::new("pod7", "pod7 · 3 waiting").with_reference("pod/7"),
            Row::new("pod112", "pod112 · 41 waiting").with_reference("pod/112"),
            Row::new("pod9", "pod9 · 1 waiting"),
        ];
        space.publish(Panel::list("queue", "Queue", rows).highlighting("pod112"))?;
        while !space.cancellation().is_cancelled() {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
}

/// A server serving Tomori's tools, offering Sky, Gauges and Queue, each of the last two with one wall of its own.
fn start(backend: &FakeBackend) -> TestServer {
    TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_environments(vec![
            Environment::new("sky", "Sky"),
            Environment::new("gauges", "Gauges")
                .with_walls([
                    tomori_protocol::Pose::at([2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])
                ])
                .with_server_side(Gauges {
                    newest: Arc::default(),
                }),
            Environment::new("queue", "Queue")
                .with_walls([
                    tomori_protocol::Pose::at([2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])
                ])
                .with_server_side(Queue),
        ])
    })
}

/// The bearer token the server registered itself with.
fn token(backend: &FakeBackend) -> String {
    let registered = eventually(|| backend.tools().into_iter().find_map(|(_, server)| server));
    let (_, value) = &registered.headers[0];
    value.strip_prefix("Bearer ").unwrap().to_owned()
}

/// Call the tool `name` with `arguments` for the agent of `session`, and return its structured result.
fn call(server: &TestServer, token: &str, session: &str, name: &str, arguments: Value) -> Value {
    let params = json!({ "name": name, "arguments": arguments,
        "_meta": { "ai.opencode/sessionID": session } });
    let body =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }).to_string();
    let mut stream = TcpStream::connect(server.tools.unwrap()).unwrap();
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let result = serde_json::from_str::<Value>(body).unwrap()["result"].clone();
    assert_ne!(result["isError"], true, "{result}");
    result["structuredContent"].clone()
}

/// The next projection `client` is sent whose chart panel shows other than `shown`: the first chart, or an update of
/// it, never the same chart sent again with some other change to the space. Panics after the helpers' timeout.
fn charted(client: &TestClient, shown: Option<&tomori_protocol::Panel>) -> Projection {
    client.wait_for(|message| match message {
        ServerMessage::Projection { projection }
            if projection.panels.iter().any(|panel| {
                panel.chart.is_some() && shown.is_none_or(|shown| panel.chart != shown.chart)
            }) =>
        {
            Some((**projection).clone())
        }
        _ => None,
    })
}

#[test]
fn an_environment_s_live_chart_is_on_its_wall_for_everyone_and_the_agent_reads_it() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Gauges".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some("gauges".to_owned()),
        instructions: None,
    });
    let space = ada.space(|state| state.title == "Gauges").id;
    // On the environment's wall, in the slot it prefers, a chart of every point, its body a summary for older clients.
    let first = charted(&ada, None);
    assert_eq!(first.space, space);
    let panel = first.panels[0].clone();
    assert_eq!(
        panel.place,
        Place::Wall {
            wall: "environment-0".into(),
            slot: 1
        }
    );
    let chart = panel.chart.clone().unwrap();
    assert_eq!(chart.series[0].values.len(), POINTS);
    assert!(chart.series[0].values.contains(&None), "a gap stays a gap");
    assert!(
        panel.body.starts_with("8 of 8 reporting\nmean: "),
        "{}",
        panel.body
    );
    // Each update is the same panel, in place, a point newer, no sooner than a second after the last: timed from the
    // moment one update arrives to the next, as the first chart may have been sent again just before its update.
    let values =
        |panel: &tomori_protocol::Panel| panel.chart.clone().unwrap().series[0].values.clone();
    let second = charted(&ada, Some(&panel)).panels[0].clone();
    let started = std::time::Instant::now();
    let third = charted(&ada, Some(&second)).panels[0].clone();
    assert!(started.elapsed() >= Duration::from_millis(500));
    for (earlier, later) in [(&panel, &second), (&second, &third)] {
        assert_eq!(
            (later.id.as_str(), &later.place),
            (panel.id.as_str(), &panel.place)
        );
        // A point or more newer: one a tick, unless a tick waited and the next replaced it.
        let (earlier, later) = (values(earlier), values(later));
        assert!(
            (1..=3).any(|shift| later[..POINTS - shift] == earlier[shift..]),
            "the window moves on by a point or so"
        );
    }
    // The agent reads it back as its environment's chart, and inspects it through the side.
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let token = token(&backend);
    let panels = call(&server, &token, &session, "panels", json!({}));
    let listed = &panels["panels"][0];
    assert_eq!(listed["kind"], "chart");
    assert_eq!(listed["environment_panel"], "pressure");
    assert_eq!(listed["inspect_id"], "gauges:chart/pressure");
    assert!(listed["body"].as_str().unwrap().contains("greatest: "));
    let inspected = call(
        &server,
        &token,
        &session,
        "inspect",
        json!({"id": "gauges:chart/pressure"}),
    );
    assert_eq!(inspected["details"]["metric"], "scaling pressure");
    assert!(
        inspected["description"]
            .as_str()
            .unwrap()
            .starts_with("Scaling pressure: 8 of 8 reporting; mean: ")
    );
    // The agent cannot rewrite it.
    let refused = {
        let params = json!({ "name": "update_panel", "arguments": {"panel": panel.id, "title": "Mine"},
            "_meta": { "ai.opencode/sessionID": session } });
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params })
            .to_string();
        let mut stream = TcpStream::connect(server.tools.unwrap()).unwrap();
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    assert!(
        refused.contains("only the environment updates it"),
        "{refused}"
    );
    // In another environment it is hidden with the environment's walls.
    ada.send(&ClientMessage::SetEnvironment {
        environment: "sky".to_owned(),
    });
    let elsewhere = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } if projection.environment == "sky" => {
            Some((**projection).clone())
        }
        _ => None,
    });
    assert!(elsewhere.panels.is_empty());
}

#[test]
fn an_environment_s_list_highlights_a_row_and_the_agent_inspects_a_row() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Queue".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some("queue".to_owned()),
        instructions: None,
    });
    ada.space(|state| state.title == "Queue");
    // Every client is sent a list, the highlighted row with its pointer.
    let body = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .find(|panel| panel.title == "Queue")
            .map(|panel| panel.body.clone()),
        _ => None,
    });
    assert_eq!(
        body,
        "- pod7 · 3 waiting\n- **▸** pod112 · 41 waiting\n- pod9 · 1 waiting"
    );
    // The agent reads its rows back, and inspects one by its id.
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let token = token(&backend);
    let panels = call(&server, &token, &session, "panels", json!({}));
    assert_eq!(
        panels["panels"][0]["rows"],
        json!([
            {"id": "pod7", "inspect_id": "queue:pod/7", "highlighted": false},
            {"id": "pod112", "inspect_id": "queue:pod/112", "highlighted": true},
            {"id": "pod9", "inspect_id": null, "highlighted": false},
        ])
    );
    let inspected = call(
        &server,
        &token,
        &session,
        "inspect",
        json!({"id": "queue:pod/112"}),
    );
    assert_eq!(inspected["details"], json!({"pod": "112", "waiting": 41}));
    assert_eq!(inspected["description"], "Queue: pod112 · 41 waiting");
}
