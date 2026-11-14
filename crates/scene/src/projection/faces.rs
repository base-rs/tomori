// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Each panel's faces, laid out once for what the panel shows and kept across frames: its body's Markdown parsed and
//! wrapped, or its code laid out, at each size its face has had, how large that is and how far it scrolls, and the face
//! last drawn from it, shared with the renderer, which keeps its mesh for as long as it is handed the same face. A face
//! is drawn again only when what the panel shows, its size, its scroll or its frame changes; a frame alone is
//! recoloured, without laying anything out. See `design/rendering.md`.
use std::sync::Arc;

use tomori_protocol::{Chart, Panel};

use super::{
    CODE_PADDING, CODE_PIXEL, CodeLayout, Fit, PIXEL, chart, code_columns, code_face, fitted,
    image, item_in,
    list::{Marked, RowMarks},
    sized, text_face,
};
use crate::{
    board,
    canvas::{Canvas, Kind},
    chart::{PlotArea, readout::Readout},
    transcript::{BRIGHT, Line},
    ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Style},
};

/// How many sizes a panel's faces are kept at: in its slot, floating as it is carried, and one more, so that carrying
/// it to another wall's slot of another size and back lays nothing out again.
const SIZES: usize = 3;

/// What a panel's faces are laid out from, by which they are kept or laid out again. From a server that versions panels,
/// its version, the projection revision at which it last changed in anything but its live data and its age, and its data
/// seq, at which its live data last changed (see `design/projection.md`): a new version lays it out again, and a chart's
/// new data seq at the same version only draws its face again; with what this client changes of a panel itself, which no
/// version follows, its body and code, as a tick shown at once changes it and a code panel is sent short or whole as its
/// frame allows, whether it has its chart, as a chart panel is sent as its summary where its frame cannot hold it, its
/// title, and its age as it shows. From an older server, which versions nothing, or counted each text
/// panel's changes but none its environment made, as a chart panel without a data seq shows, the panel compared whole,
/// its age as it shows, so that every field that shows on its face is followed.
#[derive(Clone, Debug)]
struct Key(Panel);

/// The most panels that show something already that one frame of a paced projector lays out again, or draws again for
/// their live data, for what the server changed of them: one whose content changed at a new version, one whose age
/// shows otherwise, and a chart whose data moved on. The rest keep the faces they have until a frame after, each in
/// turn, so that an environment's panels changing together, each up to 0.7 ms to lay out on a desktop and several
/// times that on a headset, cost each frame what two do: 32 within 16 frames, and 128 near enough to read within 64,
/// those drawn far taking no turn. What this client changed of a
/// panel itself, a tick shown at once, and a panel sized anew are laid out in their frame. See `design/rendering.md`.
pub(super) const LAYOUTS: usize = 2;

/// The most panels that show nothing yet that one frame of a paced projector lays out, besides [`LAYOUTS`]: panels
/// that arrive, as a space is entered or an environment puts up a page of its 128, the nearest first, each standing from
/// the frame it is laid out in. Four times as many as are laid out again, since nothing is seen of a panel until then, and waiting costs more
/// than a frame a little longer as a space appears: a headset's 90 panels stand within 12 frames, a sixth of a second,
/// and the 900 a space may show a client that takes them all within 113, about a second and a half.
pub(super) const ARRIVALS: usize = 8;

/// The angle, in radians, a row of a panel's text subtends from the viewer below which a paced projector draws its face
/// in a wall's slot far: its title alone, [`FAR_SCALE`] times as large, rather than text that could not be resolved
/// there, and lays nothing of it out. 0.09°, about 2.2 of a Quest 3's pixels at the centre of its lenses, about 25 to
/// a degree, 0.04° each, and fewer away from it, for a row of twelve font pixels, so that its glyphs are a blur of a
/// pixel or two whichever is drawn; a standard slot's
/// text is drawn far beyond about 14.5 m, a wall's of 1.22 times the scale beyond about 17.7 m and of 2.26 times
/// beyond about 33 m, and a code panel's, of a smaller font pixel, beyond about 10 m. What the panel shows changing
/// while it is far draws nothing until it comes near. Until November 2026 it was a quarter of a degree, beyond about
/// 5.2 m for a standard slot, which drew text still read at an ordinary distance as titles. See `design/rendering.md`.
pub(super) const FAR: f32 = 0.09 * std::f32::consts::PI / 180.0;

