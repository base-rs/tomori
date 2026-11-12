// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A scenery and an avatar that submit what they draw to the renderer's world: culled in their own coordinates, carried
//! into the space's, only while they are drawn, and forgotten when they are dropped.
use std::sync::Mutex;

use tomori_world::{Aabb, Item, ItemId, Layer, Mobility, RootId, World};

use super::*;
use crate::graphics::Renderer;

/// What a [`Submitter`] was told: the roots it submitted in, and what the eyes may see of its item each time it was
/// culled, if the views said.
#[derive(Default)]
struct Heard {
    roots: Vec<RootId>,
    seen: Vec<Option<Vec<bool>>>,
    worlds: usize,
}

/// A scenery with one dynamic item of two instances: a box 4 m ahead of its origin along −Z, and one 4 m behind.
#[derive(Clone, Default)]
struct Submitter {
    item: Arc<Mutex<Option<ItemId>>>,
    heard: Arc<Mutex<Heard>>,
}

const AHEAD: Vec3 = Vec3::new(0.0, 0.0, -4.0);

impl Scenery for Submitter {
    fn submit(&mut self, world: &mut World, root: RootId) {
        let mut item = self.item.lock().unwrap();
        let id = *item
            .get_or_insert_with(|| world.add(Item::new(root, Layer::Opaque, Mobility::Dynamic)));
        world.place(
            id,
            [AHEAD, -AHEAD].map(|at| Some(Aabb::new(at - 0.2, at + 0.2))),
        );
        self.heard.lock().unwrap().roots.push(root);
    }

    fn cull(&mut self, _queue: &wgpu::Queue, views: &Views<'_>) -> tomori_assets::bounds::Tally {
        let item = self
            .item
            .lock()
            .unwrap()
            .expect("submitted before it is culled");
        let mut heard = self.heard.lock().unwrap();
        heard.seen.push(views.seen(item).map(<[bool]>::to_vec));
        heard.worlds += usize::from(views.world().is_some());
        tomori_assets::bounds::Tally::default()
    }

    fn prepare(&self, _queue: &wgpu::Queue, _eye: usize, _scene_to_clip: Mat4) {}

    fn draw(&self, _pass: &mut wgpu::RenderPass<'_>, _eye: usize) {}

    fn set_reveal(&mut self, _reveal: Option<(Vec3, f32)>) {}
}

/// An eye at the space's origin, looking along −Z.
fn ahead() -> Mat4 {
    glam::camera::rh::proj::directx::perspective(1.4, 1.0, 0.05, 2_000.0)
}

#[test]
fn a_scenery_s_items_are_culled_in_its_own_coordinates_while_it_is_drawn() {
    crate::test_gpu::with(|crate::test_gpu::Gpu { device, queue, .. }| {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let scenery = Submitter::default();
        let mut sceneries = Sceneries::empty();
        let registered = scenery.clone();
        sceneries.register("submitting", move |_| {
            Box::new(registered.clone()) as Box<dyn Scenery>
        });
        sceneries.register("plain", test::blank);
        let mut renderer = Renderer::with_sceneries(&device, &queue, format, 8, 8, 1, &sceneries);
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("submitting scenery eye"),
                size: wgpu::Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let render = |renderer: &mut Renderer| {
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[ahead()],
                Mat4::IDENTITY,
                &[],
            );
        };
        let show = |renderer: &mut Renderer, environment| {
            renderer.set_scenery(crate::arrival_motion::Scenery {
                environment: Environment::new(environment),
                reveal: None,
            });
        };
        show(&mut renderer, "submitting");
        render(&mut renderer);
        // What is ahead of the eye is seen, and what is behind it is not.
        assert_eq!(
            scenery.heard.lock().unwrap().seen,
            [Some(vec![true, false])]
        );
        // Carried about, so that its −Z is the space's +Z, what is behind its origin is ahead of the eye.
        renderer.set_carriage(tomori_scene::anchors::Anchor {
            position: Vec3::ZERO,
            orientation: glam::Quat::from_rotation_y(std::f32::consts::PI),
        });
        render(&mut renderer);
        // Turned off to compare, both are seen.
        renderer.set_culling(false);
        render(&mut renderer);
        renderer.set_culling(true);
        {
            let heard = scenery.heard.lock().unwrap();
            assert_eq!(
                heard.seen[1..],
                [Some(vec![false, true]), Some(vec![true, true])]
            );
            assert_eq!(heard.worlds, 3);
            assert!(heard.roots.windows(2).all(|pair| pair[0] == pair[1]));
        }
        // While another environment is drawn, it is neither submitted nor culled, and its item is left as it was.
        show(&mut renderer, "plain");
        render(&mut renderer);
        assert_eq!(scenery.heard.lock().unwrap().seen.len(), 3);
        let item = scenery.item.lock().unwrap().unwrap();
        assert_eq!(renderer.world().seen(item), [true, true]);
        // Drawn again, it submits in the same coordinates.
        show(&mut renderer, "submitting");
        render(&mut renderer);
        let heard = scenery.heard.lock().unwrap();
        assert_eq!(heard.roots.len(), 4);
        assert_eq!(heard.seen.last(), Some(&Some(vec![false, true])));
    });
}

