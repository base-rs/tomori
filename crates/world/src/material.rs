// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Materials: how what an item draws is shaded, described once and without a GPU, so that each render path reads what
//! it can afford of it. See `design/rendering.md` and `docs/proposals/rendering-architecture.md`, section 8.
//!
//! Today's renderer reads a material's shading model, its base colour and emission, its alpha mode and its flags, and
//! draws it by the pipeline its shading model names, as before: `tomori-render`'s `materials` maps one to the other.
//! A stylised material's base colour texture and normal map, by digest, are laid on faces without texture coordinates
//! by its [`Projection`], in the tiers that sample them; its roughness and metalness are carried for the render paths
//! still to come, and nothing reads them yet.
use glam::{Vec3, Vec4};
use tomori_assets::Digest;

use crate::world::Key;

/// A material kept in a world, by its slot and that slot's generation, so that one removed is never mistaken for one
/// added in its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialId(pub(crate) Key);

/// How a material is shaded, which decides the pipeline that draws it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shading {
    /// Faceted, each face by its own normal, lit by its root's environment light and its own few lamps with the
    /// stylised falloff and wrap: the farmyard's models.
    #[default]
    Stylised,
    /// Lit by Tomori's own soft light from above, whatever the environment's: Tomori's cubes, among them the swarm,
    /// workers, rings and rays, and the others' swarms.
    Own,
    /// Its colour as it is, unshaded: panels, walls, signs, labels and plaques.
    Unlit,
    /// Drawn by a pipeline of its scenery's own, by the number the scenery gives it: the farmyard's ferns, Sky's
    /// platform and clouds.
    Custom(u32),
}

/// How a material covers what is behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alpha {
    /// Wholly, whatever its alpha.
    #[default]
    Opaque,
    /// Cut out where its alpha is below a half, as glTF's `MASK`.
    Mask,
    /// Blended over what is behind it by its alpha, as glTF's `BLEND`.
    Blend,
}

/// How a texture is laid on faces that have no texture coordinates, as the faceted models built in code have none: each
/// face's projected along the axis its normal is nearest, in the world, `metres` a tile across, so that faces meeting at
/// an edge meet in the texture too; and each piece's, as each instance of a model, or each chunk of one, is drawn, turned
/// by up to `turn` radians either way and shifted by a part of a tile of its own, so that copies do not repeat each
/// other, or neither, `turn` nothing, so that pieces that meet, as the chunks of one ground, meet in the texture too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    pub metres: f32,
    pub turn: f32,
}

impl Projection {
    /// A tile `metres` across, each piece's neither turned nor shifted.
    pub fn new(metres: f32) -> Self {
        Self { metres, turn: 0.0 }
    }

    /// This projection, each piece's turned by up to `turn` radians either way and shifted by a part of a tile.
    pub fn with_turn(self, turn: f32) -> Self {
        Self { turn, ..self }
    }
}

/// How something drawn is shaded: its shading model; its base colour, which multiplies the colours its vertices or
/// instances carry, as glTF's base colour factor does, and the texture it may be read from; its roughness, metalness
/// and normal map; the light it gives off; how it covers what is behind it; and whether it is hazed and lit by its
/// root's lights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub shading: Shading,
    /// Linear RGBA: white draws the colours of the vertices or instances as they are.
    pub base_color: Vec4,
    /// The base colour's texture, by the digest of its bytes in the content store.
    pub base_color_texture: Option<Digest>,
    /// The mean of the base colour's texture, linear RGB, as it is made: the colour a tier that does not sample the
    /// texture stands in for it with. Today's renderer divides the texture's colour by it, so that the texture varies the
    /// colours a model's vertices carry about themselves, and draws them as they are where it does not sample it.
    pub flat_color: Vec3,
    /// How its textures are laid on faces without texture coordinates, or none, for a model's own.
    pub projection: Option<Projection>,
    /// From 0, a mirror, to 1, wholly diffuse; not yet read.
    pub roughness: f32,
    /// From 0, a dielectric, to 1, a metal; not yet read.
    pub metalness: f32,
    /// A tangent-space normal map, by digest, its `x` across the texture and `y` down it.
    pub normal_texture: Option<Digest>,
    /// Light it gives off, linear RGB, as a lantern's glass does.
    pub emissive: Vec3,
    /// How much of its colour is light of its own, unshaded, from 0 to 1, as the others' swarms glow.
    pub glow: f32,
    pub alpha: Alpha,
    /// Whether it fades into its environment's haze by the shared haze, as everything drawn in the world does but what
    /// stands beyond the far plane.
    pub hazed: bool,
    /// Whether its root's lights light it, each instance by its own few, as [`crate::assign`] chooses them.
    pub lamps: bool,
}

