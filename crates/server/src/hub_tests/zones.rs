// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where participants are, as an environment's server side learns it through the hub: the occupancy of the zones it
//! declares, told as an intent only the server makes, by participants' heads alone, never a visitor's or a stale
//! report's, and only to the side of the binding and space that declared them; and the snapshot of where each
//! participant is that it reads. See `design/environments.md`.
use super::*;
use tomori_environment::{Around, Intent, Occupancy, Presence, Zone};
use tomori_protocol::Pose;
use tomori_protocol::cluster::Anchor;

/// What a side was handed, and where it last read that participants are.
#[derive(Clone, Default)]
struct Kept {
    intents: Arc<Mutex<Vec<Intent>>>,
    presence: Arc<Mutex<Vec<Presence>>>,
}

impl Kept {
    /// The occupancies it was told, by zone and count, taken.
    fn occupancies(&self) -> Vec<(String, u32)> {
        let mut intents = self.intents.lock().unwrap();
        let taken: Vec<(String, u32)> = intents
            .iter()
            .filter_map(Intent::occupancy)
            .map(|occupancy| (occupancy.zone().to_owned(), occupancy.count()))
            .collect();
        intents.retain(|intent| intent.occupancy().is_none());
        taken
    }

    /// Wait until it has been told `expected`, and nothing else.
    fn told(&self, what: &str, expected: &[(&str, u32)]) {
        let expected: Vec<(String, u32)> = expected
            .iter()
            .map(|(zone, count)| ((*zone).to_owned(), *count))
            .collect();
        let gathered = std::cell::RefCell::new(Vec::new());
        eventually_in(what, || {
            gathered.borrow_mut().extend(self.occupancies());
            gathered.borrow().len() >= expected.len()
        });
        // Nothing more than it was told.
        std::thread::sleep(Duration::from_millis(20));
        gathered.borrow_mut().extend(self.occupancies());
        let gathered = gathered.into_inner();
        assert_eq!(gathered, expected, "{what}");
    }
}

/// A side that keeps everything it is handed, and reads where participants are as it goes.
struct Keeping(Kept);

impl tomori_environment::ServerSide for Keeping {
    fn run(&self, space: &tomori_environment::Space) -> tomori_environment::Result<()> {
        while !space.cancellation().is_cancelled() {
            if let Some(intent) = space.next_intent(Duration::from_millis(2)) {
                self.0.intents.lock().unwrap().push(intent);
            }
            *self.0.presence.lock().unwrap() = space.presence();
        }
        Ok(())
    }
}

struct Deck {
    hub: Arc<Hub>,
    _jobs: async_channel::Receiver<Job>,
    space: String,
    outlet: SpaceOutlet,
    kept: Kept,
    ada: Client,
}

/// A deck whose participants arrive 8 m along +Z of its origin, facing it, with a console anchor 4 m before them, its
/// side keeping what it is handed; a space on it, and Ada in it.
fn deck() -> Deck {
    let (hub, jobs) = hub_waiting();
    let kept = Kept::default();
    let arrival = Pose::at([0.0, 0.0, 8.0]).facing_towards([0.0, 0.0, 0.0]);
    hub.set_environments(Environments::new([
        crate::environments::Environment::new("sky", "Sky"),
        crate::environments::Environment::new("deck", "Deck")
            .with_arrival(arrival)
            .with_anchor(Anchor::new("console/2", Pose::at([0.0, 1.4, 4.0])))
            .with_server_side(Keeping(kept.clone())),
    ]));
    hub.serving();
    let mut bound = request("ada");
    bound.environment = "deck".to_owned();
    let (space, _commands) = running(&hub, &bound, Snapshot::default());
    let outlet = outlet(&hub, &space);
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&space);
    Deck {
        hub,
        _jobs: jobs,
        space,
        outlet,
        kept,
        ada,
    }
}

/// An outlet of the deck's side in `space`, of its binding now.
fn outlet(hub: &Arc<Hub>, space: &str) -> SpaceOutlet {
    SpaceOutlet {
        hub: Arc::downgrade(hub),
        space: space.to_owned(),
        binding: hub.server_sides.binding(space).unwrap(),
        environment: "deck".to_owned(),
        version: 1,
    }
}

