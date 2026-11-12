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

/// An example environment's side that publishes a queue of pods as a list panel, highlighting one, answers the agent's
/// inspection of a pod, and, as a participant chooses a pod, highlights the one chosen, as an inspector would open it.
/// See `design/panel-actions.md`.
struct Queue;

impl ServerSide for Queue {
    fn run(&self, space: &Space) -> Result<()> {
        space.inspect_with(|reference: &str| -> Result<Option<String>> {
            Ok(reference
                .strip_prefix("pod/")
                .map(|pod| json!({"pod": pod, "waiting": 41}).to_string()))
        });
        let queue = |highlighted: &str| {
            let rows = vec![
                Row::new("pod7", "pod7 · 3 waiting")
                    .with_reference("pod/7")
                    .choosable()
                    .with_tone(Tone::Good)
                    .with_value(3.0 / 50.0),
                Row::new("pod112", "pod112 · 41 waiting")
                    .with_reference("pod/112")
                    .choosable()
                    .with_tone(Tone::Bad)
                    .with_value(41.0 / 50.0),
                Row::new("pod9", "pod9 · 1 waiting"),
            ];
            Panel::list("queue", "Queue", rows).highlighting(highlighted)
        };
        space.publish(queue("pod112"))?;
        while !space.cancellation().is_cancelled() {
            // A choice is the server's, naming this side's own panel and a row it marked choosable; a client side's
            // intent, which this environment has none of, never is.
            if let Some(chosen) = space
                .next_intent(Duration::from_millis(20))
                .and_then(|intent| intent.chosen().cloned())
                && chosen.panel() == "queue"
            {
                space.publish(queue(chosen.row()))?;
            }
        }
        Ok(())
    }
}

/// An example environment's side that publishes the observers of a running system as a list laid out as a table, a
/// name, how busy each is, with its value under that column, and how long callers wait, and, as a participant chooses
/// one, highlights it. Its data is invented. See `design/panel-actions.md`.
struct Observers;

impl ServerSide for Observers {
    fn run(&self, space: &Space) -> Result<()> {
        use tomori_environment::Column;
        let table = |highlighted: &str| {
            let rows = [
                ("grpc", "GRPC", 100, "0 ms"),
                ("http", "HTTP", 10, "12 ms"),
                ("jobs", "Jobs", 1, "0 ms"),
            ]
            .map(|(id, name, busy, wait)| {
                Row::new(id, "")
                    .with_cells([name.to_owned(), format!("{busy}%"), wait.to_owned()])
                    .with_value(busy as f32 / 100.0)
                    .choosable()
            })
            .to_vec();
            Panel::list("observers", "Observers", rows)
                .with_columns(vec![
                    Column::new("Name"),
                    Column::number("Busy").with_value(),
                    Column::number("Wait"),
                    Column::new("Breaker").aligned(tomori_environment::CellAlign::Centre),
                ])
                .highlighting(highlighted)
        };
        space.publish(table("grpc"))?;
        while !space.cancellation().is_cancelled() {
            if let Some(chosen) = space
                .next_intent(Duration::from_millis(20))
                .and_then(|intent| intent.chosen().cloned())
            {
                space.publish(table(chosen.row()))?;
            }
        }
        Ok(())
    }
}

/// How many regions the Terminals example shows, each a terminal of three charts and a list.
const REGIONS: usize = 8;

/// An example environment's side that shows as many panels as an environment may, as a console of terminals does:
/// three live charts and a list of pods for each of eight regions, every chart a point newer each tick, and, once they
/// are up, tries one more, keeping what the server said of it. See `design/projection.md`.
struct Terminals {
    /// What the server said of one panel more than it takes.
    refused: Arc<Mutex<Option<String>>>,
}

impl Terminals {
    /// The panels of the tick whose newest sample is `newest`: each region's charts on its wall's top row, its list
    /// below them.
    fn panels(newest: usize) -> Vec<Panel> {
        let mut panels = Vec::new();
        for region in 0..REGIONS {
            for (index, metric) in ["pressure", "latency", "errors"].into_iter().enumerate() {
                let id = format!("{metric}-{region}");
                let title = format!("Region {region} {metric}");
                let chart = pressure(newest + region + index);
                panels.push(Panel::chart(id, title, chart).on_wall(region as u32 / 2, None));
            }
            let rows = (0..tomori_environment::ROW_LIMIT)
                .map(|pod| Row::new(format!("pod{pod}"), format!("pod{pod} · {pod} waiting")))
                .collect();
            let list = Panel::list(
                format!("pods-{region}"),
                format!("Region {region} pods"),
                rows,
            );
            panels.push(list.on_wall(region as u32 / 2, None));
        }
        panels
    }
}

