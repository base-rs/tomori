// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Charts whose points are buckets, as the hub sends them: as they are to a client whose Hello says that it reads
//! them, and as instants to any other, whose place across one is then read on the chart it was sent, so that what an
//! older client draws, what its readout says and what the agent is told agree. See `design/projection.md`.
use tomori_environment::Outlet;
use tomori_environment::chart::{AxisLabel, Chart, Colour, Series, SeriesStyle, TimeAxis};
use tomori_protocol::{PatchOp, Projection, Resource};

use super::*;

impl Client {
    /// Connected and greeted as `participant`, by a client that reads what is projected, applying patches and their
    /// data ops to it, and reads charts of buckets or, as an older client, does not, with what that sent read.
    fn charting(hub: &Arc<Hub>, participant: &str, chart_buckets: bool) -> Self {
        let client = Self::connect(hub, Access::open());
        client.send(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: participant.to_owned(),
            names: false,
            environments: None,
            environment_state: false,
            projection: true,
            pairing_window: false,
            speech_held: false,
            images: false,
            diagrams: false,
            world_edges: false,
            projection_patches: true,
            live_panels: true,
            spawns: false,
            limits: None,
            content: false,
            chart_buckets,
        });
        client.read();
        client
    }

    /// What `held` is once everything sent since is applied to it: each projection sent whole, and each patch, with
    /// the ops of the patches.
    fn applied(&self, held: &mut Option<Projection>) -> Vec<PatchOp> {
        let mut ops = Vec::new();
        for message in self.read() {
            match message {
                ServerMessage::Projection { projection } => *held = Some(*projection),
                ServerMessage::ProjectionPatch { patch } => {
                    patch
                        .apply(held.as_mut().expect("sent whole first"))
                        .expect("a patch from what it holds");
                    ops.extend(patch.ops.iter().cloned());
                }
                _ => {}
            }
        }
        ops
    }
}

/// The requests counted in each 15 minutes of 24 hours, 96 buckets, the last ending now, `polls` polls in, each poll
/// moving them along one, labelled at their edges, and a line over the same 24 hours, 97 instants.
fn charts(polls: usize) -> [tomori_environment::Panel; 2] {
    let series = |name: &str, style: SeriesStyle, points: usize| {
        Series::new(
            name,
            Colour::Teal,
            style,
            (polls..polls + points)
                .map(|index| Some(index as f32))
                .collect(),
        )
    };
    let live = TimeAxis::Live { step_ms: 900_000 };
    let labels = |points: u32| {
        vec![
            AxisLabel::new(0, "-24 h"),
            AxisLabel::new(48, "-12 h"),
            AxisLabel::new(points, "now"),
        ]
    };
    [
        tomori_environment::Panel::chart(
            "volume",
            "Volume",
            Chart::new(96)
                .with_buckets(true)
                .with_time(live)
                .with_labels(labels(96))
                .with_series(series("requests", SeriesStyle::Bars, 96)),
        ),
        tomori_environment::Panel::chart(
            "latency",
            "Latency",
            Chart::new(97)
                .with_time(live)
                .with_labels(labels(96))
                .with_series(series("p99", SeriesStyle::Line, 97)),
        ),
    ]
}

