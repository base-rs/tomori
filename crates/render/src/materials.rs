// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The render world's materials drawn by today's pipelines: which pipeline draws each, by its shading model, and the
//! values it draws it with, so that what is submitted through the world is drawn exactly as it was before materials
//! were; and, in the tiers that sample them, the textures a stylised material lays on its faces. See
//! `design/rendering.md`.
use glam::Vec3;
use tomori_world::{Alpha, Digest, Material, Projection, Shading, Tier};

/// A pipeline of today's renderer, as a material's shading model chooses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pipeline {
    /// `tomori-assets`' models: stylised, lit by the environment's light and each piece's own lamps, cut out where
    /// masked.
    Models,
    /// Tomori's cubes, the swarm's pipeline: lit by their own soft light from above, each glowing by its material's
    /// glow.
    Cubes,
    /// The surfaces with text and shapes on them: panels, walls, signs, labels and plaques, unlit.
    Surfaces,
    /// A scenery's own, by the number it gives it.
    Custom(u32),
}

/// The pipeline that draws `material` today, or none if none draws it as it says. Every pipeline hazes what it draws by
/// the shared haze; only models, and a scenery's own, are lit by lamps; models are cut out but never blended; Tomori's
/// cubes are opaque.
pub fn pipeline(material: &Material) -> Option<Pipeline> {
    match material.shading {
        _ if !material.hazed => None,
        Shading::Stylised if material.alpha != Alpha::Blend => Some(Pipeline::Models),
        Shading::Own if !material.lamps && material.alpha == Alpha::Opaque => Some(Pipeline::Cubes),
        Shading::Unlit if !material.lamps => Some(Pipeline::Surfaces),
        Shading::Custom(pipeline) => Some(Pipeline::Custom(pipeline)),
        _ => None,
    }
}

/// What the model pipeline draws `material` with, as a model's own material says it: its base colour, its emission
/// and whether it is cut out; none if it does not draw it. Its texture is the model's own.
pub fn model(material: &Material) -> Option<tomori_assets::Material> {
    (pipeline(material)? == Pipeline::Models).then(|| {
        let mut model = tomori_assets::Material::new(String::new(), material.base_color)
            .with_emissive(material.emissive);
        model.name = None;
        model.masked = material.alpha == Alpha::Mask;
        model
    })
}

/// How much of a cube's colour is its own light, as Tomori's cubes draw `material`, from 0 to 1; none if they do not
/// draw it.
pub fn glow(material: &Material) -> Option<f32> {
    (pipeline(material)? == Pipeline::Cubes).then_some(material.glow.clamp(0.0, 1.0))
}

/// What the model pipeline lays on a material's faces: its base colour texture and its normal map, by digest, its base
/// colour's mean, which its texels are divided by, and how both are laid on faces without texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampled {
    pub color: Digest,
    pub normal: Option<Digest>,
    pub flat: Vec3,
    pub projection: Projection,
}

/// What the model pipeline samples of `material` in `tier`: in High and Ultra, its base colour texture and normal map,
/// laid on its faces by its projection; in Low, nothing, so that it is drawn by its faces' own colours, as before
/// textures were, at no cost of the GPU's; and nothing for a material the model pipeline does not draw, or without a
/// base colour texture and a projection.
pub fn sampled(material: &Material, tier: Tier) -> Option<Sampled> {
    if tier == Tier::Low || pipeline(material)? != Pipeline::Models {
        return None;
    }
    Some(Sampled {
        color: material.base_color_texture?,
        normal: material.normal_texture,
        flat: material.flat_color,
        projection: material.projection?,
    })
}

#[cfg(test)]
mod tests {
    use glam::{Vec3, Vec4};

    use super::*;

