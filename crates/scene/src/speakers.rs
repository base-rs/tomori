// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who said what, and in which colour: the agent in cyan, each person in their own colour, as their swarm is drawn,
//! the same on every headset, and anyone the client cannot name in grey. See `design/spaces.md` and `design/people.md`.
use std::collections::HashMap;

use tomori_protocol::{Participant, SpaceState};

use crate::ui_text::Rgba;

/// The agent's colour, the panel's header cyan, close to the swarm's.
pub const AGENT: Rgba = [0.12, 0.85, 1.0, 1.0];
/// Someone the client cannot name or colour, such as text typed in the agent's own terminal.
pub const UNKNOWN: Rgba = [0.62, 0.66, 0.72, 1.0];
/// The colour a person of `hue` is shown in, in text and in swatches: the colour their swarm is drawn in
/// (`tomori_protocol::hue`), which no state and neither of the agent's colours is.
pub fn colour(hue: u16) -> Rgba {
    let [red, green, blue] = tomori_protocol::hue::rgb(hue);
    [red, green, blue, 1.0]
}

/// Who a message or queue entry is from, as shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Speaker {
    /// "You" for this participant, the agent's "Agent", or a display name.
    pub name: String,
    pub colour: Rgba,
    /// The public handle, when known.
    pub handle: Option<u32>,
}

impl Speaker {
    pub fn agent() -> Self {
        Self {
            name: "Agent".into(),
            colour: AGENT,
            handle: None,
        }
    }

    pub fn someone() -> Self {
        Self {
            name: "Someone".into(),
            colour: UNKNOWN,
            handle: None,
        }
    }

    /// The person with `handle` and `name`, or "You" when `handle` is `you`, in the colour of `hue`.
    pub fn person(handle: u32, name: &str, you: Option<u32>, hue: u16) -> Self {
        Self {
            name: if Some(handle) == you {
                "You".into()
            } else {
                name.into()
            },
            colour: colour(hue),
            handle: Some(handle),
        }
    }
}

/// The names the client has seen with their handles: the space's participants, and the speakers of queue entries it has seen, who may have left since; and the colour of each participant in the space now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Known {
    names: HashMap<String, u32>,
    /// Each participant's colour, by their number, in the space's participant order, as every client works it out.
    hues: Vec<(u32, u16)>,
    resolved: Vec<u16>,
}

impl Known {
    /// Learn the names in `space`, and the colour of each of its participants.
    pub fn learn(&mut self, space: &SpaceState) {
        for entry in &space.queue {
            self.names.insert(entry.speaker.clone(), entry.source);
        }
        for Participant { name, source, .. } in &space.participants {
            self.names.insert(name.clone(), *source);
        }
        tomori_protocol::hue::resolve(
            space
                .participants
                .iter()
                .map(|participant| (participant.name.as_str(), None)),
            &mut self.resolved,
        );
        self.hues.clear();
        self.hues.extend(
            space
                .participants
                .iter()
                .map(|participant| participant.source)
                .zip(self.resolved.iter().copied()),
        );
    }

    pub fn clear(&mut self) {
        self.names.clear();
        self.hues.clear();
    }

    /// Each participant's colour, by their number, as last learnt.
    pub fn hues(&self) -> &[(u32, u16)] {
        &self.hues
    }

    /// The colour of the person with `handle` and `name`: a participant's, or else, for someone who has left, their
    /// name's.
    pub fn hue(&self, handle: u32, name: &str) -> u16 {
        self.hues
            .iter()
            .find(|(source, _)| *source == handle)
            .map_or_else(|| tomori_protocol::hue::default(name), |(_, hue)| *hue)
    }

    /// The handle of `name`: a participant's now, or else one seen before.
    pub fn handle(&self, space: &SpaceState, name: &str) -> Option<u32> {
        space
            .participants
            .iter()
            .find(|participant| participant.name == name)
            .map(|participant| participant.source)
            .or_else(|| self.names.get(name).copied())
    }
}

