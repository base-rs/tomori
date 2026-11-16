// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Each person's colour, as a hue in whole degrees on a continuous wheel, at one saturation and lightness: by default
//! from their display name, moved aside within a space where it would be too near another's, or the one they chose,
//! which the server keeps with their token. The bands about the agent's cyan and violet, a worker's or the pill's amber
//! for waiting, and red for a failure are never a person's. Every client works a space's colours out alike from its
//! participants, in their order, so that everyone sees everyone in the same colour. See `design/people.md`.

/// Degrees about the wheel.
pub const DEGREES: u16 = 360;

/// The arcs of hue a person may be, each from its first degree up to, not including, its last: coral, between a
/// failure's red, about 6°, and waiting's amber, about 40°; yellow through green to teal, short of the agent's cyan,
/// about 186° to 200°; and magenta through pink, past the agent's violet, about 240° to 268°, and short of red. Each
/// keeps about 10° clear of the colour it borders, in the sRGB hue a colour at [`SATURATION`] and [`LIGHTNESS`] shows.
pub const ARCS: [(u16, u16); 3] = [(16, 28), (54, 176), (280, 356)];

/// How many degrees apart two people's default colours are at least, where the arcs allow: hues 30° apart at this
/// lightness, as yellow and lime, or green and teal, are told apart at a glance across a space, while closer ones read
/// as one colour in a swarm's glow. The arcs hold six or seven people so far apart, and more are spread as far apart as
/// they fit.
pub const APART: u16 = 30;

/// A person's colour's saturation and lightness, in sRGB's HSL: bright and pale enough to glow in an evening's haze, in
/// the farmyard and Sky, and to be read as text on the hand menu's and transcript's dark backings, as the warm coral
/// everyone was before people had colours of their own, which is hue 13 at these.
pub const SATURATION: f32 = 1.0;
pub const LIGHTNESS: f32 = 0.765;

/// The colours offered by name, in the hand menu's order and on the console, each clear of the reserved bands and,
/// but for teal and green's 40°, at least 30° from the next.
pub const SWATCHES: [(&str, u16); 7] = [
    ("coral", 20),
    ("yellow", 58),
    ("lime", 90),
    ("green", 125),
    ("teal", 165),
    ("magenta", 290),
    ("pink", 325),
];

/// Whether `hue` is one a person may be: a degree of the wheel within one of [`ARCS`].
pub fn clear(hue: u16) -> bool {
    ARCS.iter().any(|&(first, end)| (first..end).contains(&hue))
}

/// How many degrees apart `a` and `b` are, the short way round: from 0 to 180.
pub fn distance(a: u16, b: u16) -> u16 {
    let apart = a.abs_diff(b) % DEGREES;
    apart.min(DEGREES - apart)
}

/// FNV-1a, 32 bits, of `bytes`: small, stable and the same on every platform, so that a name has one colour on every
/// client of every build.
fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// The colour a person named `name` is by default, from a hash of their name, trimmed and in lower case, so that
/// "Ada" and " ada " are one colour: spread evenly over the arcs, as one continuous wheel with the reserved bands left
/// out.
pub fn default(name: &str) -> u16 {
    let total: u16 = ARCS.iter().map(|(first, end)| end - first).sum();
    let mut at = (fnv1a(name.trim().to_lowercase().as_bytes()) % u32::from(total)) as u16;
    ARCS.iter()
        .find_map(|&(first, end)| {
            let length = end - first;
            let within = at < length;
            if !within {
                at -= length;
            }
            within.then_some(first + at)
        })
        .unwrap_or(ARCS[0].0)
}

/// How far `hue` is from the nearest of `taken`, or 180 where nothing is.
fn clearance(hue: u16, taken: &[u16]) -> u16 {
    taken
        .iter()
        .map(|&other| distance(hue, other))
        .min()
        .unwrap_or(DEGREES / 2)
}

