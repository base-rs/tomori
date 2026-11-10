// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Each panel's faces, laid out once for what the panel shows and kept across frames: its body's Markdown parsed and
//! wrapped, or its code laid out, at each size its face has had, how large that is and how far it scrolls, and the face
//! last drawn from it, shared with the renderer, which keeps its mesh for as long as it is handed the same face. A face
//! is drawn again only when what the panel shows, its size, its scroll or its frame changes; a frame alone is
//! recoloured, without laying anything out. See `design/rendering.md`.
use std::sync::Arc;

use tomori_protocol::Panel;

use super::{
    CODE_PADDING, CODE_PIXEL, CodeLayout, Fit, chart, code_columns, code_face, fitted, image,
    item_in, sized, text_face,
};
use crate::{
    board,
    canvas::Canvas,
    transcript::Line,
    ui_text::{CELL_HEIGHT, CELL_WIDTH},
};

/// How many sizes a panel's faces are kept at: in its slot, floating as it is carried, and one more, so that carrying
/// it to another wall's slot of another size and back lays nothing out again.
const SIZES: usize = 3;

/// What a panel's faces are laid out from, by which they are kept or laid out again. From a server that versions panels,
/// its version, the projection revision at which it last changed in anything but its live data and its age, and its data
/// seq, at which its live data last changed (see `design/projection.md`): a new version lays it out again, and a chart's
/// new data seq at the same version only draws its face again; with what this client changes of a panel itself, which no
/// version follows, its body and code, as a tick shown at once changes it and a code panel is sent short or whole as its
/// frame allows, its title, and its age as it shows. From an older server, which versions nothing, or counted each text
/// panel's changes but none its environment made, as a chart panel without a data seq shows, the panel compared whole,
/// its age as it shows, so that every field that shows on its face is followed.
#[derive(Clone, Debug)]
struct Key(Panel);

/// How a panel differs from the one its faces were laid out from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Same,
    /// Only its chart's data, so that its faces are drawn again, but its size and its text kept.
    Plot,
    /// What it shows, so that it is laid out again.
    Content,
}

impl Key {
    fn of(panel: &Panel) -> Self {
        Self(panel.clone())
    }

    fn change(&mut self, panel: &Panel) -> Change {
        // Its age, in seconds, is counted on for every panel at each patch, but shows only as minutes, hours or days:
        // an age that shows the same changes nothing, so that a patch lays out only the panels it changes.
        if self.0.age != panel.age && board::age(self.0.age) == board::age(panel.age) {
            self.0.age = panel.age;
        }
        // A server before versions counted each text panel's changes as its revision, read here as its version, but not
        // its environment's changes of it: a chart panel without a data seq, and its title, are followed as they show.
        let versioned =
            |panel: &Panel| panel.version != 0 && (panel.chart.is_none() || panel.data_seq != 0);
        if versioned(&self.0) && versioned(panel) {
            return if self.0.version != panel.version || self.0.id != panel.id {
                Change::Content
            } else if self.0.data_seq != panel.data_seq && panel.chart.is_some() {
                // Its summary, its body, moves on with its data.
                Change::Plot
            } else if self.0.body != panel.body
                || self.0.title != panel.title
                || self.0.code != panel.code
                || self.0.age != panel.age
            {
                Change::Content
            } else {
                Change::Same
            };
        }
        if self.0 == *panel {
            return Change::Same;
        }
        if self.0.chart.is_some() && panel.chart.is_some() {
            let mut plotted = self.0.clone();
            plotted.chart.clone_from(&panel.chart);
            if plotted == *panel {
                return Change::Plot;
            }
        }
        Change::Content
    }
}

/// A panel's faces, laid out from it at each size its face has had, the latest first.
#[derive(Clone, Debug)]
pub(super) struct Faces {
    key: Key,
    pub(super) sizes: Vec<Layout>,
    /// How far down it scrolls at most, in rows, as it is sized this frame, while it stands; `None` while it does not.
    pub(super) limit: Option<usize>,
}

impl Faces {
    pub(super) fn new(panel: &Panel) -> Self {
        Self {
            key: Key::of(panel),
            sizes: Vec::new(),
            limit: None,
        }
    }

    /// Follow `panel` as the projection has it now: its layouts are kept while it shows the same, its faces drawn again
    /// where only its chart's data changed, and all of it laid out again where what it shows changed.
    pub(super) fn follow(&mut self, panel: &Panel) {
        match self.key.change(panel) {
            Change::Same => return,
            Change::Plot => self
                .sizes
                .iter_mut()
                .for_each(|layout| layout.replot(panel)),
            Change::Content => self.sizes.clear(),
        }
        self.key = Key::of(panel);
    }

    /// Its layout sized as `fit` says, laid out now if it was not already, kept first among its [`SIZES`].
    pub(super) fn at(&mut self, panel: &Panel, fit: Fit) -> &mut Layout {
        match self.sizes.iter().position(|layout| layout.fit == fit) {
            Some(index) => self.sizes[..=index].rotate_right(1),
            None => {
                self.sizes.insert(0, Layout::new(panel, fit));
                self.sizes.truncate(SIZES);
            }
        }
        &mut self.sizes[0]
    }
}

