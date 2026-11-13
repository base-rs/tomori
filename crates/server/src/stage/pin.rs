// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Pin and freeze, by hand: a participant pulling an environment's panel off its wall to float pins it, the server copying it as
//! an instance with what it showed there fixed, every parameter it depends on that followed or that its wall fixed, the
//! wall keeping the panel itself, which goes on following; and freezing a panel fixes its time to the window it shows,
//! its chart kept as the content for that window, unfreezing letting it follow again. See `design/panel-context.md`.
use tomori_environment::TIME;
use tomori_protocol::chart::TimeAxis;
use tomori_protocol::context::ChipPress;

use super::context::Setting;
use super::published::Published;
use super::{Destination, Kept, Room, Stage, Target, Walls};

/// A copy made, by its identity and its environment's id for it.
pub type Copied = (String, String);

/// The settings a copy was pinned with, each a parameter's or time's.
pub type Pinned = Vec<(String, Setting)>;

/// Why a panel that shows no window of its own is not frozen.
pub const NO_WINDOW: &str = "Only a chart that shows a live window is frozen.";

/// What pulling `kept` off its wall in `room` fixes: each parameter it depends on that its side declares, and that it
/// does not fix itself, to the value its wall gives it or else the side's selection now; and its time, where its wall
/// sets it and it does not. A parameter it fixes itself it keeps; time it follows it goes on following.
fn pins(room: &Room, kept: &Kept) -> Vec<(String, Setting)> {
    let Some(published) = &kept.published else {
        return Vec::new();
    };
    let wall = kept.wall().and_then(|wall| room.contexts.get(wall));
    let inherited = |parameter: &str| wall.and_then(|wall| wall.get(parameter)).cloned();
    let mut pinned: Vec<(String, Setting)> = room
        .parameters
        .iter()
        .filter(|declared| {
            published.depends.contains(&declared.id) && !kept.context.contains_key(&declared.id)
        })
        .map(|declared| {
            let setting = inherited(&declared.id).unwrap_or_else(|| Setting::Fixed {
                value: declared.value.clone(),
            });
            (declared.id.clone(), setting)
        })
        .collect();
    if kept.timed()
        && !kept.context.contains_key(TIME)
        && let Some(time) = inherited(TIME)
    {
        pinned.push((TIME.to_owned(), time));
    }
    pinned
}

/// Why a panel that is not a copy of a chart shown here is not frozen or unfrozen.
pub const ONLY_COPIES: &str = "Only a copy of a chart is frozen or unfrozen: pin it first, pulling it off its wall or with pin_panel.";

/// Whether `published` is frozen and unfrozen: a copy, an instance, of a chart, whose panel was not withdrawn.
fn freezes(published: &Published) -> bool {
    published.of.is_some() && published.chart.is_some() && published.gone.is_none()
}

/// The window `chart`, `published`'s, shows, as absolute instants in milliseconds since the Unix epoch in UTC: a live
/// chart's, its last point the instant it was published, as many steps back as it has points; a fixed chart's, its own.
/// None for one whose time its chart does not say.
fn shown_window(
    chart: &tomori_protocol::chart::Chart,
    published: &Published,
) -> Option<(i64, i64)> {
    match chart.time? {
        TimeAxis::Live { step_ms } => {
            let span = i64::from(chart.points.max(2) - 1) * i64::from(step_ms);
            Some((published.at_ms - span, published.at_ms))
        }
        TimeAxis::Fixed { from_ms, to_ms } => Some((from_ms, to_ms)),
        _ => None,
    }
}