impl Material {
    /// Shaded `shading`, white, wholly rough and not metal, untextured, giving off no light, opaque and hazed, and
    /// lit by its root's lights if it is [`Shading::Stylised`].
    pub fn new(shading: Shading) -> Self {
        Self {
            shading,
            base_color: Vec4::ONE,
            base_color_texture: None,
            flat_color: Vec3::ONE,
            projection: None,
            roughness: 1.0,
            metalness: 0.0,
            normal_texture: None,
            emissive: Vec3::ZERO,
            glow: 0.0,
            alpha: Alpha::Opaque,
            hazed: true,
            lamps: shading == Shading::Stylised,
        }
    }

    /// This material, of base colour `base_color`, linear RGBA.
    pub fn with_base_color(self, base_color: Vec4) -> Self {
        Self { base_color, ..self }
    }

    /// This material, its base colour read from the texture `digest` names.
    pub fn with_base_color_texture(self, digest: Digest) -> Self {
        Self {
            base_color_texture: Some(digest),
            ..self
        }
    }

    /// This material, its base colour texture's mean `flat_color`, linear RGB.
    pub fn with_flat_color(self, flat_color: Vec3) -> Self {
        Self { flat_color, ..self }
    }

    /// This material, its textures laid on faces without texture coordinates by `projection`.
    pub fn with_projection(self, projection: Projection) -> Self {
        Self {
            projection: Some(projection),
            ..self
        }
    }

    /// This material, `roughness` rough and `metalness` metal.
    pub fn with_surface(self, roughness: f32, metalness: f32) -> Self {
        Self {
            roughness,
            metalness,
            ..self
        }
    }

    /// This material, its normals bent by the normal map `digest` names.
    pub fn with_normal_texture(self, digest: Digest) -> Self {
        Self {
            normal_texture: Some(digest),
            ..self
        }
    }

    /// This material, giving off `emissive`.
    pub fn with_emissive(self, emissive: Vec3) -> Self {
        Self { emissive, ..self }
    }

    /// This material, `glow` of its colour its own light.
    pub fn with_glow(self, glow: f32) -> Self {
        Self { glow, ..self }
    }

    /// This material, covering what is behind it as `alpha` says.
    pub fn with_alpha(self, alpha: Alpha) -> Self {
        Self { alpha, ..self }
    }

    /// This material, lit by its root's lights or not.
    pub fn with_lamps(self, lamps: bool) -> Self {
        Self { lamps, ..self }
    }

    /// This material, hazed or not.
    pub fn with_haze(self, hazed: bool) -> Self {
        Self { hazed, ..self }
    }
}

/// A model's material, as its glTF or the code that built it says: stylised, of its base colour and emission, cut out
/// if it is masked. Its texture, if it has one, is the model's own, embedded rather than named by digest, and stays
/// with the model.
impl From<&tomori_assets::Material> for Material {
    fn from(material: &tomori_assets::Material) -> Self {
        Self::new(Shading::Stylised)
            .with_base_color(material.base_color)
            .with_emissive(material.emissive)
            .with_alpha(if material.masked {
                Alpha::Mask
            } else {
                Alpha::Opaque
            })
    }
}

#[cfg(test)]
mod tests;
