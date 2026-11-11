// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's server side: it owns the flock of each space bound to the farmyard, moves it on by the farmyard's
//! clock, the time since it started, takes participants' notices, keeps the campsite's marshmallow sticks, taking
//! participants' grabs, toasting and letting go, and shares every change with the clients in the space, with the clock,
//! through the environment state channel. See `design/farmyard.md`.
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tomori_environment::{Action, Error, Object, Outcome, Parameter, Result, ServerSide, Space};

use crate::campsite::{self, Campsite};
use crate::chicken::Chicken;
use crate::flock::{Event, Flock, Intent, PECKS, seed};

/// How long the side waits for an intent before moving the flock on, and how often it describes the chickens to the
/// space's agent.
pub const TICK: Duration = Duration::from_millis(50);
pub const DESCRIBE: Duration = Duration::from_secs(1);

/// The farmyard's server side.
#[derive(Default)]
pub struct FarmyardSide;

/// The actions the space's agent may take on a hen: having her peck at the ground, which changes only what the yard
/// shows, for a moment, so it needs nobody named. See `design/environment-actions.md`.
pub fn actions() -> Vec<Action> {
    vec![
        Action::new(
            "peck",
            "Have this hen peck at the ground for a moment, as if she found something",
        )
        .with(
            Parameter::number("pecks", "how many times she pecks", 1.0, PECKS as f64).stepped(1.0),
        ),
    ]
}

fn json(value: impl serde::Serialize) -> String {
    serde_json::to_string(&value).expect("the flock's messages are JSON")
}

/// The farmyard's state, as everyone in the space is sent it: the flock's, with the clock, and the campsite's sticks
/// beside it, which a client from before the campsite ignores.
pub fn state(flock: &Flock, campsite: &Campsite, clock: u64) -> String {
    let mut state = serde_json::to_value(flock.state(clock)).expect("the flock's state is JSON");
    state["sticks"] = serde_json::to_value(campsite.shared().sticks).expect("the sticks are JSON");
    json(state)
}

impl ServerSide for FarmyardSide {
    fn run(&self, space: &Space) -> Result<()> {
        run(space, Chicken::shared(), Instant::now())
    }
}

/// Run the flock of `space`, for `chicken`, its clock beginning at `start`, until the space's cancellation; or fail at
/// once, without the chicken, which could not be loaded.
pub fn run(space: &Space, chicken: Option<Arc<Chicken>>, start: Instant) -> Result<()> {
    let Some(chicken) = chicken else {
        return Err(Error::new("The farmyard's chickens could not be loaded."));
    };
    let clock = move || start.elapsed().as_millis() as u64;
    // Shared with the inspector, which the server asks on a thread of its own; held only briefly on either side.
    let shared = Arc::new(Mutex::new(Flock::new(seed(space.id()), chicken, clock())));
    let camp = Arc::new(Mutex::new(Campsite::new()));
    let (inspected, inspected_camp) = (shared.clone(), camp.clone());
    space.inspect_with(move |reference: &str| -> Result<Option<String>> {
        if reference.starts_with("campsite/") {
            let campsite = inspected_camp
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            return Ok(campsite.inspect(reference).map(json));
        }
        let flock = inspected.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(flock.inspect(reference, clock()).map(json))
    });
    let flock = shared.lock().unwrap_or_else(PoisonError::into_inner);
    space.share(state(
        &flock,
        &camp.lock().unwrap_or_else(PoisonError::into_inner),
        clock(),
    ))?;
    drop(flock);
    space.declare_actions(actions())?;
    let mut described: Option<Instant> = None;
    while !space.cancellation().is_cancelled() {
        let mut changes = Vec::new();
        let intent = space.next_intent(TICK);
        let mut flock = shared.lock().unwrap_or_else(PoisonError::into_inner);
        let mut campsite = camp.lock().unwrap_or_else(PoisonError::into_inner);
        let mut sticks = Vec::new();
        if let Some(acted) = intent.as_ref().and_then(|intent| intent.acted()) {
            // The server checked the action against what the side declared: only peck, with a whole number of pecks.
            let pecks = serde_json::from_str::<serde_json::Value>(acted.parameters())
                .ok()
                .and_then(|parameters| parameters["pecks"].as_f64())
                .unwrap_or(1.0) as u64;
            let outcome = match flock.peck(acted.reference(), pecks, clock()) {
                Ok((change, summary)) => {
                    changes.push(change);
                    Outcome::done(summary)
                }
                Err(reason) => Outcome::refused(reason),
            };
            space.answer(acted.ticket(), outcome);
        } else if let Some(intent) = intent {
            if let Ok(asked) = serde_json::from_str::<campsite::Intent>(intent.payload()) {
                sticks.extend(campsite.ask(intent.participant(), asked, clock()));
            } else {
                match serde_json::from_str::<Intent>(intent.payload()) {
                    Ok(asked) => changes.extend(flock.ask(intent.participant(), asked, clock())),
                    Err(_) => log::debug!("A farmyard intent could not be read."),
                }
            }
        }
        // A stick whose holder has left goes back on its log, and an empty space tidies itself after a while.
        sticks.extend(campsite.sweep(space.present().as_deref(), clock()));
        changes.extend(flock.update(clock()));
        // Each change, the flock's and then the sticks', announced with the state it leaves.
        let mut events: Vec<String> = changes
            .into_iter()
            .map(|(chicken, behaviour)| {
                json(Event {
                    clock: clock(),
                    chicken,
                    behaviour,
                })
            })
            .collect();
        events.extend(sticks.into_iter().map(|stick| {
            json(campsite::Event {
                clock: clock(),
                stick,
                now: campsite.sticks()[stick].clone(),
            })
        }));
        for event in events {
            space.announce(event, state(&flock, &campsite, clock()))?;
        }
        if described.is_none_or(|described| described.elapsed() >= DESCRIBE) {
            described = Some(Instant::now());
            let now = clock();
            // Each where she stands now, so that the agent can go to her when asked; and the campsite's bonfire and
            // sticks, a stick where it rests.
            let mut objects: Vec<Object> = flock
                .describe(now)
                .into_iter()
                .enumerate()
                .map(|(index, (id, reference, description))| {
                    Object::new(id, description)
                        .with_reference(reference)
                        .at(flock.position(index, now).to_array())
                        .offering(["peck"])
                })
                .collect();
            objects.extend(campsite.describe().into_iter().map(
                |(id, reference, description, at)| {
                    let object = Object::new(id, description).with_reference(reference);
                    match at {
                        Some(at) => object.at(at.to_array()),
                        None => object,
                    }
                },
            ));
            space.describe(objects)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
