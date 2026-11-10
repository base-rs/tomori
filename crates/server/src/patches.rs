// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Which message brings a client up to date with what its space projects: a patch from the projection it was last
//! sent, or the projection whole. See `design/projection.md`.
use tomori_protocol::{Projection, ProjectionPatch, ServerMessage};

/// The message that brings a client holding `projected`, the projection it was last sent, if it takes patches, up to
/// `projection`: a patch, where it holds the same space's in the same environment and the patch sends at most half of
/// what is shown whole, since a patch sending more saves little; otherwise `projection` whole, as on joining, after the
/// space changes environment, on the client's asking, and always to a client that takes no patches. A client that
/// applies data ops, `live`, is sent a live panel's data apart, and a panel that only moved as a place op carrying its
/// version; another is sent either panel whole.
pub(crate) fn message(
    projected: Option<&Projection>,
    projection: &Projection,
    live: bool,
) -> ServerMessage {
    if let Some(projected) = projected.filter(|projected| {
        projected.space == projection.space && projected.environment == projection.environment
    }) {
        let patch = if live {
            ProjectionPatch::between_live(projected, projection)
        } else {
            ProjectionPatch::between(projected, projection)
        };
        if patch.whole() * 2 <= projection.walls.len() + projection.panels.len() {
            return ServerMessage::ProjectionPatch {
                patch: Box::new(patch),
            };
        }
    }
    ServerMessage::Projection {
        projection: Box::new(projection.clone()),
    }
}

#[cfg(test)]
#[path = "patches/tests.rs"]
mod tests;
