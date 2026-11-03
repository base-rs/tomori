// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The instructions Tomori gives a space's agent, in layers, each its own instruction entry of the agent's session. See `design/spaces.md`.

use protocol::AgentInstructions;

use crate::environments;

/// The entry of Tomori's own layer.
pub const TOMORI_KEY: &str = "tomori";
/// The entry of the space's environment's layer.
pub const ENVIRONMENT_KEY: &str = "tomori-environment";
/// The entry of the space's own layer.
pub const SPACE_KEY: &str = "tomori-space";
/// Every entry Tomori sets, which are removed when a space is forgotten.
pub const KEYS: [&str; 3] = [TOMORI_KEY, ENVIRONMENT_KEY, SPACE_KEY];

/// Tomori's own layer, the same in every space.
pub const TOMORI: &str = "You are Tomori, the coordinator of a shared spatial workspace. People talk to you aloud from headsets, and your replies are spoken. Reply briefly, in one to three short paragraphs of plain speech, without Markdown, tables or code unless someone asks to see them. Each message begins with the speaker's name. Delegate any non-trivial work, such as reading code, running commands or investigating, to sub-agents; say briefly what you have started, and report the findings when they return.";

/// The layers in effect for a space in `environment` whose own instructions are `space`, as participants see them.
pub fn layers(environment: &str, space: &str, error: Option<String>) -> AgentInstructions {
    AgentInstructions {
        tomori: TOMORI.to_owned(),
        environment: environments::instructions(environment).to_owned(),
        space: space.to_owned(),
        error,
    }
}

/// Each entry and its value, or `None` to remove an empty layer, so that the agent is given exactly the layers shown.
pub fn entries(layers: &AgentInstructions) -> Vec<(String, Option<String>)> {
    [
        (TOMORI_KEY, &layers.tomori),
        (ENVIRONMENT_KEY, &layers.environment),
        (SPACE_KEY, &layers.space),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), entry(value)))
    .collect()
}

/// The value of a layer's entry: `None`, removing it, when the layer is empty.
pub fn entry(text: &str) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layer_is_its_own_entry_and_an_empty_one_is_removed() {
        for key in KEYS {
            assert!(backend::valid_instruction_key(key), "{key}");
        }
        let shown = layers(environments::SKY, "Mind the docs.", None);
        assert_eq!(shown.tomori, TOMORI);
        // Sky contributes nothing until environments do.
        assert_eq!(shown.environment, "");
        assert_eq!(
            entries(&shown),
            [
                (TOMORI_KEY.to_owned(), Some(TOMORI.to_owned())),
                (ENVIRONMENT_KEY.to_owned(), None),
                (SPACE_KEY.to_owned(), Some("Mind the docs.".to_owned())),
            ]
        );
        assert_eq!(entry("  \n"), None);
    }
}