impl Stage {
    /// Pin `panel` as a participant pulls it to `target`, off the wall it is on: for one its environment published that
    /// depends on anything it does not fix itself, a copy of it, an instance with that fixed as [`pins`] says, made to
    /// be put at `target` in its place, the panel staying where it is, following; for an instance, those fixed on it
    /// itself, as it is Tomori's already, and moved itself. Returns the copy made, if any, or why it is refused, as at the
    /// most instances an environment shows. Nothing for a panel put on a wall, its own or another, whose context it then
    /// takes, one floating already, or one fixing all it depends on already.
    pub(crate) fn pin_pulled(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        target: &Target,
    ) -> Result<Option<Copied>, &'static str> {
        let Some(room) = self.rooms.get(walls.environment) else {
            return Ok(None);
        };
        let Some(kept) = room.panels.iter().find(|kept| kept.id == panel) else {
            return Ok(None);
        };
        // Only pulled off a wall to float: put on another wall, it takes that wall's context, as it is moved.
        if kept.wall().is_none() || !matches!(target, Target::Free(_)) {
            return Ok(None);
        }
        let pinned = pins(room, kept);
        if pinned.is_empty() {
            return Ok(None);
        }
        let instance = kept
            .published
            .as_ref()
            .is_some_and(|published| published.of.is_some());
        if instance {
            let kept = self
                .here_mut(walls.environment, panel)
                .expect("found above");
            kept.context.extend(pinned);
            return Ok(None);
        }
        // Made floating where the panel is put next, so that the copy goes there as the panel would have.
        let aside = Destination::Free(tomori_protocol::Pose::at([0.0, 1.5, 0.0]));
        self.copy_as(walls, panel, pinned, &aside, false).map(Some)
    }

    /// Pin `panel`, its environment's, in `walls`' environment, as the agent asks, to `to`: a copy of it, an instance with
    /// what it shows fixed, as [`pins`] says, as pulling it off its wall would make. Returns the copy, by its identity
    /// and its environment's id for it, and what was fixed; or why it is refused: a panel not here, not its
    /// environment's, one fixing all it depends on already, and whatever a copy is refused for.
    pub(crate) fn pin_copy(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        to: &Destination,
    ) -> Result<(Copied, Pinned), &'static str> {
        if self.view("", walls, 0).panel(panel).is_none() {
            return Err("That panel is not here.");
        }
        let pinned = self
            .rooms
            .get(walls.environment)
            .and_then(|room| {
                let kept = room.panels.iter().find(|kept| kept.id == panel)?;
                Some(pins(room, kept))
            })
            .ok_or("Only a panel its environment published is pinned.")?;
        if pinned.is_empty() {
            return Err("That panel follows nothing to pin: it fixes all it depends on already.");
        }
        let copied = self.copy_as(walls, panel, pinned.clone(), to, false)?;
        Ok((copied, pinned))
    }

    /// Put `panel` where a participant pulls it, at `target`, `size` as [`Stage::place_sized`] takes it, pinning it as
    /// [`Stage::pin_pulled`] says: the copy made goes there in its place, and is taken away again where it cannot.
    /// Returns whether anything changed, and the copy made, by its identity and its environment's id for it.
    pub(crate) fn place_pulled(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        target: Target,
        size: Option<f32>,
    ) -> Result<(bool, Option<Copied>), &'static str> {
        let Some((copy, instance)) = self.pin_pulled(walls, panel, &target)? else {
            return self
                .place_sized(walls, panel, target, size)
                .map(|changed| (changed, None));
        };
        match self.place_sized(walls, &copy, target, size) {
            Ok(_) => Ok((true, Some((copy, instance)))),
            Err(error) => {
                self.take(walls.environment, &copy);
                Err(error)
            }
        }
    }

    /// Freeze `panel`, a copy of a chart in `walls`' environment, as `frozen` says: its time fixed to the window its
    /// chart shows now, which it keeps as the content for that window, its own time setting before kept; or, unfrozen,
    /// that setting given back, or else none, so that it follows its wall's or its side's again. Returns whether that
    /// changed anything, or why it is refused: a panel not here, or not a copy of a chart whose panel is here
    /// ([`ONLY_COPIES`]), as pressing its time chip offers it only on one, or, frozen, one showing no window, or one wider
    /// than a setting holds.
    pub(crate) fn freeze(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        frozen: bool,
    ) -> Result<bool, &'static str> {
        if self.view("", walls, 0).panel(panel).is_none() {
            return Err("That panel is not here.");
        }
        let kept = self
            .here_mut(walls.environment, panel)
            .expect("a panel shown is kept");
        let shown = match &kept.published {
            Some(published) if freezes(published) => {
                let chart = published
                    .chart
                    .as_ref()
                    .expect("a copy frozen is of a chart");
                shown_window(chart, published)
            }
            _ => return Err(ONLY_COPIES),
        };
        let held = matches!(kept.context.get(TIME), Some(Setting::Window { .. }));
        if !frozen {
            if held {
                kept.context.remove(TIME);
                if let Some(thaw) = kept.thaw.take() {
                    kept.context.insert(TIME.to_owned(), thaw);
                }
            }
            return Ok(held);
        }
        if held {
            return Ok(false);
        }
        let (from_ms, to_ms) = shown.ok_or(NO_WINDOW)?;
        let window = Setting::Window { from_ms, to_ms }.checked(TIME, &[])?;
        kept.thaw = kept.context.insert(TIME.to_owned(), window);
        let chart = kept
            .published
            .as_mut()
            .and_then(|published| published.chart.as_mut())
            .expect("a copy frozen is of a chart");
        chart.time = Some(TimeAxis::Fixed { from_ms, to_ms });
        Ok(true)
    }

    /// What pressing the time chip of `kept`, `published` as it is, does, if anything: for an instance's chart, freezing it where its time is
    /// not fixed by its own window, or unfreezing it where it is.
    pub(super) fn chip_press(kept: &Kept, published: &Published) -> Option<ChipPress> {
        if !freezes(published) {
            return None;
        }
        Some(match kept.context.get(TIME) {
            Some(Setting::Window { .. }) => ChipPress::Unfreeze,
            _ => ChipPress::Freeze,
        })
    }
}

#[cfg(test)]
mod tests;