/// The angle above which a face drawn far is drawn whole again: a fifth more than [`FAR`], so that a head swaying at
/// the threshold does not switch it back and forth.
pub(super) const NEAR: f32 = 1.2 * FAR;

/// How many times as large as its text a far face draws its title, so that it can still be read from three times as
/// far as its text could.
pub(super) const FAR_SCALE: f32 = 3.0;

/// The most panels drawn far that one frame of a paced projector lays out to be drawn whole again as they come near,
/// the nearest first, as many as it lays out again for what the server changed ([`LAYOUTS`]): the rest stay far until a
/// frame after, so that walking up to a wall never lays out all of its panels in one frame.
pub(super) const NEARING: usize = LAYOUTS;

/// How a panel differs from the one its faces were laid out from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Same,
    /// Only its chart's data, so that its faces are drawn again, but its size and its text kept.
    Plot,
    /// Only its list's rows' tones and values, as the server changed them, so that its lines are kept, their marks
    /// changed, and its faces drawn again, in its turn.
    Restyle,
    /// What it shows, as the server changed it, so that it is laid out again, in its turn.
    Content,
    /// What it shows, as this client changed it itself, or as a server that versions nothing sent it, so that it is laid
    /// out again in this frame.
    Own,
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
            return if super::list::restyled(&self.0, panel) {
                Change::Restyle
            } else if self.0.version != panel.version
                || self.0.id != panel.id
                || self.0.chart.is_some() != panel.chart.is_some()
            {
                Change::Content
            } else if self.0.data_seq != panel.data_seq && panel.chart.is_some() {
                // Its summary, its body, moves on with its data.
                Change::Plot
            } else if self.0.body != panel.body
                || self.0.title != panel.title
                || self.0.code != panel.code
            {
                // At the version it had, only this client changed it: a tick shown at once, or its code short or whole.
                Change::Own
            } else if self.0.age != panel.age {
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
        Change::Own
    }
}

/// A panel's faces, laid out from it at each size its face has had, the latest first.
#[derive(Clone, Debug)]
pub(super) struct Faces {
    key: Key,
    pub(super) sizes: Vec<Layout>,
    /// How far down it scrolls at most, in rows, as it is sized this frame, while it stands; `None` while it does not.
    pub(super) limit: Option<usize>,
    /// Whether the projection's panel has changed since its faces were laid out, its turn to be laid out again not
    /// yet come: it shows what it showed, and none of its items is pointed at, since its lines are no longer the
    /// panel's.
    behind: bool,
    /// For a list laid out again as it changed, the row that stood at the top of its face, by its id, and how many lines
    /// into it, by which its scroll is found again among its rows as they are now. See [`super::list::anchor`].
    pub(super) anchor: Option<(String, f32)>,
    /// Whether it is drawn far, its title alone, with nothing of it laid out. See [`FAR`].
    pub(super) far: bool,
    /// Its far face, as last drawn, kept while it is drawn far at the same size and frame and its title is the same.
    far_face: Option<FarFace>,
}

/// A far face drawn: the title, size and frame it was drawn with.
#[derive(Clone, Debug)]
struct FarFace {
    title: String,
    size: (f32, f32),
    border: [f32; 4],
    canvas: Arc<Canvas>,
}

impl Faces {
    pub(super) fn new(panel: &Panel) -> Self {
        Self {
            key: Key::of(panel),
            sizes: Vec::new(),
            limit: None,
            behind: false,
            anchor: None,
            far: false,
            far_face: None,
        }
    }

    /// Draw it far, or whole again, as a row of its text subtends `angle` from the viewer: far below [`FAR`], forgetting
    /// what was laid out of it; whole again above [`NEAR`], while `nearing` allows, taking one of them, so that it is laid
    /// out as it is next sized.
    pub(super) fn distance(&mut self, angle: f32, nearing: &mut usize) {
        if self.far {
            if angle > NEAR && *nearing > 0 {
                self.far = false;
                self.far_face = None;
                *nearing -= 1;
            }
        } else if angle < FAR {
            self.far = true;
            self.behind = false;
            self.anchor = None;
            self.sizes.clear();
        }
    }

