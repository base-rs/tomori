// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's text, below layout: the bundled fonts in [`fonts`], which characters they cover, and the signed-distance-field glyph atlas in [`atlas`] that panels and labels are drawn from. It has no GPU code: a renderer uploads [`atlas::Atlas::pixels`] into a single-channel texture and samples it, as `tomori-render` does, and layout, such as `tomori-scene`'s `ui_text`, places glyphs from their metrics.

mod distance;

pub mod atlas;
pub mod fonts;