/// The colour a default `hue` takes beside `taken`: itself where it is at least [`APART`] from each, and otherwise the
/// nearest clear hue that is, looking a degree further either way in turn, higher first; or, where none is, the clear hue
/// farthest from them, the nearest to `hue` of those.
fn aside(hue: u16, taken: &[u16]) -> u16 {
    let mut best = (hue, clearance(hue, taken));
    if best.1 >= APART {
        return hue;
    }
    for step in 1..=DEGREES / 2 {
        for candidate in [(hue + step) % DEGREES, (hue + DEGREES - step) % DEGREES] {
            if !clear(candidate) {
                continue;
            }
            let apart = clearance(candidate, taken);
            if apart >= APART {
                return candidate;
            }
            if apart > best.1 {
                best = (candidate, apart);
            }
        }
    }
    best.0
}

/// The colour of each of `people`, by their names and the colours they chose, in the space's participant order, into
/// `colours`, cleared first, in the same order. A chosen colour always wins, so that each default is moved aside from
/// every chosen one; then each default is moved aside from those before it, where it is within [`APART`] of one, so
/// that a later participant moves and an earlier one keeps theirs, and nobody's colour changes where nothing is too
/// near. A chosen colour that is no degree of the wheel, as no server sends, counts as none.
pub fn resolve<'a, I>(people: I, colours: &mut Vec<u16>)
where
    I: IntoIterator<Item = (&'a str, Option<u16>)>,
    I::IntoIter: Clone,
{
    let people = people.into_iter();
    let chosen = |colour: Option<u16>| colour.filter(|&hue| hue < DEGREES);
    let mut taken: Vec<u16> = people
        .clone()
        .filter_map(|(_, colour)| chosen(colour))
        .collect();
    colours.clear();
    for (name, colour) in people {
        colours.push(chosen(colour).unwrap_or_else(|| {
            let hue = aside(default(name), &taken);
            taken.push(hue);
            hue
        }));
    }
}

/// The colour `hue` is drawn in, as linear RGB from 0 to 1: at [`SATURATION`] and [`LIGHTNESS`] in sRGB's HSL, decoded
/// to linear, as Tomori's colours are given.
pub fn rgb(hue: u16) -> [f32; 3] {
    let hue = f32::from(hue % DEGREES) / 60.0;
    let chroma = (1.0 - (2.0 * LIGHTNESS - 1.0).abs()) * SATURATION;
    let second = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let (red, green, blue) = match hue as u8 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let least = LIGHTNESS - chroma / 2.0;
    // Every channel is at least `least`, far above sRGB's linear toe, so each decodes by its curve alone.
    [red, green, blue].map(|value| (((value + least).min(1.0) + 0.055) / 1.055).powf(2.4))
}

/// A colour as a person asks for it, on the console or as the agent is told it: `default`, or nothing, for none, so
/// that their colour is their name's again; one of [`SWATCHES`] by name, in any case; or a hue in whole degrees, from
/// 0 to 359. Says why where it is none of those, or a hue no person may be.
pub fn parse(text: &str) -> Result<Option<u16>, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("default") {
        return Ok(None);
    }
    if let Some(&(_, hue)) = SWATCHES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
    {
        return Ok(Some(hue));
    }
    let hue = text
        .strip_suffix('°')
        .unwrap_or(text)
        .parse::<u16>()
        .map_err(|_| unknown())?;
    check(hue).map(Some)
}

/// `hue`, where a person may be it; or why not.
pub fn check(hue: u16) -> Result<u16, String> {
    if hue >= DEGREES {
        return Err(format!("A hue is from 0 to 359 degrees, not {hue}."));
    }
    if !clear(hue) {
        return Err(format!(
            "Hue {hue} is kept for the agent or for a state, such as waiting or a failure: choose 16 to 27, 54 to 175 or 280 to 355, or one of {}.",
            names()
        ));
    }
    Ok(hue)
}

fn unknown() -> String {
    format!(
        "A colour is one of {}, a hue in degrees, or default.",
        names()
    )
}

fn names() -> String {
    SWATCHES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The name of `hue` where it is one of [`SWATCHES`], or else its degrees, as `200°`.
pub fn name(hue: u16) -> String {
    SWATCHES
        .iter()
        .find(|(_, swatch)| *swatch == hue)
        .map_or_else(|| format!("{hue}°"), |(name, _)| (*name).to_owned())
}

#[cfg(test)]
mod tests;
