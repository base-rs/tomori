// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The projection's panels keep their meshes on the GPU for as long as the frame step hands over the same faces,
//! wherever they are drawn among the others, rebuild one only when its face changes, recolour only its frame when that
//! is all that changed, and free those no longer drawn beyond a few.
use super::*;
use std::sync::Arc;
use tomori_scene::canvas::Canvas;
use tomori_scene::ui_text::{Row, Style};

/// A framed face reading `text`.
fn face(text: &str) -> Arc<Canvas> {
    let mut canvas = Canvas::new(12, 0.002);
    canvas.rows = vec![Row::text(text, Style::plain([1.0; 4]), 12)];
    canvas.border = Some([0.3, 0.4, 0.5, 0.9]);
    Arc::new(canvas)
}

fn sheets(faces: &[Arc<Canvas>]) -> Vec<Sheet> {
    faces
        .iter()
        .enumerate()
        .map(|(index, face)| Sheet {
            canvas: Some(face.clone()),
            model: Some(Mat4::from_translation(Vec3::X * index as f32)),
        })
        .collect()
}

/// How many times every card has been built, and recoloured.
fn builds(panels: &Panels) -> (usize, usize) {
    panels
        .cards
        .iter()
        .fold((0, 0), |(built, recoloured), card| {
            let (b, r) = card.panel.builds;
            (built + b, recoloured + r)
        })
}

/// The faces drawn now, in order, as their cards show them.
fn drawn(panels: &Panels) -> Vec<Arc<Canvas>> {
    panels
        .drawn
        .iter()
        .map(|(card, _)| panels.cards[*card].panel.shown().unwrap().clone())
        .collect()
}

#[test]
fn a_face_keeps_its_mesh_wherever_it_is_drawn_and_only_its_frame_is_recoloured() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 4, 4);
        let mut panels = Panels::new(&device, &renderer);
        let mut faces: Vec<Arc<Canvas>> = ["one", "two", "three"].map(face).to_vec();
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        assert_eq!(builds(&panels), (3, 0));
        // The same faces again, in any order, keep their meshes, and are drawn in that order where they are put.
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        faces.reverse();
        let reversed = sheets(&faces);
        panels.set_cards(&device, &queue, &renderer, &reversed);
        assert_eq!(builds(&panels), (3, 0));
        let listed: Vec<Mat4> = panels
            .list(Mat4::IDENTITY)
            .iter()
            .map(|(_, model)| *model)
            .collect();
        assert_eq!(
            listed,
            reversed
                .iter()
                .map(|sheet| sheet.model.unwrap())
                .collect::<Vec<_>>()
        );
        assert!(
            drawn(&panels)
                .iter()
                .zip(&faces)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        );
        // A face added before them all is built, and nothing else.
        faces.insert(0, face("zero"));
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        assert_eq!(builds(&panels), (4, 0));
        // Framed brightly, as it is pointed at, a face's frame alone is rewritten.
        let mut pointed = (*faces[2]).clone();
        pointed.border = Some([0.35, 0.95, 1.0, 1.0]);
        faces[2] = Arc::new(pointed);
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        assert_eq!(builds(&panels), (4, 1));
        // The same face made again, as a sheet drawn afresh each frame is, keeps its mesh; another is built again.
        faces[1] = Arc::new((*faces[1]).clone());
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        assert_eq!(builds(&panels), (4, 1));
        faces[1] = face("changed");
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        assert_eq!(builds(&panels), (5, 1));
        // A face drawn twice at once is drawn by two cards.
        faces.push(faces[0].clone());
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        panels.set_cards(&device, &queue, &renderer, &sheets(&faces));
        assert_eq!(panels.cards.len(), 5);
        assert_eq!(builds(&panels), (6, 1));
    });
}

#[test]
fn cards_no_longer_drawn_are_freed_beyond_a_few_spare() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 4, 4);
        let mut panels = Panels::new(&device, &renderer);
        let many: Vec<Arc<Canvas>> = (0..20).map(|index| face(&format!("{index}"))).collect();
        panels.set_cards(&device, &queue, &renderer, &sheets(&many));
        assert_eq!(panels.cards.len(), 20);
        // Two of them still drawn, the rest removed: a few are kept for faces to come, and the others freed.
        let few = vec![many[5].clone(), many[17].clone()];
        panels.set_cards(&device, &queue, &renderer, &sheets(&few));
        assert_eq!(panels.cards.len(), 2 + SPARE_CARDS);
        assert_eq!(panels.faces.len(), 2 + SPARE_CARDS);
        assert!(
            drawn(&panels)
                .iter()
                .zip(&few)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        );
        assert!(
            panels.cards.iter().all(|card| card.panel.builds == (1, 0)),
            "those kept keep their meshes"
        );
        // Every face a card knows is the one it shows.
        for (address, card) in &panels.faces {
            assert_eq!(
                *address,
                crate::scene_graphics::address(panels.cards[*card].panel.shown().unwrap())
            );
        }
        // A face to come takes a spare card, rather than a new one.
        let more = vec![many[5].clone(), many[17].clone(), face("new")];
        panels.set_cards(&device, &queue, &renderer, &sheets(&more));
        assert_eq!(panels.cards.len(), 2 + SPARE_CARDS);
        assert_eq!(builds(&panels), (2 + SPARE_CARDS + 1, 0));
        // Drawn again, a face a spare card still shows keeps its mesh.
        let spare = many
            .iter()
            .find(|face| {
                !more.iter().any(|shown| Arc::ptr_eq(shown, face))
                    && panels
                        .faces
                        .contains_key(&crate::scene_graphics::address(face))
            })
            .expect("a spare card shows a face removed")
            .clone();
        panels.set_cards(&device, &queue, &renderer, &sheets(&[spare]));
        let (card, _) = panels.drawn[0];
        assert_eq!(panels.cards[card].panel.builds, (1, 0));
        let none: [Sheet; 0] = [];
        panels.set_cards(&device, &queue, &renderer, &none);
        assert!(panels.list(Mat4::IDENTITY).is_empty());
        assert_eq!(panels.cards.len(), SPARE_CARDS);
    });
}
