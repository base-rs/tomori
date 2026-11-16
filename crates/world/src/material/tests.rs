// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Materials described, from a model's and by hand, kept with their roots, painted on instances and lighting them.
use glam::{Mat4, Vec3, Vec4};

use super::*;
use crate::view::tests::both;
use crate::{Aabb, Item, Layer, Light, Mobility, World};

#[test]
fn a_material_is_white_rough_opaque_and_hazed_until_said_otherwise() {
    let stylised = Material::new(Shading::Stylised);
    assert_eq!(stylised.base_color, Vec4::ONE);
    assert_eq!((stylised.roughness, stylised.metalness), (1.0, 0.0));
    assert_eq!(
        (stylised.base_color_texture, stylised.normal_texture),
        (None, None)
    );
    assert_eq!(
        (stylised.flat_color, stylised.projection),
        (Vec3::ONE, None)
    );
    assert_eq!((stylised.emissive, stylised.glow), (Vec3::ZERO, 0.0));
    assert_eq!(stylised.alpha, Alpha::Opaque);
    // Only what is lit by its environment is lit by its root's lights by default.
    assert!(stylised.hazed && stylised.lamps);
    for shading in [Shading::Own, Shading::Unlit, Shading::Custom(3)] {
        let material = Material::new(shading);
        assert!(material.hazed && !material.lamps, "{shading:?}");
    }
    assert_eq!(Shading::default(), Shading::Stylised);
    assert_eq!(Alpha::default(), Alpha::Opaque);
    // Every part can be said.
    let grain = Digest::of(b"grain");
    let bumps = Digest::of(b"bumps");
    let painted = Material::new(Shading::Custom(1))
        .with_base_color(Vec4::new(0.5, 0.25, 0.125, 0.75))
        .with_base_color_texture(grain)
        .with_flat_color(Vec3::new(0.2, 0.1, 0.05))
        .with_projection(Projection::new(2.0).with_turn(0.5))
        .with_surface(0.4, 0.9)
        .with_normal_texture(bumps)
        .with_emissive(Vec3::new(1.0, 0.5, 0.0))
        .with_glow(0.3)
        .with_alpha(Alpha::Blend)
        .with_lamps(true)
        .with_haze(false);
    assert_eq!(
        painted,
        Material {
            shading: Shading::Custom(1),
            base_color: Vec4::new(0.5, 0.25, 0.125, 0.75),
            base_color_texture: Some(grain),
            flat_color: Vec3::new(0.2, 0.1, 0.05),
            projection: Some(Projection {
                metres: 2.0,
                turn: 0.5
            }),
            roughness: 0.4,
            metalness: 0.9,
            normal_texture: Some(bumps),
            emissive: Vec3::new(1.0, 0.5, 0.0),
            glow: 0.3,
            alpha: Alpha::Blend,
            hazed: false,
            lamps: true,
        }
    );
    assert_eq!(
        Projection::new(1.5),
        Projection {
            metres: 1.5,
            turn: 0.0
        }
    );
}

#[test]
fn a_model_s_material_is_stylised_of_its_colour_and_light_cut_out_if_masked() {
    let glass = tomori_assets::Material::new("glass", Vec4::new(0.9, 0.8, 0.4, 1.0))
        .with_emissive(Vec3::new(2.0, 1.5, 0.5));
    let material = Material::from(&glass);
    assert_eq!(
        material,
        Material::new(Shading::Stylised)
            .with_base_color(glass.base_color)
            .with_emissive(glass.emissive)
    );
    let mut leaf = tomori_assets::Material::new("leaf", Vec4::ONE);
    leaf.masked = true;
    assert_eq!(Material::from(&leaf).alpha, Alpha::Mask);
}