    /// Its far face, `size` and framed in `border`: its title alone, [`FAR_SCALE`] times as large as its text, as the
    /// panel was last followed, the one last drawn while those are the same.
    pub(super) fn far_face(&mut self, size: (f32, f32), border: [f32; 4]) -> Arc<Canvas> {
        let title = &self.key.0.title;
        if let Some(face) = self
            .far_face
            .as_ref()
            .filter(|face| face.size == size && face.border == border && face.title == *title)
        {
            return face.canvas.clone();
        }
        count(0, 1);
        let canvas = Arc::new(far(title, size, border));
        self.far_face = Some(FarFace {
            title: title.clone(),
            size,
            border,
            canvas: canvas.clone(),
        });
        canvas
    }

    /// Follow `panel` as the projection has it now: its layouts are kept while it shows the same, its faces drawn again
    /// where only its chart's data changed, and all of it laid out again where what it shows changed. Where the server
    /// changed it and `turn` is false, as when this frame has laid out as many panels as it may, it is left as it was,
    /// showing what it showed, to be followed in a later frame; what this client changed itself is followed whatever
    /// `turn` is. Returns whether it took a turn: whether the server's change of it was followed. A list laid out again
    /// keeps, as its [`Faces::anchor`], the row at the top of its face, scrolled `scroll` rows down, so that its scroll
    /// follows that row wherever the list now puts it.
    pub(super) fn follow(&mut self, panel: &Panel, turn: bool, scroll: f32) -> bool {
        let change = self.key.change(panel);
        // Drawn far, nothing of it is laid out, so following it costs nothing, and takes no turn.
        if self.far {
            self.key = Key::of(panel);
            return false;
        }
        self.behind = matches!(change, Change::Plot | Change::Restyle | Change::Content) && !turn;
        if change == Change::Same || self.behind {
            return false;
        }
        if change == Change::Plot {
            self.sizes
                .iter_mut()
                .for_each(|layout| layout.replot(panel));
        } else if change == Change::Restyle {
            self.sizes.iter_mut().for_each(|layout| {
                super::list::retone(&mut layout.lines, &panel.rows);
                layout.face = None;
            });
        } else {
            self.anchor = self
                .sizes
                .first()
                .and_then(|layout| super::list::anchor(&self.key.0.rows, &layout.lines, scroll));
            self.sizes.clear();
        }
        self.key = Key::of(panel);
        change != Change::Own
    }

    /// How far down to scroll it, laid out as `fit` says, so that the row its [`Faces::anchor`] names stands at the top
    /// of its face as it did, within `limit` rows: none where it has no anchor, or the list no longer has that row. Its
    /// anchor is taken.
    pub(super) fn anchored(&mut self, fit: Fit, limit: usize) -> Option<f32> {
        let anchor = self.anchor.take()?;
        self.at(fit);
        super::list::anchored(&self.key.0.rows, &self.sizes[0].lines, &anchor)
            .map(|scroll| scroll.min(limit as f32))
    }

    /// Its layout sized as `fit` says, of the panel as it was last followed, laid out now if it was not already, kept
    /// first among its [`SIZES`].
    pub(super) fn at(&mut self, fit: Fit) -> &mut Layout {
        match self.sizes.iter().position(|layout| layout.fit == fit) {
            Some(index) => self.sizes[..=index].rotate_right(1),
            None => {
                self.sizes.insert(0, Layout::new(&self.key.0, fit));
                self.sizes.truncate(SIZES);
            }
        }
        &mut self.sizes[0]
    }

    /// Its face sized as `fit` says, scrolled `scroll` rows down, its rows marked as this participant's `marks` say, and
    /// framed in `border`, of the panel as it was last followed, as [`Layout::face`] draws or keeps it. The marks are
    /// found among the rows of the panel it draws, so that a list waiting its turn to be laid out again, reordered,
    /// marks the row named, where it now stands in what is drawn, never another.
    pub(super) fn face(
        &mut self,
        fit: Fit,
        (scroll, marks): (usize, &RowMarks),
        border: [f32; 4],
    ) -> Arc<Canvas> {
        self.at(fit);
        let marked = marks.of(&self.key.0);
        self.sizes[0].face(&self.key.0, (scroll, marked), border)
    }

