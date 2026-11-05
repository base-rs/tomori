// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who said what, and in which colour: the agent in cyan, each person in a colour from their public handle, the same on every headset, and anyone the client cannot name in grey. See `design/spaces.md`.

use std::collections::HashMap;

use tomori_protocol::{Participant, SpaceState};

use crate::ui_text::Rgba;

/// The agent's colour, the panel's header cyan, close to the swarm's.
pub const AGENT: Rgba = [0.12, 0.85, 1.0, 1.0];
/// Someone the client cannot name or colour, such as text typed in the agent's own terminal.
pub const UNKNOWN: Rgba = [0.62, 0.66, 0.72, 1.0];
/// People's colours: violet, green, pink, peach, periwinkle and lime, avoiding the agent's cyan, attention's amber and a problem's red.
pub const PALETTE: [Rgba; 6] = [
    [0.74, 0.56, 1.0, 1.0],
    [0.42, 0.9, 0.48, 1.0],
    [1.0, 0.52, 0.78, 1.0],
    [1.0, 0.74, 0.52, 1.0],
    [0.58, 0.68, 1.0, 1.0],
    [0.78, 0.95, 0.32, 1.0],
];

/// The colour of the participant with public handle `handle`, the same for everyone in the space. Handles are given across the server, so two people in a space may share one; a name is always shown with it.
pub fn colour(handle: u32) -> Rgba {
    PALETTE[handle as usize % PALETTE.len()]
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

    /// The person with `handle` and `name`, or "You" when `handle` is `you`.
    pub fn person(handle: u32, name: &str, you: Option<u32>) -> Self {
        Self {
            name: if Some(handle) == you {
                "You".into()
            } else {
                name.into()
            },
            colour: colour(handle),
            handle: Some(handle),
        }
    }
}

/// The names the client has seen with their handles: the space's participants, and the speakers of queue entries it has seen, who may have left since.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Known {
    names: HashMap<String, u32>,
}

impl Known {
    /// Learn the names in `space`.
    pub fn learn(&mut self, space: &SpaceState) {
        for entry in &space.queue {
            self.names.insert(entry.speaker.clone(), entry.source);
        }
        for Participant { name, source, .. } in &space.participants {
            self.names.insert(name.clone(), *source);
        }
    }

    pub fn clear(&mut self) {
        self.names.clear();
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
            Some(handle) => Speaker::person(handle, name, you),
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
    fn colours_come_from_handles_and_avoid_the_reserved_colours() {
        let colours: Vec<_> = (1..=6).map(colour).collect();
        for (index, colour) in colours.iter().enumerate() {
            assert!(!colours[index + 1..].contains(colour));
            assert_ne!(*colour, AGENT);
            assert_ne!(*colour, UNKNOWN);
        }
        // The same handle is the same colour on every headset.
        assert_eq!(colour(7), colour(7));
    }

    #[test]
    fn the_attribution_prefix_names_the_speaker() {
        let space = space();
        let known = Known::default();
        let (speaker, text) = of_user_message("Bob: run the tests", &space, &known, Some(1));
        assert_eq!(speaker, Speaker::person(2, "Bob", Some(1)));
        assert_eq!(speaker.colour, colour(2));
        assert_eq!(text, "run the tests");
        // This participant is "You", in their own colour, as the others see them.
        let (speaker, _) = of_user_message("Ada: and lint it", &space, &known, Some(1));
        assert_eq!(speaker.name, "You");
        assert_eq!(speaker.colour, colour(1));
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
        assert_eq!(speaker, Speaker::person(5, "Cy", Some(1)));
        known.clear();
        assert_eq!(known.handle(&space, "Cy"), None);
    }
}