#[test]
fn materials_are_kept_with_their_roots_and_painted_on_instances() {
    let mut world = World::new();
    let scenery = world.add_root();
    let space = world.add_root();
    let stone = world.add_material(scenery, Material::new(Shading::Stylised));
    let moss = world.add_material(
        scenery,
        Material::new(Shading::Stylised).with_base_color(Vec4::new(0.2, 0.5, 0.1, 1.0)),
    );
    let card = world.add_material(space, Material::new(Shading::Unlit));
    assert_eq!(
        world.material(stone),
        Some(&Material::new(Shading::Stylised))
    );
    // Changed, it is drawn so.
    world.material_mut(stone).unwrap().roughness = 0.5;
    assert_eq!(world.material(stone).unwrap().roughness, 0.5);
    // An item's instances are drawn with its material, but where they are painted with their own.
    let item = world.add(Item::new(scenery, Layer::Opaque, Mobility::Static).with_material(stone));
    let bare = world.add(Item::new(scenery, Layer::Opaque, Mobility::Static));
    world.place(item, [None, None, None]);
    assert_eq!(world.instance_material(item, 0), world.material(stone));
    world.paint(item, [None, Some(moss)]);
    assert_eq!(world.instance_material(item, 0), world.material(stone));
    assert_eq!(world.instance_material(item, 1), world.material(moss));
    assert_eq!(world.instance_material(item, 2), world.material(stone));
    // Placed again, it keeps its painting until it is painted again.
    world.place(item, [None, None]);
    assert_eq!(world.instance_material(item, 1), world.material(moss));
    world.paint(item, []);
    assert_eq!(world.instance_material(item, 1), world.material(stone));
    // An item naming none, or not here, has none.
    world.place(bare, [None]);
    assert_eq!(world.instance_material(bare, 0), None);
    world.remove(bare);
    assert_eq!(world.instance_material(bare, 0), None);
    // Removed, a material is gone, and what names it is drawn as though it named none.
    assert!(world.remove_material(moss));
    assert!(!world.remove_material(moss));
    assert_eq!(world.material(moss), None);
    assert!(world.material_mut(moss).is_none());
    world.paint(item, [Some(moss)]);
    assert_eq!(world.instance_material(item, 0), None);
    // Clearing a root takes its materials, and leaves the others'.
    world.clear(scenery);
    assert_eq!(world.material(stone), None);
    assert_eq!(world.material(card), Some(&Material::new(Shading::Unlit)));
    let next = world.add_material(scenery, Material::new(Shading::Own));
    assert_ne!(next, stone);
}

#[test]
fn only_what_its_material_says_is_lit_by_lamps_is_lit_by_its_root_s_lights() {
    let mut world = World::new();
    let root = world.add_root();
    world.set_views(&both(Vec3::new(0.0, 1.6, 0.0), 0.0));
    world.add_light(Light::new(root, Vec3::new(0.0, 1.0, -3.0), 4.0, Vec3::ONE));
    let lit = world.add_material(root, Material::new(Shading::Stylised));
    let glowing = world.add_material(root, Material::new(Shading::Own));
    let near = Some(Aabb::new(
        Vec3::new(-0.5, 0.5, -3.5),
        Vec3::new(0.5, 1.5, -2.5),
    ));
    // An item of each, one of both painted, and one naming a material since removed, which is lit as before.
    let models = world.add(Item::new(root, Layer::Opaque, Mobility::Static).with_material(lit));
    let cubes = world.add(Item::new(root, Layer::Opaque, Mobility::Dynamic).with_material(glowing));
    let parts = world.add(Item::new(root, Layer::Opaque, Mobility::Static).with_material(lit));
    let gone = world.add_material(root, Material::new(Shading::Own));
    let orphan = world.add(Item::new(root, Layer::Opaque, Mobility::Dynamic).with_material(gone));
    let plain = world.add(Item::new(root, Layer::Opaque, Mobility::Dynamic));
    world.remove_material(gone);
    for item in [models, cubes, parts, orphan, plain] {
        world.place(item, [near, near]);
    }
    world.paint(parts, [Some(glowing)]);
    world.set_transform(root, Mat4::IDENTITY);
    world.cull();
    let counts = |item| world.lit(item).map(<[_]>::len).collect::<Vec<_>>();
    assert_eq!(counts(models), [1, 1]);
    assert_eq!(counts(cubes), [0, 0]);
    assert_eq!(counts(parts), [0, 1]);
    assert_eq!(counts(orphan), [1, 1]);
    assert_eq!(counts(plain), [1, 1]);
}