impl ServerSide for Terminals {
    fn run(&self, space: &Space) -> Result<()> {
        let mut next = POINTS - 1;
        while !space.cancellation().is_cancelled() {
            for panel in Self::panels(next) {
                space.publish(panel)?;
            }
            let mut refused = self.refused.lock().unwrap();
            if refused.is_none() {
                let more = space.publish(Panel::text("more", "More", "One more than it may show."));
                *refused = Some(more.unwrap_err().to_string());
            }
            drop(refused);
            next += 1;
            std::thread::sleep(TICK);
        }
        Ok(())
    }
}

/// A server serving Tomori's tools, offering Sky, Gauges, Queue and Observers, each of the last three with one wall of
/// its own.
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
            Environment::new("observers", "Observers")
                .with_walls([
                    tomori_protocol::Pose::at([2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])
                ])
                .with_server_side(Observers),
        ])
    })
}

/// An example environment's side that changes a panel once more just after telling the space of a change, within the
/// quarter of a second in which the server tells a space's clients of its environment's changes once, and then
/// nothing more: the first note, then, when `go` is set, a second, and 120 ms after that a third.
struct Paced {
    go: Arc<std::sync::atomic::AtomicBool>,
}

impl ServerSide for Paced {
    fn run(&self, space: &Space) -> Result<()> {
        let note = |body: &str| Panel::text("note", "Note", body);
        space.publish(note("first"))?;
        let mut sent = false;
        while !space.cancellation().is_cancelled() {
            if !sent && self.go.load(std::sync::atomic::Ordering::Acquire) {
                space.publish(Panel::text("other", "Other", "second"))?;
                std::thread::sleep(Duration::from_millis(120));
                space.publish(Panel::text("last", "Last", "third"))?;
                sent = true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

/// A server offering Sky and Paced, which has a wall of its own, its side changing panels when `go` is set.
fn start_paced(backend: &FakeBackend, go: Arc<std::sync::atomic::AtomicBool>) -> TestServer {
    TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_environments(vec![
            Environment::new("sky", "Sky"),
            Environment::new("paced", "Paced")
                .with_walls([
                    tomori_protocol::Pose::at([2.0, 1.4, -3.0]).facing_towards([0.0, 1.4, 0.0])
                ])
                .with_server_side(Paced { go }),
        ])
    })
}

/// A server offering Sky and Terminals, which has four walls of its own of four by two slots, a terminal a row of each,
/// and says into `refused` what the server said of one panel more than it takes.
fn start_terminals(backend: &FakeBackend, refused: Arc<Mutex<Option<String>>>) -> TestServer {
    TestServer::with_tools(backend.clone(), |backends| {
        let walls = [-4.5_f32, -1.5, 1.5, 4.5].map(|x| {
            tomori_server::EnvironmentWall::new(
                tomori_protocol::Pose::at([x, 1.4, -4.0]).facing_towards([x, 1.4, 0.0]),
            )
            .with_grid(tomori_protocol::Grid::from((4, 2)))
        });
        Server::new(backends, Arc::new(TextTranscriber::default())).with_environments(vec![
            Environment::new("sky", "Sky"),
            Environment::new("terminals", "Terminals")
                .with_walls(walls)
                .with_server_side(Terminals { refused }),
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
    // As it is sent, its text and its structured content both, no answer is longer than an agent reads at once.
    assert!(body.len() <= 32 * 1024, "{} bytes", body.len());
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
    assert!(listed["excerpt"].as_str().unwrap().contains("mean: "));
    assert!(listed.get("body").is_none());
    // Named, it is read whole: its summary as it is now.
    let read = call(
        &server,
        &token,
        &session,
        "panels",
        json!({"panel": "Scaling pressure"}),
    );
    let read = &read["panels"][0];
    assert_eq!(read["panel"], listed["panel"]);
    assert!(read["body"].as_str().unwrap().contains("greatest: "));
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
    assert_eq!(panels["panels"][0]["row_count"], 3);
    assert!(panels["panels"][0].get("rows").is_none());
    let read = call(
        &server,
        &token,
        &session,
        "panels",
        json!({"panel": "Queue"}),
    );
    assert_eq!(
        read["panels"][0]["rows"],
        json!([
            {"id": "pod7", "inspect_id": "queue:pod/7", "highlighted": false, "choosable": true,
                "tone": "good", "value": 0.06, "text": "pod7 · 3 waiting"},
            {"id": "pod112", "inspect_id": "queue:pod/112", "highlighted": true, "choosable": true,
                "tone": "bad", "value": 0.82, "text": "pod112 · 41 waiting"},
            {"id": "pod9", "inspect_id": null, "highlighted": false, "choosable": false,
                "tone": "neutral", "value": null, "text": "pod9 · 1 waiting"},
        ])
    );
    assert!(read["panels"][0].get("body").is_none());
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

#[test]
fn an_environment_shows_as_many_panels_as_it_may_every_chart_live_and_is_refused_one_more() {
    let backend = FakeBackend::new();
    let refused = Arc::new(Mutex::new(None));
    let server = start_terminals(&backend, refused.clone());
    let mut ada = TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Terminals".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some("terminals".to_owned()),
        instructions: None,
    });
    ada.space(|state| state.title == "Terminals");
    // Every panel it may show is up, each on a wall of its own environment's, three charts and a list a region.
    let limit = tomori_environment::PANEL_LIMIT;
    assert_eq!(Terminals::panels(0).len(), limit);
    let whole = |ada: &TestClient, newer: &dyn Fn(&Projection) -> bool| {
        ada.wait_for(|message| match message {
            ServerMessage::Projection { projection }
                if projection.panels.len() == limit && newer(projection) =>
            {
                Some((**projection).clone())
            }
            _ => None,
        })
    };
    let first = whole(&ada, &|_| true);
    let charts = |projection: &Projection| -> Vec<(String, Vec<Option<f32>>)> {
        projection
            .panels
            .iter()
            .filter_map(|panel| {
                let chart = panel.chart.as_ref()?;
                Some((panel.id.clone(), chart.series[0].values.clone()))
            })
            .collect()
    };
    let began = charts(&first);
    assert_eq!(began.len(), 3 * REGIONS);
    assert!(first.panels.iter().all(|panel| matches!(
        &panel.place,
        Place::Wall { wall, .. } if wall.starts_with("environment-")
    )));
    let lists = first.panels.iter().filter(|panel| panel.chart.is_none());
    assert!(lists.clone().count() == REGIONS);
    assert!(
        lists
            .into_iter()
            .all(|panel| panel.body.lines().count() == 30)
    );
    // One more is refused, saying how many it may show, and nothing of the rest changes for it.
    let said = eventually(|| refused.lock().unwrap().clone());
    assert_eq!(
        said,
        "An environment publishes at most 32 panels in a space."
    );
    // Each chart is live: published again every second, every one of them moves on, in place.
    let later = whole(&ada, &|projection| {
        let now = charts(projection);
        now.len() == began.len()
            && now
                .iter()
                .zip(&began)
                .all(|(now, began)| now.0 == began.0 && now.1 != began.1)
    });
    assert_eq!(later.panels.len(), limit);
    // The agent reads every one back as its environment's.
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let token = token(&backend);
    let panels = call(&server, &token, &session, "panels", json!({}));
    let listed = panels["panels"].as_array().unwrap();
    assert_eq!(listed.len(), limit);
    assert!(
        listed
            .iter()
            .all(|panel| panel["environment_panel"].is_string())
    );
    let kinds = |kind: &str| listed.iter().filter(|panel| panel["kind"] == kind).count();
    assert_eq!((kinds("chart"), kinds("text")), (3 * REGIONS, REGIONS));
    // Listed briefly, a console of 24 charts and 8 lists of 30 rows is within what an agent reads at once, where
    // with every panel's text, items and rows it was 44 KB.
    let lists = listed.iter().filter(|panel| panel["row_count"] == 30);
    assert_eq!(lists.count(), REGIONS);
    assert_eq!(panels["total"], limit);
    assert!(panels.get("next").is_none());
}

#[test]
fn a_change_within_the_interval_of_the_last_is_sent_as_it_ends_though_nothing_else_happens() {
    let backend = FakeBackend::new();
    let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server = start_paced(&backend, go.clone());
    let mut ada = TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Paced".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some("paced".to_owned()),
        instructions: None,
    });
    ada.space(|state| state.title == "Paced");
    // When a projection showing the panel titled `title` arrives, and how many panels it shows.
    let shown = |ada: &TestClient, title: &str| {
        ada.wait_for(|message| match message {
            ServerMessage::Projection { projection } => projection
                .panels
                .iter()
                .any(|panel| panel.title == title)
                .then(|| (std::time::Instant::now(), projection.panels.len())),
            _ => None,
        })
    };
    shown(&ada, "Note");
    // Its side's first note was told long enough ago, so the second change is told at once; the third, 120 ms later,
    // is within the interval, and waits. Nothing else then happens in the space: nobody speaks, moves or changes
    // anything, and its side publishes no more. The server's timer sends it as the interval ends, a quarter of a
    // second after the second, and not before: were it not woken for it, the third would never arrive.
    std::thread::sleep(Duration::from_millis(300));
    go.store(true, std::sync::atomic::Ordering::Release);
    let (second, panels) = shown(&ada, "Other");
    let (third, _) = shown(&ada, "Last");
    // Unless this machine was so slow to send the second that the third was already published, they come apart.
    if panels == 2 {
        let waited = third.duration_since(second);
        assert!(waited >= Duration::from_millis(50), "{waited:?}");
    }
}

#[test]
fn choosing_a_row_of_an_environment_s_list_reaches_its_side_which_highlights_it() {
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
    let state = ada.space(|state| state.title == "Queue");
    assert!(state.choices, "the server takes choices");
    let list = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .find(|panel| panel.title == "Queue")
            .cloned(),
        _ => None,
    });
    assert_eq!(
        list.rows,
        [
            tomori_protocol::ListRow::new("pod7", true)
                .with_tone(Tone::Good)
                .with_value(0.06),
            tomori_protocol::ListRow::new("pod112", true)
                .with_highlight(true)
                .with_tone(Tone::Bad)
                .with_value(0.82),
            tomori_protocol::ListRow::new("pod9", false),
        ]
    );
    // Ada chooses pod7, by its id and the version she saw; its side is handed the choice, and highlights it.
    ada.send(&ClientMessage::ChoosePanelRow {
        panel: list.id.clone(),
        row: "pod7".into(),
        revision: Some(list.version),
    });
    let body = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .find(|panel| panel.id == list.id && panel.body.contains("**▸** pod7"))
            .map(|panel| panel.body.clone()),
        _ => None,
    });
    assert_eq!(
        body,
        "- **▸** pod7 · 3 waiting\n- pod112 · 41 waiting\n- pod9 · 1 waiting"
    );
    // A row it did not mark choosable is refused, naming the panel, and its side is told nothing.
    ada.send(&ClientMessage::ChoosePanelRow {
        panel: list.id.clone(),
        row: "pod9".into(),
        revision: None,
    });
    let refused = ada.wait_for(|message| match message {
        ServerMessage::Error { error, panel, .. } => Some((error.clone(), panel.clone())),
        _ => None,
    });
    assert_eq!(
        refused,
        (
            "That row cannot be chosen.".to_owned(),
            Some(list.id.clone())
        )
    );
}

