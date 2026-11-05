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