    /// The line of its body that the list item where the ray meets it begins on, sized as `fit` says, as
    /// [`Layout::item`] finds it; none while it is behind the projection's panel, whose lines it no longer shows.
    pub(super) fn item(
        &mut self,
        fit: Fit,
        model: glam::Mat4,
        scroll: usize,
        point: glam::Vec3,
    ) -> Option<u32> {
        if self.behind || self.far {
            return None;
        }
        self.at(fit).item(model, scroll, point)
    }

    /// The chip of its context where the ray meets its face, at `point`, sized as `fit` says, by its parameter, as
    /// [`Layout::chip`] finds it; none while it is behind the projection's panel, whose chips it no longer shows.
    pub(super) fn chip(
        &mut self,
        fit: Fit,
        model: glam::Mat4,
        point: glam::Vec3,
    ) -> Option<String> {
        if self.behind || self.far {
            return None;
        }
        self.at(fit).chip(model, point)
    }

    /// Its chart's readout where the ray meets its face, sized as `fit` says, as [`Layout::readout`] finds it, of the
    /// panel as it was last followed, whose face is drawn.
    pub(super) fn readout(
        &mut self,
        fit: Fit,
        model: glam::Mat4,
        point: glam::Vec3,
    ) -> Option<(f32, Arc<Canvas>, Readout)> {
        if self.far {
            return None;
        }
        self.at(fit);
        self.sizes[0].readout(&self.key.0, model, point)
    }

    /// Its chart's readout at `at`, a point's index or a share of the way between two, sized as `fit` says, linked to the
    /// readout of another chart pointed at, as [`Layout::readout_at`] draws it, of the panel as it was last followed.
    pub(super) fn linked(&mut self, fit: Fit, at: f32) -> Option<(Arc<Canvas>, Readout)> {
        if self.far {
            return None;
        }
        self.at(fit);
        let (face, readout) = self.sizes[0].readout_at(&self.key.0, at)?;
        Some((face, readout.linked()))
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
    /// Its context's chips as its face lays them out, which the ray finds the chip it meets among without laying them
    /// out again: none for a code, image or volume panel, which draws none.
    chips: super::chips::Laid,
}

/// `panel`'s context's chips as its face lays them out on `columns`, beside or below its metadata: a text, list or
/// chart panel's, those an environment publishes; none for a code, image or volume panel, which draws none.
fn chips_of(panel: &Panel, columns: usize) -> super::chips::Laid {
    if [
        panel.code.is_some(),
        panel.image.is_some(),
        panel.diagram.is_some(),
    ]
    .contains(&true)
    {
        return super::chips::Laid::default();
    }
    let after = match &panel.chart {
        Some(chart) => chart::metadata(&crate::chart::bounded(chart)),
        None => vec![crate::ui_text::Span::new(
            super::about(panel),
            crate::ui_text::Style::plain(crate::transcript::DIM),
        )],
    };
    super::chips::header(
        &panel.context,
        (&after, &after),
        columns,
        super::chips::today(),
    )
    .2
}

/// A far face of `title`, `width` by `height` metres, framed in `border`: the title in bold, wrapped onto as many rows as
/// fit, at [`FAR_SCALE`] times the size of a slot's text, its font pixel made exactly as wide as the face is, and its
/// rows and padding filling its height to within half a font pixel.
fn far(title: &str, (width, height): (f32, f32), border: [f32; 4]) -> Canvas {
    const MARGIN: (usize, usize) = (2, 2);
    let across = width / (PIXEL * FAR_SCALE) - (2 * MARGIN.0) as f32;
    let columns = ((across / CELL_WIDTH as f32) as usize).max(1);
    let pixel = width / (columns * CELL_WIDTH + 2 * MARGIN.0) as f32;
    let down = height / pixel;
    let rows = (((down - (2 * MARGIN.1) as f32) / CELL_HEIGHT as f32) as usize).max(1);
    let vertical = ((down - (rows * CELL_HEIGHT) as f32) / 2.0)
        .round()
        .max(0.0) as usize;
    let mut canvas = Canvas::new(columns, pixel);
    canvas.kind = Kind::PanelFace;
    canvas.padding = (MARGIN.0, vertical);
    canvas.background = [0.02, 0.035, 0.06, 0.94];
    canvas.border = Some(border);
    let lines = crate::ui_text::lines(title, columns, rows);
    let empty = rows - lines.len();
    canvas.rows = lines
        .iter()
        .map(|line| Row::text(line, Style::bold(BRIGHT), columns))
        .chain(std::iter::repeat_n(Row::new(Vec::new()), empty))
        .collect();
    canvas
}

/// A face drawn: the scroll, the rows marked and the frame it was drawn with, and, a chart's, where its plot is.
#[derive(Clone, Debug)]
struct Face {
    view: (usize, Marked),
    border: [f32; 4],
    canvas: Arc<Canvas>,
    plot: Option<PlotArea>,
}

#[cfg(test)]
thread_local! {
    /// How many layouts this thread has laid out, and how many faces it has drawn from them, so that tests see that
    /// a frame lays out and draws nothing unless something changed.
    pub(super) static COUNTS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Count one more layout, or face, for the tests, and the faces for the frame's timing.
fn count(layouts: usize, faces: usize) {
    crate::frame_timing::faces(faces);
    counted(layouts, faces);
}

#[cfg(test)]
fn counted(layouts: usize, faces: usize) {
    COUNTS.with(|counts| {
        let (laid, drawn) = counts.get();
        counts.set((laid + layouts, drawn + faces));
    });
}

#[cfg(not(test))]
fn counted(_: usize, _: usize) {}

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
            chips: chips_of(panel, cells.0),
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
        self.chips = chips_of(panel, self.cells.0);
        self.face = None;
    }

