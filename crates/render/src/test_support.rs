// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Devices and connections for tests, from [`tomori_scene::test_support`], and saving what a rendering test drew.
pub use tomori_scene::test_support::*;

/// Save `sheet`, what a test drew, as a PNG at the path the environment variable `variable` names, if it names one, to look at.
pub fn preview(variable: &str, sheet: &image::RgbaImage) {
    save_preview(std::env::var_os(variable), sheet);
}

fn save_preview(path: Option<std::ffi::OsString>, sheet: &image::RgbaImage) {
    if let Some(path) = path {
        sheet.save(path).expect("the preview is saved");
    }
}

#[test]
fn a_preview_is_saved_where_its_variable_says() {
    let path = std::env::temp_dir().join(format!("tomori-preview-{}.png", std::process::id()));
    save_preview(Some(path.clone().into()), &image::RgbaImage::new(2, 1));
    assert_eq!(image::open(&path).unwrap().width(), 2);
    std::fs::remove_file(path).unwrap();
}

/// Sets this thread's glyph budget, and restores it to none when dropped, even if a test fails, as a test may run on a thread another used.
pub struct Budgeted;

impl Budgeted {
    pub fn new(budget: tomori_text::atlas::Budget) -> Self {
        tomori_text::atlas::budget(budget);
        tomori_text::atlas::frame();
        Self
    }

    /// A budget that draws no new glyph this frame.
    pub fn spent() -> Self {
        Self::new(tomori_text::atlas::Budget {
            time: std::time::Duration::MAX,
            glyphs: 0,
        })
    }
}

impl Drop for Budgeted {
    fn drop(&mut self) {
        tomori_text::atlas::budget(tomori_text::atlas::Budget::UNLIMITED);
        tomori_text::atlas::frame();
    }
}

/// `count` kanji that the shared atlas has not drawn yet, looking down from `from`, which each test gives differently so that tests running at once do not draw each other's: each is asked for with no budget, and waits if it is not drawn.
pub fn undrawn(from: char, count: usize) -> String {
    let _budget = Budgeted::spent();
    ('\u{4e00}'..=from)
        .rev()
        .filter(|ch| tomori_text::fonts::covers(*ch))
        .filter(|ch| {
            let waited = tomori_text::atlas::waited();
            tomori_text::atlas::glyph(*ch);
            tomori_text::atlas::waited() > waited
        })
        .take(count)
        .collect()
}

#[test]
fn undrawn_kanji_are_not_in_the_atlas_until_drawn() {
    let kanji = undrawn('\u{9e40}', 2);
    assert_eq!(kanji.chars().count(), 2);
    for ch in kanji.chars() {
        assert!(tomori_text::atlas::glyph(ch).is_some());
    }
    assert!(!undrawn('\u{9e40}', 2).contains(kanji.chars().next().unwrap()));
}

/// What `texture`, `width` by `height`, holds once drawn.
pub fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    (width, height): (u32, u32),
) -> image::RgbaImage {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: (width * height * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receiver.recv().unwrap().unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    buffer.unmap();
    image::RgbaImage::from_raw(width, height, data).unwrap()
}
