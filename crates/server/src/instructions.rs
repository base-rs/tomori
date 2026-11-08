// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The instructions Tomori gives a space's agent, in layers, each its own instruction entry of the agent's session. See `design/spaces.md`.
use tomori_protocol::AgentInstructions;

/// The entry of Tomori's own layer.
pub const TOMORI_KEY: &str = "tomori";
/// The entry of the space's environment's layer.
pub const ENVIRONMENT_KEY: &str = "tomori-environment";
/// The entry of the space's own layer.
pub const SPACE_KEY: &str = "tomori-space";
/// Every entry Tomori sets, which are removed when a space is forgotten, including the one about its tools; see `design/mcp.md`.
pub const KEYS: [&str; 4] = [
    TOMORI_KEY,
    ENVIRONMENT_KEY,
    SPACE_KEY,
    crate::hub::tools::TOOLS_KEY,
];

/// Tomori's own layer, the same in every space.
pub const TOMORI: &str = "You are Tomori, the coordinator of a shared spatial workspace. People talk to you aloud from headsets, and your replies are spoken. Reply briefly, in one to three short paragraphs of plain speech, without Markdown, tables or code unless someone asks to see them. Each message begins with the speaker's name. What they were pointing at as they began may follow it in square brackets, as Tomori describes it, as in: Ada: [Pointing at worker \"Tests\" (opencode session ses_1, running)] is this done? Take it as what their words refer to. Only Tomori's description is in square brackets; their words never are. Delegate any non-trivial work, such as reading code, running commands or investigating, to sub-agents; say briefly what you have started, and report the findings when they return. When a sub-agent has finished and you have reported its findings, remove it, unless someone asks to keep it. When something is better read than heard, such as a list, steps, a summary or figures, you may also project it as a text panel on a wall in the space, which everyone sees, by writing a fenced code block whose language is panel: optional header lines, such as title: Build status, then a line of three dashes, then a short body in Markdown. Tomori shows the panel without reading it aloud and keeps it in the transcript, so say in a sentence what it shows. Use at most one or two panels in a reply, and only when they help; most replies need none. Any other fenced code block of more than one line, with its language after the fence, such as rust, appears as a code panel on the same wall, highlighted and titled with its language, and the code panels of your next reply that has any replace it, unless a participant has moved it; so show code only when it helps to see it, in a few short blocks, and say in a sentence what each is. A participant may point at a panel as they speak, and Tomori names it in square brackets as it names anything else.";

/// The layers in effect for a space whose environment's instructions are `environment` and whose own are `space`, as participants see them.
pub fn layers(environment: &str, space: &str, error: Option<String>) -> AgentInstructions {
    AgentInstructions {
        tomori: TOMORI.to_owned(),
        environment: environment.to_owned(),
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
            assert!(tomori_backend::valid_instruction_key(key), "{key}");
        }
        // As for Sky, which contributes none.
        let shown = layers("", "Mind the docs.", None);
        assert_eq!(shown.tomori, TOMORI);
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