    /// `panel`'s face as laid out here, scrolled and its rows marked as `view` says, and framed in `border`: the one
    /// last drawn, if it was drawn so; recoloured, if only its frame was another colour; or else drawn now, from what is
    /// laid out.
    fn face(&mut self, panel: &Panel, view: (usize, Marked), border: [f32; 4]) -> Arc<Canvas> {
        let face = match self.face.take() {
            Some(face) if face.view == view && face.border == border => face,
            Some(face) if face.view == view => {
                crate::frame_timing::faces(1);
                let mut canvas = Canvas::clone(&face.canvas);
                canvas.border = Some(border);
                Face {
                    border,
                    canvas: Arc::new(canvas),
                    ..face
                }
            }
            _ => {
                let (mut canvas, plot) = self.draw(panel, view, border);
                canvas.kind = Kind::PanelFace;
                Face {
                    view,
                    border,
                    canvas: Arc::new(canvas),
                    plot,
                }
            }
        };
        let canvas = face.canvas.clone();
        self.face = Some(face);
        canvas
    }

    /// `panel`'s face drawn from what is laid out here, as [`super::scrolled_face`] draws it, and, a chart's, where its
    /// plot is.
    fn draw(
        &self,
        panel: &Panel,
        (scroll, marked): (usize, Marked),
        border: [f32; 4],
    ) -> (Canvas, Option<PlotArea>) {
        count(0, 1);
        let border = Some(border);
        // A volume's face is its heading and the plate its diagram stands in front of.
        if let Some(diagram) = &panel.diagram {
            return (super::volume::face(panel, diagram, self.fit, border), None);
        }
        match (&self.code, &panel.chart, &panel.image) {
            (Some(layout), ..) => {
                let mut canvas = code_face(panel, layout, self.fit, scroll);
                canvas.border = border;
                (canvas, None)
            }
            (None, Some(chart), _) => chart::face(panel, chart, self.fit, border),
            (None, None, Some(image)) => (image::face(panel, image, self.fit, border), None),
            (None, None, None) => (
                text_face(panel, self.cells, &self.lines, (scroll, marked), border),
                None,
            ),
        }
    }