    #[test]
    fn high_and_ultra_sample_a_stylised_material_s_textures_and_low_its_faces_colours() {
        let (grain, bumps) = (Digest::of(b"grain"), Digest::of(b"bumps"));
        let earth = Material::new(Shading::Stylised)
            .with_base_color_texture(grain)
            .with_normal_texture(bumps)
            .with_flat_color(Vec3::new(0.25, 0.17, 0.1))
            .with_projection(Projection::new(2.0));
        let drawn = Sampled {
            color: grain,
            normal: Some(bumps),
            flat: Vec3::new(0.25, 0.17, 0.1),
            projection: Projection::new(2.0),
        };
        assert_eq!(sampled(&earth, Tier::High), Some(drawn));
        assert_eq!(sampled(&earth, Tier::Ultra), Some(drawn));
        assert_eq!(sampled(&earth, Tier::Low), None);
        // Without a normal map, its colour alone.
        let flat = Material {
            normal_texture: None,
            ..earth
        };
        assert_eq!(
            sampled(&flat, Tier::High).map(|sampled| sampled.normal),
            Some(None)
        );
        // Without a texture, or a projection, or drawn by another pipeline, nothing.
        for material in [
            Material {
                base_color_texture: None,
                ..earth
            },
            Material {
                projection: None,
                ..earth
            },
            Material {
                shading: Shading::Own,
                lamps: false,
                ..earth
            },
            earth.with_haze(false),
        ] {
            assert_eq!(sampled(&material, Tier::High), None, "{material:?}");
        }
    }

    #[test]
    fn each_shading_model_is_drawn_by_today_s_pipeline_for_it() {
        let drawn = |material: Material| pipeline(&material);
        assert_eq!(
            drawn(Material::new(Shading::Stylised)),
            Some(Pipeline::Models)
        );
        assert_eq!(
            drawn(Material::new(Shading::Stylised).with_alpha(Alpha::Mask)),
            Some(Pipeline::Models)
        );
        assert_eq!(drawn(Material::new(Shading::Own)), Some(Pipeline::Cubes));
        assert_eq!(
            drawn(Material::new(Shading::Unlit).with_alpha(Alpha::Blend)),
            Some(Pipeline::Surfaces)
        );
        assert_eq!(
            drawn(Material::new(Shading::Custom(4)).with_lamps(true)),
            Some(Pipeline::Custom(4))
        );
        // What no pipeline of today draws as it says.
        for material in [
            Material::new(Shading::Stylised).with_haze(false),
            Material::new(Shading::Custom(4)).with_haze(false),
            Material::new(Shading::Stylised).with_alpha(Alpha::Blend),
            Material::new(Shading::Own).with_lamps(true),
            Material::new(Shading::Own).with_alpha(Alpha::Mask),
            Material::new(Shading::Unlit).with_lamps(true),
        ] {
            assert_eq!(drawn(material), None, "{material:?}");
        }
    }

    #[test]
    fn a_model_s_material_through_the_world_is_drawn_with_what_it_was() {
        let mut leaf = tomori_assets::Material::new("leaf", Vec4::new(0.2, 0.6, 0.1, 1.0))
            .with_emissive(Vec3::new(0.0, 0.05, 0.0));
        leaf.masked = true;
        let glass = tomori_assets::Material::new("glass", Vec4::new(1.0, 0.9, 0.6, 0.5))
            .with_emissive(Vec3::splat(2.0));
        for material in [leaf, glass] {
            let drawn = model(&Material::from(&material)).expect("models draw it");
            assert_eq!(
                (
                    drawn.base_color,
                    drawn.emissive,
                    drawn.masked,
                    drawn.texture
                ),
                (
                    material.base_color,
                    material.emissive,
                    material.masked,
                    None
                )
            );
            assert_eq!(drawn.name, None);
        }
        assert_eq!(model(&Material::new(Shading::Own)), None);
        assert_eq!(
            model(&Material::new(Shading::Stylised).with_haze(false)),
            None
        );
    }

    #[test]
    fn a_cube_glows_by_its_material_s_glow() {
        assert_eq!(glow(&Material::new(Shading::Own)), Some(0.0));
        assert_eq!(
            glow(&Material::new(Shading::Own).with_glow(0.85)),
            Some(0.85)
        );
        assert_eq!(glow(&Material::new(Shading::Own).with_glow(3.0)), Some(1.0));
        assert_eq!(glow(&Material::new(Shading::Unlit)), None);
        assert_eq!(glow(&Material::new(Shading::Own).with_haze(false)), None);
    }
}
