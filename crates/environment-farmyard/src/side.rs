// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's server side: it owns the flock of each space bound to the farmyard, moves it on by the farmyard's
//! clock, the time since it started, takes participants' notices, and shares every change with the clients in the
//! space, with the clock, through the environment state channel. See `design/farmyard.md`.
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tomori_environment::{Action, Error, Object, Outcome, Parameter, Result, ServerSide, Space};

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
    let inspected = shared.clone();
    space.inspect_with(move |reference: &str| -> Result<Option<String>> {
        let flock = inspected.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(flock.inspect(reference, clock()).map(json))
    });
    let flock = shared.lock().unwrap_or_else(PoisonError::into_inner);
    space.share(json(flock.state(clock())))?;
    drop(flock);
    space.declare_actions(actions())?;
    let mut described: Option<Instant> = None;
    while !space.cancellation().is_cancelled() {
        let mut changes = Vec::new();
        let intent = space.next_intent(TICK);
        let mut flock = shared.lock().unwrap_or_else(PoisonError::into_inner);
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
            match serde_json::from_str::<Intent>(intent.payload()) {
                Ok(asked) => changes.extend(flock.ask(intent.participant(), asked, clock())),
                Err(_) => log::debug!("A farmyard intent could not be read."),
            }
        }
        changes.extend(flock.update(clock()));
        for (chicken, behaviour) in changes {
            let now = clock();
            space.announce(
                json(Event {
                    clock: now,
                    chicken,
                    behaviour,
                }),
                json(flock.state(now)),
            )?;
        }
        if described.is_none_or(|described| described.elapsed() >= DESCRIBE) {
            described = Some(Instant::now());
            let now = clock();
            // Each where she stands now, so that the agent can go to her when asked.
            space.describe(
                flock
                    .describe(now)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (id, reference, description))| {
                        Object::new(id, description)
                            .with_reference(reference)
                            .at(flock.position(index, now).to_array())
                            .offering(["peck"])
                    })
                    .collect(),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