    /// The chip of its panel's context where the ray meets its face, at `point`, drawn with `model`, by its parameter: on
    /// the row its face draws its chips on, among them as laid out once with it; none elsewhere, nor on a panel that
    /// draws none.
    fn chip(&self, model: glam::Mat4, point: glam::Vec3) -> Option<String> {
        if self.chips.spans.is_empty() {
            return None;
        }
        // As its face is laid out, whether or not it is drawn yet: its columns, its size and so its font pixel.
        let ((width, height), columns) = (self.size, self.cells.0);
        let pixel = width / (columns * CELL_WIDTH + 2 * super::PADDING.0) as f32;
        let local = model.inverse().transform_point3(point);
        let x = (local.x + width / 2.0) / pixel - super::PADDING.0 as f32;
        let y = (height / 2.0 - local.y) / pixel - super::PADDING.1 as f32;
        let top = (self.chips.row * CELL_HEIGHT) as f32;
        if !((top..top + CELL_HEIGHT as f32).contains(&y) && x >= 0.0) {
            return None;
        }
        super::chips::at(&self.chips, (x / CELL_WIDTH as f32) as usize).map(str::to_owned)
    }

    /// The readout of its chart where the ray meets its face, at `point`, drawn with `model`: where along the time axis
    /// that is, as a point's index or a share of the way between two, and the readout there, drawn from the face last
    /// drawn and where its plot is, without laying anything out; `None` off its plot, or for a panel of no chart.
    fn readout(
        &self,
        panel: &Panel,
        model: glam::Mat4,
        point: glam::Vec3,
    ) -> Option<(f32, Arc<Canvas>, Readout)> {
        // Every face pointed at is drawn, as the frame draws it first; only a chart's has a plot.
        let (canvas, plot, chart) = self.plotted(panel)?;
        let (width, height) = canvas.size();
        let local = model.inverse().transform_point3(point);
        // Across, to the nearest font pixel, so that a hand nearly still keeps the readout drawn.
        let inside = [
            ((local.x + width / 2.0) / canvas.pixel - canvas.padding.0 as f32).round(),
            (height / 2.0 - local.y) / canvas.pixel - canvas.padding.1 as f32,
        ];
        let at = plot.at(inside)?;
        Some((at, canvas.clone(), Self::read(canvas, plot, chart, at)))
    }

    /// The readout of its chart at `at`, a point's index or a share of the way between two, drawn from the face last
    /// drawn and where its plot is, without laying anything out; `None` for a panel of no chart or not yet drawn.
    fn readout_at(&self, panel: &Panel, at: f32) -> Option<(Arc<Canvas>, Readout)> {
        self.plotted(panel)
            .map(|(canvas, plot, chart)| (canvas.clone(), Self::read(canvas, plot, chart, at)))
    }

    /// The face last drawn, where its plot is and `panel`'s chart, if it is a chart's face.
    fn plotted<'a>(
        &'a self,
        panel: &'a Panel,
    ) -> Option<(&'a Arc<Canvas>, &'a PlotArea, &'a Chart)> {
        match (self.face.as_ref(), panel.chart.as_deref()) {
            (
                Some(Face {
                    canvas,
                    plot: Some(plot),
                    ..
                }),
                Some(chart),
            ) => Some((canvas, plot, chart)),
            _ => None,
        }
    }

    /// `chart`'s readout at `at` over `canvas`, its plot drawn at `plot`.
    fn read(canvas: &Canvas, plot: &PlotArea, chart: &Chart, at: f32) -> Readout {
        let bounds = (
            (canvas.columns * CELL_WIDTH) as f32,
            (canvas.rows.len() * CELL_HEIGHT) as f32,
        );
        Readout::new(chart, plot, at, bounds)
    }

    /// The line of its panel's body that the list item where the ray meets it, at `point`, drawn with `model` and
    /// scrolled `scroll` rows down, begins on, as [`super::item_at`] finds it; none on a code panel.
    fn item(&self, model: glam::Mat4, scroll: usize, point: glam::Vec3) -> Option<u32> {
        if self.code.is_some() {
            return None;
        }
        item_in(&self.lines, self.cells, model, scroll, point)
    }
}

#[cfg(test)]
mod tests;