#[test]
fn a_client_that_reads_no_buckets_is_sent_them_as_instants_and_read_as_it_shows_them() {
    let (hub, _jobs) = hub_waiting();
    hub.set_environments(Environments::new([
        crate::environments::Environment::new("sky", "Sky"),
        crate::environments::Environment::new("yard", "Yard")
            .with_walls([upright([3.0, 1.4, -4.0])])
            .with_server_side(Idle),
    ]));
    hub.serving();
    let mut bound = request("ada");
    bound.environment = "yard".to_owned();
    let idle = Snapshot {
        busy: Some(false),
        ..Snapshot::default()
    };
    let (id, commands) = running(&hub, &bound, idle);
    let outlet = SpaceOutlet {
        hub: Arc::downgrade(&hub),
        space: id.clone(),
        binding: hub.server_sides.binding(&id).unwrap(),
        environment: "yard".to_owned(),
        version: 1,
    };
    // Ada's client reads buckets, and says so; Old's is from before them, and says nothing.
    let ada = Client::charting(&hub, "ada", true);
    let old = Client::charting(&hub, "old", false);
    for client in [&ada, &old] {
        client.send(ClientMessage::Join { space: id.clone() });
    }
    for panel in charts(0) {
        assert_eq!(outlet.publish(panel), Ok(()));
    }
    let (mut seen, mut older) = (None, None);
    let paced = || {
        while let Some(due) = hub.snapshot_deadline() {
            hub.flush_snapshots(due);
        }
    };
    paced();
    ada.applied(&mut seen);
    old.applied(&mut older);
    let chart = |held: &Option<Projection>, title: &str| {
        let shown = held.as_ref().expect("sent what is projected");
        let panel = shown.panels.iter().find(|panel| panel.title == title);
        let panel = panel.expect("published");
        (panel.id.clone(), *panel.chart.clone().expect("a chart"))
    };
    // Ada is sent the buckets, labelled at their edges, from where the first begins to where the last ends.
    let (volume, buckets) = chart(&seen, "Volume");
    assert!(buckets.buckets);
    let at = |chart: &Chart| -> Vec<u32> { chart.labels.iter().map(|label| label.at).collect() };
    assert_eq!(at(&buckets), [0, 48, 96]);
    // Old is sent the same panel as instants, each where its bucket ends, with the same values and summary: `now`
    // under its last point, where it ignored a label at a point it has not, and the window's start, a step before its
    // first point, left out. A chart of instants is sent to both as it is.
    let (_, instants) = chart(&older, "Volume");
    assert_eq!(instants, buckets.clone().instants());
    assert!(!instants.buckets);
    assert_eq!(at(&instants), [47, 95]);
    assert!(instants.same_data(&buckets));
    assert_eq!(chart(&older, "Latency"), chart(&seen, "Latency"));
    let body =
        |held: &Option<Projection>| held.as_ref().unwrap().panel(&volume).unwrap().body.clone();
    assert_eq!(body(&older), body(&seen));
    // Old points at 94.4 of the 95 steps its chart spans, where its readout says nine minutes ago and reads the bar
    // it draws there, and its linked readout on the line says nine minutes ago too: the agent is told the same.
    let across = |at: f32, steps: f32| (at / steps * 10_000.0).round() as u16;
    let pointing = |client: &Client, across: u16| {
        let prompt = said(
            &hub,
            client,
            &commands,
            vec![Resource::chart_at(volume.clone(), across)],
            None,
        );
        hub.lock()
            .space_mut(&id)
            .unwrap()
            .model
            .sent(&prompt.key, Err("test".to_owned()));
        prompt.text
    };
    let told = pointing(&old, across(94.4, 95.0));
    assert!(
        told.contains(
            "; at the time pointed at: \"-9 min: requests 94\"; linked charts at that time: \"Latency: -9 min: \
             p99 95.4\")"
        ),
        "{told}"
    );
    assert_eq!(
        instants.read_at(instants.at_across(across(94.4, 95.0))),
        "-9 min: requests 94"
    );
    // Ada, pointing at the same share of her plot, is in the last bucket, nine minutes ago, and is told so.
    let told = pointing(&ada, across(94.4, 95.0));
    assert!(
        told.contains("; at the time pointed at: \"-9 min: requests 95\"; linked charts"),
        "{told}"
    );
    // A client that names what it pointed at only as its words end is read the same way.
    let prompt = said(
        &hub,
        &old,
        &commands,
        Vec::new(),
        Some(Resource::chart_at(volume.clone(), across(0.6, 95.0))),
    );
    assert!(
        prompt
            .text
            .contains("; at the time pointed at: \"-23 h 36 min: requests 1\""),
        "{}",
        prompt.text
    );
    // Its next values arrive as its live data alone, for both, and applied to what each holds are the chart each is
    // sent whole: the values are the same points, whichever way they are read.
    for panel in charts(1) {
        assert_eq!(
            hub.channel_publish_at(&outlet, panel, Instant::now() + Duration::from_secs(2)),
            Ok(())
        );
    }
    hub.flush_publishes(Instant::now() + Duration::from_secs(3));
    paced();
    for (client, held) in [(&ada, &mut seen), (&old, &mut older)] {
        let ops = client.applied(held);
        assert!(
            ops.iter()
                .all(|op| matches!(op, PatchOp::ChartAppend { .. }))
                && ops.len() == 2,
            "{ops:?}"
        );
    }
    let (_, moved) = chart(&seen, "Volume");
    assert_eq!(moved.series[0].values[95], Some(96.0));
    assert_eq!(chart(&older, "Volume").1, moved.instants());
}