#[test]
fn an_environment_s_table_reaches_clients_in_columns_the_agent_reads_its_cells_and_a_row_is_chosen()
{
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Observers".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some("observers".to_owned()),
        instructions: None,
    });
    ada.space(|state| state.title == "Observers");
    let table = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .find(|panel| panel.title == "Observers")
            .cloned(),
        _ => None,
    });
    // An older client draws the body, each row's text written from its cells; a newer one, its columns and cells.
    assert_eq!(
        table.body,
        "- **▸** GRPC · 100% · 0 ms\n- HTTP · 10% · 12 ms\n- Jobs · 1% · 0 ms"
    );
    assert_eq!(
        serde_json::to_value(&table.columns).unwrap(),
        json!([
            {"title": "Name"},
            {"title": "Busy", "align": "right", "value": true},
            {"title": "Wait", "align": "right"},
            {"title": "Breaker", "align": "centre"},
        ])
    );
    assert_eq!(table.rows[1].cells, ["HTTP", "10%", "12 ms"]);
    // The agent reads its columns and each row's cells.
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let token = token(&backend);
    let read = call(
        &server,
        &token,
        &session,
        "panels",
        json!({"panel": "Observers"}),
    );
    assert_eq!(
        read["panels"][0]["columns"],
        json!([
            {"title": "Name", "align": "left", "value": false},
            {"title": "Busy", "align": "right", "value": true},
            {"title": "Wait", "align": "right", "value": false},
            {"title": "Breaker", "align": "centre", "value": false},
        ])
    );
    assert_eq!(
        read["panels"][0]["rows"][0]["cells"],
        json!(["GRPC", "100%", "0 ms"])
    );
    assert!(read["panels"][0]["rows"][0].get("text").is_none());
    // Choosing a row of the table reaches its side by the row's id, which highlights it.
    ada.send(&ClientMessage::ChoosePanelRow {
        panel: table.id.clone(),
        row: "jobs".into(),
        revision: Some(table.version),
    });
    let highlighted = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .find(|panel| panel.id == table.id && panel.rows[2].highlighted)
            .map(|panel| panel.rows[2].cells.clone()),
        _ => None,
    });
    assert_eq!(highlighted, ["Jobs", "1%", "0 ms"]);
}