/// The console, a cylinder 1.5 m about its anchor from 1 m below it to 1 m above; and a well, 1 m about a point by the
/// arrival, in the deck's own coordinates.
fn zones() -> Vec<Zone> {
    vec![
        Zone::cylinder("console", Around::anchor("console/2"), 1.5, 1.0, 1.0),
        Zone::sphere("well", Around::point([3.0, 1.6, 8.0]), 1.0),
    ]
}

/// The number the hub gives `client`'s participant.
fn source(hub: &Hub, client: &Client) -> u32 {
    let state = hub.lock();
    let key = state.connections[&client.connection]
        .participant
        .clone()
        .unwrap();
    state.participants[&key].source
}

#[test]
fn a_side_is_told_each_zone_as_it_declares_them_and_as_participants_enter_and_leave() {
    let deck = deck();
    let hub = &deck.hub;
    assert_eq!(hub.channel_declare_zones(&deck.outlet, zones()), Ok(()));
    // Told each, though nobody is there, at the next sweep, which Ada, who has not reported, does not count in.
    hub.sweep_zones(Instant::now());
    deck.kept
        .told("each zone as declared", &[("console", 0), ("well", 0)]);
    // Ada walks to the console, 4 m ahead of home in the space's coordinates, where the deck's anchor stands carried.
    deck.ada.send(standing([0.5, 1.6, -4.5]));
    hub.sweep_zones(Instant::now());
    deck.kept.told("Ada at the console", &[("console", 1)]);
    hub.sweep_zones(Instant::now());
    deck.kept.told("nothing as she stays", &[]);
    // And on to the well, 3 m to her right as she arrives.
    deck.ada.send(standing([3.0, 1.6, 0.2]));
    hub.sweep_zones(Instant::now());
    deck.kept.told(
        "Ada from the console to the well",
        &[("console", 0), ("well", 1)],
    );
    // A visitor, who reports no presence, is never counted.
    let vic = Client::greeted(hub, Access::visitor(), "vic");
    vic.join(&deck.space);
    vic.send(standing([0.0, 1.6, -4.0]));
    hub.sweep_zones(Instant::now());
    deck.kept.told("nothing of the visitor", &[]);
    // A second participant at the well counts with her; leaving the space, she no longer does.
    let ben = Client::greeted(hub, Access::open(), "ben");
    ben.join(&deck.space);
    ben.send(standing([3.5, 1.6, 0.0]));
    hub.sweep_zones(Instant::now());
    deck.kept.told("Ben joins her", &[("well", 2)]);
    hub.disconnect(deck.ada.connection);
    hub.sweep_zones(Instant::now());
    deck.kept.told("Ada gone", &[("well", 1)]);
}

#[test]
fn a_stale_report_counts_as_absent_and_the_snapshot_says_how_old_each_is() {
    let deck = deck();
    let hub = &deck.hub;
    assert_eq!(hub.channel_declare_zones(&deck.outlet, zones()), Ok(()));
    let reported = Instant::now();
    deck.ada.send(standing([0.0, 1.6, -4.0]));
    hub.sweep_zones(reported);
    deck.kept
        .told("Ada at the console", &[("console", 1), ("well", 0)]);
    // Just short of stale, she is still there; once stale, as her headset sleeps, she is not.
    let at = hub
        .lock()
        .participants
        .values()
        .find_map(|participant| participant.seen.as_ref().map(|seen| seen.at));
    let at = at.unwrap();
    hub.sweep_zones(at + crate::presence::STALE - Duration::from_millis(1));
    deck.kept.told("not yet stale", &[]);
    hub.sweep_zones(at + crate::presence::STALE);
    deck.kept.told("stale, absent", &[("console", 0)]);
    // Reporting again, she is there again.
    deck.ada.send(standing([0.0, 1.6, -4.0]));
    hub.sweep_zones(Instant::now());
    deck.kept.told("reporting again", &[("console", 1)]);

    // What the side reads of where she is says how engaged she is: lifting her headset off, her client says she is
    // away and reports nothing more, which the side is told at once, and the console counts her out.
    let presence = || deck.kept.presence.lock().unwrap().clone();
    eventually_in("the side reads her, engagement unsaid", || {
        presence()
            .first()
            .is_some_and(|her| her.engagement.is_none())
    });
    deck.ada.send(ClientMessage::Engagement {
        state: tomori_protocol::Engagement::Away,
    });
    eventually_in("the side reads that she is away", || {
        presence()
            .first()
            .is_some_and(|her| her.engagement == Some(tomori_protocol::Engagement::Away))
    });
    let presence = presence();
    assert_eq!(presence.len(), 1, "{presence:?}");
    assert_eq!(presence[0].participant, source(hub, &deck.ada));
    hub.sweep_zones(Instant::now());
    deck.kept.told("away, absent", &[("console", 0)]);
    // Putting it back on, she is there again.
    deck.ada.send(ClientMessage::Engagement {
        state: tomori_protocol::Engagement::Engaged,
    });
    hub.sweep_zones(Instant::now());
    deck.kept.told("engaged again", &[("console", 1)]);
}

