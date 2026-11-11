// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's server side: it owns the flock of each space bound to the farmyard, moves it on by the farmyard's
//! clock, the time since it started, takes participants' notices, keeps the campsite's marshmallow sticks, taking
//! participants' grabs, toasting and letting go, simulates the beach's stones, taking participants' taking and letting
//! go of them, and shares every change with the clients in the space, with the clock, through the environment state
//! channel. Behind the crate's `side` feature, which builds the physics engine that only a server needs. See
//! `design/farmyard.md`.
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tomori_environment::{Action, Error, Object, Outcome, Parameter, Result, ServerSide, Space};

use crate::campsite::{self, Campsite};
use crate::chicken::Chicken;
use crate::flock::{Event, Flock, Intent, PECKS, seed};
pub use shingle::Shingle;

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

pub mod shingle;

fn json(value: impl serde::Serialize) -> String {
    serde_json::to_string(&value).expect("the flock's messages are JSON")
}

/// The farmyard's state, as everyone in the space is sent it: the flock's, with the clock, and the campsite's sticks
/// and the beach's stones beside it, which a client from before them ignores.
pub fn state(
    flock: &Flock,
    campsite: &Campsite,
    stones: &tomori_physics::message::State,
    clock: u64,
) -> String {
    let mut state = serde_json::to_value(flock.state(clock)).expect("the flock's state is JSON");
    state["sticks"] = serde_json::to_value(campsite.shared().sticks).expect("the sticks are JSON");
    state["stones"] = serde_json::to_value(stones).expect("the stones are JSON");
    json(state)
}

pub use crate::beach::Event as Stones;

/// The most events the side keeps waiting to be announced: beyond it, the oldest is dropped, which the log says.
pub const WAITING: usize = 1024;

/// The side's own pace of announcing, held to the rate the server takes a side's updates at,
/// [`tomori_environment::UPDATES_PER_SECOND`] after a burst of [`tomori_environment::UPDATE_BURST`], so that a burst of
/// changes, as several participants taking stones and sticks at once, waits its turn, in order, rather than being
/// refused: the events waiting, and what it may still announce, as of when.
pub struct Pace {
    waiting: std::collections::VecDeque<String>,
    tokens: f32,
    at: Instant,
}

impl Pace {
    /// The pace at `now`, having just shared the state, which the server counts.
    pub fn new(now: Instant) -> Self {
        Self {
            waiting: std::collections::VecDeque::new(),
            tokens: tomori_environment::UPDATE_BURST as f32 - 1.0,
            at: now,
        }
    }

    /// `event`, to be announced in turn.
    pub fn push(&mut self, event: String) {
        if self.waiting.len() >= WAITING {
            self.waiting.pop_front();
            log::warn!("A farmyard update was dropped, as too many were waiting.");
        }
        self.waiting.push_back(event);
    }

    /// The events that may be announced at `now`, in order, the rest waiting.
    pub fn due(&mut self, now: Instant) -> Vec<String> {
        let elapsed = now
            .saturating_duration_since(self.at)
            .min(Duration::from_secs(60));
        self.at = self.at.max(now);
        self.tokens = (self.tokens
            + elapsed.as_secs_f32() * tomori_environment::UPDATES_PER_SECOND as f32)
            .min(tomori_environment::UPDATE_BURST as f32);
        let count = (self.tokens.floor() as usize).min(self.waiting.len());
        self.tokens -= count as f32;
        self.waiting.drain(..count).collect()
    }
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
    let mut shingle = Shingle::new(Instant::now());
    let stones = Arc::new(Mutex::new(shingle.shared()));
    let (inspected, inspected_camp, inspected_stones) =
        (shared.clone(), camp.clone(), stones.clone());
    space.inspect_with(move |reference: &str| -> Result<Option<String>> {
        if reference == "beach/shingle" {
            let stones = inspected_stones
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            return Ok(Some(json(shingle::inspect(&stones))));
        }
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
        &shingle.shared(),
        clock(),
    ))?;
    drop(flock);
    let mut pace = Pace::new(Instant::now());
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
            if shingle.ask(intent.participant(), intent.payload(), &space.presence()) {
                // The stones' changes are said below, with the rest of what moved.
            } else if let Ok(asked) = serde_json::from_str::<campsite::Intent>(intent.payload()) {
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
        // A stone whose holder has left, or gone quiet, is let go where their hand was; then the stones move on.
        let presence = space.presence();
        shingle.sweep(space.present().as_deref(), &presence, Instant::now());
        let moved = shingle.update(Instant::now());
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
        let shared = shingle.shared();
        if let Some(moved) = moved {
            events.push(json(Stones {
                clock: clock(),
                stones: moved,
            }));
            *stones.lock().unwrap_or_else(PoisonError::into_inner) = shared.clone();
        }
        for event in events {
            pace.push(event);
        }
        for event in pace.due(Instant::now()) {
            space.announce(event, state(&flock, &campsite, &shared, clock()))?;
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
            let (id, reference, description, at) = shingle.describe();
            objects.push(
                Object::new(id, description)
                    .with_reference(reference)
                    .at(at.to_array()),
            );
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