#[test]
fn an_avatar_s_items_are_forgotten_when_its_scenery_is_dropped() {
    crate::test_gpu::with(|crate::test_gpu::Gpu { device, queue, .. }| {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let made: Arc<Mutex<Vec<Submitter>>> = Arc::default();
        let mut sceneries = Sceneries::empty();
        let making = made.clone();
        sceneries.set_avatar(
            "river",
            |_| Box::new(Still) as Box<dyn tomori_scene::avatar::Avatar>,
            move |_| {
                let scenery = Submitter::default();
                making.lock().unwrap().push(scenery.clone());
                Box::new(scenery) as Box<dyn Scenery>
            },
        );
        let mut renderer = Renderer::with_sceneries(&device, &queue, format, 8, 8, 1, &sceneries);
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("submitting avatar eye"),
                size: wgpu::Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let shown = |space| tomori_scene::avatar::Shown {
            avatar: "river",
            space,
            drawn: None,
        };
        renderer.set_avatar(&device, &queue, Some(&shown("space-1")));
        renderer.render(
            &device,
            &queue,
            std::slice::from_ref(&view),
            &[ahead()],
            Mat4::IDENTITY,
            &[],
        );
        let first = made.lock().unwrap()[0].clone();
        let item = first.item.lock().unwrap().unwrap();
        assert_eq!(first.heard.lock().unwrap().seen, [Some(vec![true, false])]);
        assert!(renderer.world().contains(item));
        // Made anew for another space, the last one's item is gone; with the swarm shown, so is the new one's.
        renderer.set_avatar(&device, &queue, Some(&shown("space-2")));
        assert!(!renderer.world().contains(item));
        renderer.render(
            &device,
            &queue,
            std::slice::from_ref(&view),
            &[ahead()],
            Mat4::IDENTITY,
            &[],
        );
        let second = made.lock().unwrap()[1].item.lock().unwrap().unwrap();
        assert!(renderer.world().contains(second));
        renderer.set_avatar(&device, &queue, None);
        assert!(!renderer.world().contains(second));
    });
}

#[test]
fn views_without_a_world_say_nothing_of_items_and_a_stage_culls_a_scenery_alone() {
    let clips = [ahead()];
    let views = Views::new(&clips);
    assert!(views.world().is_none());
    let mut world = World::new();
    let root = world.add_root();
    let item = world.add(Item::new(root, Layer::Opaque, Mobility::Static));
    assert!(views.seen(item).is_none());
    assert!(Views::unculled(&clips).frustum().is_none());
    // In a world, an item not in it is not said of either.
    world.remove(item);
    assert!(Views::of(&world, root).seen(item).is_none());
    crate::test_gpu::with(|crate::test_gpu::Gpu { queue, .. }| {
        let mut scenery = Submitter::default();
        let mut stage = Stage::default();
        let tally = stage.cull(&mut scenery, &queue, &clips, true);
        assert_eq!(tally, tomori_assets::bounds::Tally::default());
        stage.cull(&mut scenery, &queue, &clips, false);
        assert_eq!(
            scenery.heard.lock().unwrap().seen,
            [Some(vec![true, false]), Some(vec![true, true])]
        );
        let (world, root) = stage.world();
        assert_eq!(world.tally(root).drawn, 2);
    });
}