#[test]
fn a_change_waits_for_its_dwell_and_declaring_again_tells_each_zone_as_it_is() {
    let deck = deck();
    let hub = &deck.hub;
    let slow =
        Zone::sphere("well", Around::point([3.0, 1.6, 8.0]), 1.0).dwelling(Duration::from_secs(10));
    assert_eq!(hub.channel_declare_zones(&deck.outlet, vec![slow]), Ok(()));
    let start = Instant::now();
    hub.sweep_zones(start);
    deck.kept.told("the well, empty", &[("well", 0)]);
    deck.ada.send(standing([3.0, 1.6, 0.0]));
    hub.sweep_zones(start + Duration::from_secs(1));
    hub.sweep_zones(start + Duration::from_secs(4));
    deck.kept.told("not yet her dwell", &[]);
    // Declared again, as the side's terminal changes, each zone is told as it is, without waiting.
    assert_eq!(hub.channel_declare_zones(&deck.outlet, zones()), Ok(()));
    hub.sweep_zones(start + Duration::from_secs(5));
    deck.kept
        .told("declared again", &[("console", 0), ("well", 1)]);
    // Declaring none tells nothing more.
    assert_eq!(hub.channel_declare_zones(&deck.outlet, Vec::new()), Ok(()));
    hub.sweep_zones(start + Duration::from_secs(6));
    deck.kept.told("none", &[]);
}

#[test]
fn a_zone_around_an_object_follows_where_the_side_describes_it() {
    let deck = deck();
    let hub = &deck.hub;
    let lamp = |at: [f32; 3]| vec![tomori_environment::Object::new("lamp-1", "a lamp").at(at)];
    assert_eq!(
        hub.channel_describe(&deck.outlet, lamp([-3.0, 1.6, 8.0])),
        Ok(())
    );
    let zone = Zone::sphere("lamp", Around::object("lamp-1"), 1.0);
    assert_eq!(hub.channel_declare_zones(&deck.outlet, vec![zone]), Ok(()));
    deck.ada.send(standing([-3.0, 1.6, 0.5]));
    hub.sweep_zones(Instant::now());
    deck.kept.told("Ada by the lamp", &[("lamp", 1)]);
    // Described further off, she is no longer by it; described nowhere, nobody is.
    assert_eq!(
        hub.channel_describe(&deck.outlet, lamp([-3.0, 1.6, 4.0])),
        Ok(())
    );
    hub.sweep_zones(Instant::now());
    deck.kept.told("the lamp moved away", &[("lamp", 0)]);
    assert_eq!(
        hub.channel_describe(&deck.outlet, lamp([-3.0, 1.6, 8.0])),
        Ok(())
    );
    hub.sweep_zones(Instant::now());
    deck.kept.told("the lamp back", &[("lamp", 1)]);
    assert_eq!(hub.channel_describe(&deck.outlet, Vec::new()), Ok(()));
    hub.sweep_zones(Instant::now());
    deck.kept.told("the lamp gone", &[("lamp", 0)]);
}