/// The speaker of a user message's `text` in `space`, and its text without the attribution prefix the server adds ("Ada: "), when attribution is on. A name the client knows has its colour; another is grey; without a prefix the speaker is "Someone".
pub fn of_user_message<'a>(
    text: &'a str,
    space: &SpaceState,
    known: &Known,
    you: Option<u32>,
) -> (Speaker, &'a str) {
    if space.attribution
        && let Some((name, rest)) = text.split_once(": ")
        && !name.is_empty()
        && name.chars().count() <= 40
        && !name.contains('\n')
    {
        let speaker = match known.handle(space, name) {
            Some(handle) => Speaker::person(handle, name, you, known.hue(handle, name)),
            None => Speaker {
                name: name.into(),
                colour: UNKNOWN,
                handle: None,
            },
        };
        return (speaker, rest);
    }
    (Speaker::someone(), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::{QueueEntry, QueueState};

    fn space() -> SpaceState {
        SpaceState {
            attribution: true,
            participants: vec![
                Participant {
                    name: "Ada".into(),
                    source: 1,
                    engagement: None,
                },
                Participant {
                    name: "Bob".into(),
                    source: 2,
                    engagement: None,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn people_are_in_their_own_colours_never_the_agent_s() {
        let mut known = Known::default();
        let mut space = space();
        known.learn(&space);
        let ada = tomori_protocol::hue::default("Ada");
        let bob = tomori_protocol::hue::default("Bob");
        assert_eq!(known.hues(), [(1, ada), (2, bob)]);
        assert_ne!(colour(ada), colour(bob));
        for hue in [ada, bob] {
            assert_ne!(colour(hue), AGENT);
            assert_ne!(colour(hue), UNKNOWN);
        }
        // A second Ada is moved aside from the first, alike on every headset.
        space.participants.push(Participant {
            name: "Ada".into(),
            source: 3,
            engagement: None,
        });
        known.learn(&space);
        assert_eq!(known.hue(1, "Ada"), ada);
        assert_ne!(known.hue(3, "Ada"), ada);
        // Someone no longer here is in their name's colour.
        assert_eq!(known.hue(9, "Ada"), ada);
    }

    #[test]
    fn the_attribution_prefix_names_the_speaker() {
        let space = space();
        let known = Known::default();
        let (speaker, text) = of_user_message("Bob: run the tests", &space, &known, Some(1));
        let bob = tomori_protocol::hue::default("Bob");
        assert_eq!(speaker, Speaker::person(2, "Bob", Some(1), bob));
        assert_eq!(speaker.colour, colour(bob));
        assert_eq!(text, "run the tests");
        // This participant is "You", in their own colour, as the others see them.
        let (speaker, _) = of_user_message("Ada: and lint it", &space, &known, Some(1));
        assert_eq!(speaker.name, "You");
        assert_eq!(speaker.colour, colour(tomori_protocol::hue::default("Ada")));
        // An unknown name is grey, and no prefix is "Someone".
        let (speaker, text) = of_user_message("Eve: hello", &space, &known, Some(1));
        assert_eq!(
            (speaker.name.as_str(), speaker.colour, text),
            ("Eve", UNKNOWN, "hello")
        );
        let (speaker, text) = of_user_message("typed in the terminal", &space, &known, Some(1));
        assert_eq!(
            (speaker, text),
            (Speaker::someone(), "typed in the terminal")
        );
        // Without attribution, the text is not taken apart.
        let plain = SpaceState {
            attribution: false,
            ..space.clone()
        };
        let (speaker, text) = of_user_message("Bob: hi", &plain, &known, Some(1));
        assert_eq!((speaker, text), (Speaker::someone(), "Bob: hi"));
    }

    #[test]
    fn a_speaker_who_left_is_still_known_from_the_queue() {
        let mut space = space();
        space.queue.push(QueueEntry {
            id: "q".into(),
            source: 5,
            speaker: "Cy".into(),
            text: "deploy".into(),
            state: QueueState::Delivered,
        });
        let mut known = Known::default();
        known.learn(&space);
        space.queue.clear();
        let (speaker, _) = of_user_message("Cy: deploy", &space, &known, Some(1));
        assert_eq!(
            speaker,
            Speaker::person(5, "Cy", Some(1), tomori_protocol::hue::default("Cy"))
        );
        known.clear();
        assert_eq!(known.handle(&space, "Cy"), None);
        assert!(known.hues().is_empty());
    }
}