/// A panel's face laid out at one size: its code, or its body's rows at the columns its face has, its columns and rows
/// of body, its size in metres, how far it scrolls at most, and the face last drawn from them.
#[derive(Clone, Debug)]
pub(super) struct Layout {
    pub(super) fit: Fit,
    pub(super) code: Option<CodeLayout>,
    /// A text panel's body as [`super::body_lines`] lays it out at its columns, as a chart's or an image's summary is,
    /// which is how far those scroll; nothing for a code panel.
    lines: Vec<Line>,
    cells: (usize, usize),
    pub(super) size: (f32, f32),
    pub(super) most_scroll: usize,
    face: Option<Face>,
}

/// A face drawn: the scroll and frame it was drawn with.
#[derive(Clone, Debug)]
struct Face {
    scroll: usize,
    border: [f32; 4],
    canvas: Arc<Canvas>,
}

#[cfg(test)]
thread_local! {
    /// How many layouts this thread has laid out, and how many faces it has drawn from them, so that tests see that
    /// a frame lays out and draws nothing unless something changed.
    pub(super) static COUNTS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Count one more layout, or face, for the tests.
#[cfg(test)]
fn count(layouts: usize, faces: usize) {
    COUNTS.with(|counts| {
        let (laid, drawn) = counts.get();
        counts.set((laid + layouts, drawn + faces));
    });
}

#[cfg(not(test))]
fn count(_: usize, _: usize) {}

impl Layout {
    fn new(panel: &Panel, fit: Fit) -> Self {
        count(1, 0);
        let code = panel
            .code
            .as_ref()
            .map(|code| CodeLayout::new(panel, code, code_columns(fit)));
        let (cells, lines) = match (&code, fit) {
            (Some(_), Fit::Slot(cells)) => ((cells.columns, cells.body), Vec::new()),
            (Some(_), Fit::Text) => ((0, 0), Vec::new()),
            (None, _) => {
                let (columns, rows, lines) = fitted(panel, fit);
                ((columns, rows), lines)
            }
        };
        let (size, most_scroll) = match &code {
            Some(layout) => {
                let size = match fit {
                    Fit::Text => {
                        let width = layout.columns * CELL_WIDTH + 2 * CODE_PADDING.0;
                        let height = layout.face_rows(fit) * CELL_HEIGHT + 2 * CODE_PADDING.1;
                        (width as f32 * CODE_PIXEL, height as f32 * CODE_PIXEL)
                    }
                    Fit::Slot(_) => sized(cells.0, cells.1),
                };
                (size, layout.most_scroll(fit))
            }
            None => (sized(cells.0, cells.1), lines.len().saturating_sub(cells.1)),
        };
        Self {
            fit,
            code,
            lines,
            cells,
            size,
            most_scroll,
            face: None,
        }
    }

    /// Follow `panel`'s chart's live data, its points and its summary: its face is drawn again, and its summary's
    /// rows, which say how far it scrolls and which items the ray meets, are its summary's now, its size kept, so that
    /// nothing else is laid out again.
    fn replot(&mut self, panel: &Panel) {
        let (_, _, lines) = fitted(panel, self.fit);
        self.most_scroll = lines.len().saturating_sub(self.cells.1);
        self.lines = lines;
        self.face = None;
    }

    /// `panel`'s face as laid out here, scrolled `scroll` rows down and framed in `border`: the one last drawn, if it
    /// was drawn so; recoloured, if only its frame was another colour; or else drawn now, from what is laid out.
    pub(super) fn face(&mut self, panel: &Panel, scroll: usize, border: [f32; 4]) -> Arc<Canvas> {
        let face = match self.face.take() {
            Some(face) if face.scroll == scroll && face.border == border => face,
            Some(face) if face.scroll == scroll => {
                let mut canvas = Canvas::clone(&face.canvas);
                canvas.border = Some(border);
                Face {
                    border,
                    canvas: Arc::new(canvas),
                    ..face
                }
            }
            _ => Face {
                scroll,
                border,
                canvas: Arc::new(self.draw(panel, scroll, border)),
            },
        };
        let canvas = face.canvas.clone();
        self.face = Some(face);
        canvas
    }

    /// `panel`'s face drawn from what is laid out here, as [`super::scrolled_face`] draws it.
    fn draw(&self, panel: &Panel, scroll: usize, border: [f32; 4]) -> Canvas {
        count(0, 1);
        let border = Some(border);
        match (&self.code, &panel.chart, &panel.image) {
            (Some(layout), ..) => {
                let mut canvas = code_face(panel, layout, self.fit, scroll);
                canvas.border = border;
                canvas
            }
            (None, Some(chart), _) => chart::face(panel, chart, self.fit, border),
            (None, None, Some(image)) => image::face(panel, image, self.fit, border),
            (None, None, None) => text_face(panel, self.cells, &self.lines, scroll, border),
        }
    }

    /// The line of its panel's body that the list item where the ray meets it, at `point`, drawn with `model` and
    /// scrolled `scroll` rows down, begins on, as [`super::item_at`] finds it; none on a code panel.
    pub(super) fn item(&self, model: glam::Mat4, scroll: usize, point: glam::Vec3) -> Option<u32> {
        if self.code.is_some() {
            return None;
        }
        item_in(&self.lines, self.cells, model, scroll, point)
    }
}

#[cfg(test)]
mod tests;