#[test]
fn a_declaration_sooner_than_the_interval_waits_for_the_next_sweep_after_it() {
    let deck = deck();
    let hub = &deck.hub;
    assert_eq!(hub.channel_declare_zones(&deck.outlet, zones()), Ok(()));
    let well = Zone::sphere("well", Around::point([3.0, 1.6, 8.0]), 1.0);
    assert_eq!(hub.channel_declare_zones(&deck.outlet, vec![well]), Ok(()));
    let now = Instant::now();
    hub.sweep_zones(now);
    deck.kept.told("the first", &[("console", 0), ("well", 0)]);
    hub.sweep_zones(now + tomori_environment::ZONE_INTERVAL);
    deck.kept.told("the second, the well alone", &[("well", 0)]);
}

#[test]
fn a_declaration_out_of_bounds_is_refused_keeping_nothing() {
    let deck = deck();
    let hub = &deck.hub;
    let many: Vec<Zone> = (0..=tomori_environment::ZONE_LIMIT)
        .map(|index| Zone::sphere(format!("zone/{index}"), Around::point([0.0; 3]), 1.0))
        .collect();
    let refused = hub.channel_declare_zones(&deck.outlet, many).unwrap_err();
    assert!(refused.message().contains("more than the 16"), "{refused}");
    // A point beyond 20 m of home, once carried, is refused too.
    let far = Zone::sphere("far", Around::point([0.0, 1.6, 30.0]), 1.0);
    let refused = hub
        .channel_declare_zones(&deck.outlet, vec![far])
        .unwrap_err();
    assert_eq!(
        refused.message(),
        "The zone \"far\" is around a point more than 20 m from home."
    );
    hub.sweep_zones(Instant::now());
    deck.kept.told("nothing kept", &[]);
}

#[test]
fn a_side_learns_only_of_its_own_zones() {
    let deck = deck();
    let hub = &deck.hub;
    // Another space on the deck, its own side running, with Ben at its console.
    let mut bound = request("ben");
    bound.environment = "deck".to_owned();
    bound.directory = "/tmp/other".to_owned();
    let (other, _commands) = running(hub, &bound, Snapshot::default());
    let ben = Client::greeted(hub, Access::open(), "ben");
    ben.join(&other);
    ben.send(standing([0.0, 1.6, -4.0]));
    // Ada's space declares; the other's side is told nothing, though Ben stands in a zone like it.
    assert_eq!(hub.channel_declare_zones(&deck.outlet, zones()), Ok(()));
    hub.sweep_zones(Instant::now());
    deck.kept
        .told("Ada's space alone", &[("console", 0), ("well", 0)]);
    // A declaration of an earlier binding, or another space's outlet, is kept nowhere.
    let earlier = SpaceOutlet {
        binding: deck.outlet.binding + 1000,
        ..outlet(hub, &deck.space)
    };
    assert_eq!(hub.channel_declare_zones(&earlier, zones()), Ok(()));
    hub.sweep_zones(Instant::now());
    deck.kept.told("nothing of an earlier binding", &[]);
    // Moved to another environment and back, the side runs anew, and what was declared is forgotten.
    deck.ada.send(ClientMessage::SetEnvironment {
        environment: "sky".to_owned(),
    });
    deck.ada.send(ClientMessage::SetEnvironment {
        environment: "deck".to_owned(),
    });
    deck.ada.send(standing([0.0, 1.6, -4.0]));
    hub.sweep_zones(Instant::now());
    deck.kept.told("forgotten with its binding", &[]);
    // Nothing it kept, zones included, outlives its binding.
    assert!(hub.lock().space(&deck.space).unwrap().channel.is_none());
}

#[test]
fn a_client_cannot_forge_an_occupancy() {
    let deck = deck();
    let hub = &deck.hub;
    deck.ada.send(ClientMessage::EnvironmentIntent {
        environment: "deck".into(),
        version: 1,
        intent: tomori_protocol::Payload::new(serde_json::json!({
            "occupancy": {"zone": "console", "count": 9},
            "zone": "console",
        })),
    });
    let ada = source(hub, &deck.ada);
    eventually_in("the side is handed her intent", || {
        !deck.kept.intents.lock().unwrap().is_empty()
    });
    let intents = deck.kept.intents.lock().unwrap().clone();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].participant(), ada);
    assert_eq!(intents[0].occupancy(), None::<&Occupancy>);
}
