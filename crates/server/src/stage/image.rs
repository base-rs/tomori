// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Image panels: what the agent shows with the `show_image` tool, an image with a title and a caption. Laid out, carried,
//! pinned and dismissed as every panel is, and never replaced by the agent's next code panels. See
//! `design/projection.md`.
use std::collections::BTreeSet;

use tomori_protocol::{Image, Panel};

use super::{Content, Stage, Walls, blocks};

/// The most characters of an image panel's caption, on one line, before the "…" that ends one cut.
pub const CAPTION_CHARS: usize = 240;

impl Content {
    /// The image panel showing `image`, titled `title` and captioned `caption`, both from the agent and so cleaned onto
    /// one line and cut, as a text panel's title is.
    pub fn image(title: &str, caption: &str, image: Image) -> Self {
        Self {
            title: tomori_protocol::text::one_line(title, blocks::TITLE_CHARS)
                .trim()
                .to_owned(),
            body: tomori_protocol::text::one_line(caption, CAPTION_CHARS)
                .trim()
                .to_owned(),
            meta: Vec::new(),
            code: None,
            slug: None,
            image: Some(image),
        }
    }
}

impl Stage {
    /// The images of every panel it keeps, shown or hidden, by their SHA-256.
    pub(crate) fn images(&self) -> BTreeSet<&str> {
        self.tomori
            .iter()
            .chain(self.rooms.values().flat_map(|room| &room.panels))
            .filter_map(|kept| kept.content.image.as_ref())
            .map(|image| image.hash.as_str())
            .collect()
    }

    /// Whether a panel it shows in `walls`' environment has the image `hash`, so that a client there may fetch it.
    pub(crate) fn shows_image(&self, walls: &Walls<'_>, hash: &str) -> bool {
        self.view("", walls, 0)
            .panels
            .iter()
            .any(|panel| panel.image.as_ref().is_some_and(|image| image.hash == hash))
    }
}

/// `panel`, as a client that shows no images is sent it: an image panel as a text panel saying that it is an image, and
/// how large, with its caption, so that an older headset shows something useful in its place.
pub fn without_image(mut panel: Panel) -> Panel {
    if let Some(image) = panel.image.take() {
        let note = format!(
            "*An image, {} × {} pixels: this headset needs an update to show it.*",
            image.width, image.height
        );
        panel.body = if panel.body.is_empty() {
            note
        } else {
            format!("{}\n\n{note}", panel.body)
        };
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Walls;
    use tomori_protocol::{Place, Pose, TOMORI_WALL};

    fn image(digit: char) -> Image {
        Image::new(digit.to_string().repeat(64), 1600, 900, 1234)
    }

    #[test]
    fn an_image_panel_is_cleaned_shown_kept_and_described() {
        let content = Content::image(
            "Layout\u{202e}\nrender",
            &format!("The session browser, {}", "very ".repeat(100)),
            image('a'),
        );
        assert_eq!(content.title, "Layout render");
        assert_eq!(content.body.chars().count(), CAPTION_CHARS + 1);
        assert!(content.body.ends_with('…'));
        let walls = Walls {
            environment: "sky",
            tomori: Pose::default(),
            declared: &[],
        };
        let mut stage = Stage::default();
        let id = stage.project(&walls, content, None, 0);
        let shown = stage.view("s", &walls, 60);
        let panel = shown.panel(&id).unwrap();
        assert_eq!(panel.image, Some(image('a')));
        assert!(matches!(&panel.place, Place::Wall { wall, slot: 0 } if wall == TOMORI_WALL));
        assert_eq!(stage.images(), BTreeSet::from(["a".repeat(64).as_str()]));
        assert!(stage.shows_image(&walls, &"a".repeat(64)));
        assert!(!stage.shows_image(&walls, &"b".repeat(64)));
        let described = stage.describe(&walls, &id, None, 60).unwrap();
        assert!(
            described.starts_with(
                "image panel \"Layout render\" (on the Tomori wall, projected a minute ago; 1600 × 900 pixels; caption: \"The session browser, very"
            ),
            "{described}"
        );
        // Kept across a restart, with its image.
        let kept = serde_json::to_value(&stage).unwrap();
        let read: Stage = serde_json::from_value(kept).unwrap();
        assert_eq!(read, stage);
        // A code panel replaces no image panel.
        let code = crate::stage::Content {
            title: "rust".into(),
            body: "fn main() {}\nfn other() {}".into(),
            meta: Vec::new(),
            code: Some(tomori_protocol::Code::new("rust", 2)),
            slug: None,
            image: None,
        };
        stage.project_reply(&walls, vec![code.clone()], None, 0);
        stage.project_reply(&walls, vec![code], None, 0);
        assert!(stage.shows_image(&walls, &"a".repeat(64)));
        // Dismissed, it is no longer shown, nor its image kept.
        stage.dismiss("sky", &id).unwrap();
        assert!(stage.images().is_empty());
        assert!(!stage.shows_image(&walls, &"a".repeat(64)));
        // Hidden in another environment, its image is kept, but not shown there.
        let farm = Walls {
            environment: "farm",
            tomori: Pose::default(),
            declared: &[],
        };
        let floating = stage.project(&farm, Content::image("", "", image('c')), None, 0);
        stage
            .place(
                &farm,
                &floating,
                crate::stage::Target::Free(Pose {
                    position: [0.0, 1.5, -1.0],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                }),
            )
            .unwrap();
        assert!(stage.shows_image(&farm, &"c".repeat(64)));
        // Without a caption, it is described by its size alone.
        let described = stage.describe(&farm, &floating, None, 0).unwrap();
        assert!(
            described.ends_with("(floating, projected just now; 1600 × 900 pixels)"),
            "{described}"
        );
        assert!(!stage.shows_image(&walls, &"c".repeat(64)));
        assert!(stage.images().contains("c".repeat(64).as_str()));
    }

    #[test]
    fn a_client_that_shows_no_images_is_sent_a_text_panel_saying_what_it_is() {
        let place = Place::Wall {
            wall: TOMORI_WALL.into(),
            slot: 0,
        };
        let captioned = Panel::new("panel-1", "Layout", "The session browser", place.clone())
            .with_image(image('a'));
        let text = without_image(captioned);
        assert_eq!(text.image, None);
        assert_eq!(
            text.body,
            "The session browser\n\n*An image, 1600 × 900 pixels: this headset needs an update to show it.*"
        );
        let bare =
            without_image(Panel::new("panel-2", "", "", place.clone()).with_image(image('b')));
        assert_eq!(
            bare.body,
            "*An image, 1600 × 900 pixels: this headset needs an update to show it.*"
        );
        // Other panels are sent as they are.
        let plain = Panel::new("panel-3", "Notes", "Text", place);
        assert_eq!(without_image(plain.clone()), plain);
    }
}
