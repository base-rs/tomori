// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The images of image panels: reading a file a space's agent shows, safely, into the one format clients decode, and
//! keeping each by its content. See `design/projection.md`.
//!
//! **Reading.** The file is any readable regular file on the server's machine, by a path absolute or relative to the
//! space's directory, its symbolic links resolved: the agent can already read any file the server can with its own
//! tools, so confining it to a directory would protect nothing. What the path names is refused unless it is a regular
//! file of at most [`SOURCE_BYTES`], before it is opened, so that a device is never opened; and it is opened without
//! waiting and read only so far, so that a pipe or a device put in its place cannot hold its thread, and is refused as
//! too large or not an image. Only PNG, JPEG, WebP and GIF are decoded, by their content rather than their name, with
//! `image`'s pure Rust decoders, within [`SOURCE_SIDE`] on a side, [`SOURCE_PIXELS`] and [`DECODE_BYTES`] of memory,
//! so that a small file claiming an enormous image is refused before it is decoded.
//!
//! **What clients are sent.** The image is turned as its EXIF orientation says, laid over white where it is
//! transparent, scaled down to [`IMAGE_SIDE`] on its longer side, and encoded again as a PNG of 8-bit RGB without
//! metadata, scaled down further until it fits [`IMAGE_BYTES`]. That PNG is the only format a client decodes, and is
//! kept and sent by its SHA-256.
//!
//! **Keeping.** [`ImageStore`] keeps each image in the state directory's `images` folder as `HASH.png`, and the latest
//! few in memory, and removes those no panel kept by any space shows, once they are a minute old, so that an image just
//! made for a panel not yet projected is not removed under it.
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage, imageops::FilterType};
use sha2::{Digest, Sha256};
use tomori_protocol::{IMAGE_BYTES, IMAGE_SIDE, Image, hash_text};

/// The largest file read for an image.
pub const SOURCE_BYTES: u64 = 20 * 1024 * 1024;

/// The longest side of an image decoded, in pixels.
pub const SOURCE_SIDE: u32 = 16_384;

/// The most pixels of an image decoded: a little over a 48-megapixel camera's, of 8,000 by 6,000.
pub const SOURCE_PIXELS: u64 = 50_000_000;

/// The most memory a decoder may take for one image.
pub const DECODE_BYTES: u64 = 512 * 1024 * 1024;

/// The longest path the agent may give, in bytes.
pub const PATH_BYTES: usize = 4096;

/// How old an image no panel shows must be before it is removed.
const GRACE: Duration = Duration::from_secs(60);

/// How many bytes of images are kept in memory, besides those in the state directory.
const MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Only one image is prepared at a time, so that several shown at once cannot take several decoders' memory together.
static PREPARING: Mutex<()> = Mutex::new(());

/// An image made for clients from a file: what a panel says of it, and its PNG.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub image: Image,
    pub png: Arc<[u8]>,
}

/// The image of the file at `path`, absolute or relative to `directory`, as clients are sent it, or why it cannot be
/// shown, as the agent is told.
pub fn prepare(directory: &Path, path: &str) -> Result<Prepared, String> {
    let bytes = read(directory, path)?;
    let _preparing = PREPARING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let image = decode(&bytes)?;
    drop(bytes);
    let rgb = flatten(image);
    Ok(encode(rgb))
}

/// The bytes of the regular file at `path`, absolute or relative to `directory`, of at most [`SOURCE_BYTES`].
fn read(directory: &Path, path: &str) -> Result<Vec<u8>, String> {
    if path.is_empty() {
        return Err("Give the path of an image file.".to_owned());
    }
    if path.len() > PATH_BYTES {
        return Err("That path is too long.".to_owned());
    }
    // Symbolic links are followed. What the path names is checked before it is opened, so that a device is never opened.
    let path = directory.join(path);
    let metadata = fs::metadata(&path).map_err(unreadable)?;
    regular(&metadata)?;
    if metadata.len() > SOURCE_BYTES {
        return Err(too_large());
    }
    read_file(open(&path).map_err(unreadable)?)
}

/// What a file that could not be read is, as the agent is told.
fn unreadable(error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "There is no file at that path.".to_owned(),
        io::ErrorKind::PermissionDenied => "Tomori may not read that file.".to_owned(),
        _ => format!("That file could not be read: {error}."),
    }
}

/// The bytes of `file`, at most [`SOURCE_BYTES`]: a file replaced, after its path was checked, by a device or a pipe is
/// read only so far, and without waiting, so that it is refused as too large or not an image.
fn read_file(file: fs::File) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() as u64 > SOURCE_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

/// Refuses what `metadata` describes unless it is a regular file.
fn regular(metadata: &fs::Metadata) -> Result<(), String> {
    if metadata.is_dir() {
        return Err("That is a folder, not an image file.".to_owned());
    }
    if !metadata.is_file() {
        return Err(
            "That is not a regular file, such as a device or a pipe, so Tomori will not read it."
                .to_owned(),
        );
    }
    Ok(())
}

fn too_large() -> String {
    format!(
        "That file is larger than {} MB, the most Tomori reads for an image.",
        SOURCE_BYTES / (1024 * 1024)
    )
}

/// `path`, opened for reading without waiting, so that a pipe or a device cannot hold the thread that opens it.
#[cfg(unix)]
fn open(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
}

#[cfg(not(unix))]
fn open(path: &Path) -> io::Result<fs::File> {
    fs::File::open(path)
}

/// The image `bytes` hold, decoded within bounds and turned as its EXIF orientation says.
fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    let unsupported = || "That file is not a PNG, JPEG, WebP or GIF image.".to_owned();
    let format = image::guess_format(bytes).map_err(|_| unsupported())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Gif
    ) {
        return Err(unsupported());
    }
    let mut reader = ImageReader::with_format(io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(SOURCE_SIDE);
    limits.max_image_height = Some(SOURCE_SIDE);
    limits.max_alloc = Some(DECODE_BYTES);
    reader.limits(limits);
    let unreadable = |error: image::ImageError| match error {
        image::ImageError::Limits(_) => too_many_pixels(),
        error => format!(
            "That image could not be read: {}.",
            tomori_protocol::text::one_line(&error.to_string(), 160)
        ),
    };
    let mut decoder = reader.into_decoder().map_err(unreadable)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > SOURCE_PIXELS {
        return Err(too_many_pixels());
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn too_many_pixels() -> String {
    format!(
        "That image is too large to show: at most {} megapixels, and {SOURCE_SIDE} pixels on a side.",
        SOURCE_PIXELS / 1_000_000
    )
}

/// `image` in 8-bit RGB, laid over white where it is transparent, and scaled down to fit [`IMAGE_SIDE`].
fn flatten(image: DynamicImage) -> RgbImage {
    let image = fit(image, IMAGE_SIDE);
    if !image.color().has_alpha() {
        return image.into_rgb8();
    }
    let rgba = image.into_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [red, green, blue, alpha] = rgba.get_pixel(x, y).0;
        let over = |value: u8| {
            let (value, alpha) = (u32::from(value), u32::from(alpha));
            ((value * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        };
        image::Rgb([over(red), over(green), over(blue)])
    })
}

/// `image`, scaled down, keeping its shape, so that its longer side is at most `side`.
fn fit(image: DynamicImage, side: u32) -> DynamicImage {
    if image.width().max(image.height()) <= side {
        return image;
    }
    image.resize(side, side, FilterType::Triangle)
}

/// `rgb` as the PNG clients are sent, scaled down by halves until it fits [`IMAGE_BYTES`].
fn encode(mut rgb: RgbImage) -> Prepared {
    loop {
        let mut png = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Default,
            image::codecs::png::FilterType::Adaptive,
        );
        image::ImageEncoder::write_image(
            encoder,
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        // A decoded image has pixels, and its buffer is its size, so it always encodes.
        .expect("an RGB image of its own size encodes");
        if png.len() <= IMAGE_BYTES as usize || rgb.width().max(rgb.height()) <= 1 {
            let hash: [u8; 32] = Sha256::digest(&png).into();
            let image = Image::new(
                hash_text(&hash),
                rgb.width(),
                rgb.height(),
                png.len() as u32,
            );
            return Prepared {
                image,
                png: png.into(),
            };
        }
        let side = rgb.width().max(rgb.height()) / 2;
        rgb = fit(DynamicImage::ImageRgb8(rgb), side).into_rgb8();
    }
}

/// The images of a server's image panels, by their SHA-256: in the state directory's `images` folder once the server
/// keeps spaces there, and the latest in memory. See the module's documentation.
#[derive(Default)]
pub struct ImageStore {
    directory: RwLock<Option<PathBuf>>,
    memory: Mutex<Memory>,
    /// The images shown when the store was last swept, so that it is swept again only when they change.
    swept: Mutex<Option<BTreeSet<String>>>,
    /// Held while an image is written to the directory: a write goes through a temporary file named after the image,
    /// which two spaces keeping the same image at once would otherwise share, one renaming it part written or finding it
    /// gone.
    writing: Mutex<()>,
}

#[derive(Default)]
struct Memory {
    images: HashMap<String, (Arc<[u8]>, SystemTime)>,
    /// The order they were last used in, the latest last.
    order: VecDeque<String>,
    bytes: usize,
}

impl Memory {
    fn used(&mut self, hash: &str) {
        self.order.retain(|other| other != hash);
        self.order.push_back(hash.to_owned());
    }

    fn remove(&mut self, hash: &str) {
        if let Some((png, _)) = self.images.remove(hash) {
            self.bytes -= png.len();
            self.order.retain(|other| other != hash);
        }
    }
}

impl ImageStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep images in `directory` from now on, creating it if needed, and keep in memory only the latest.
    pub fn keep_in(&self, directory: PathBuf) -> io::Result<()> {
        tomori_security::create_private_directory(&directory)?;
        *self.directory.write().expect("never left poisoned") = Some(directory);
        Ok(())
    }

    fn directory(&self) -> Option<PathBuf> {
        self.directory.read().expect("never left poisoned").clone()
    }

    fn file(directory: &Path, hash: &str) -> PathBuf {
        directory.join(format!("{hash}.png"))
    }

    /// Keep `prepared`'s image, or say why it could not be, as the agent is told.
    pub fn put(&self, prepared: &Prepared) -> Result<(), String> {
        let hash = &prepared.image.hash;
        if let Some(directory) = self.directory() {
            let file = Self::file(&directory, hash);
            // One already kept is the same image; written again, it is marked new, so that it is not swept before its panel
            // is projected.
            let _writing = self.writing.lock().expect("never left poisoned");
            tomori_security::write_private(&file, &prepared.png)
                .map_err(|error| format!("Tomori could not keep the image: {error}."))?;
        }
        self.remember(hash, prepared.png.clone());
        Ok(())
    }

    /// Hold `png` in memory as the image `hash`, forgetting the least recently used beyond [`MEMORY_BYTES`] while the
    /// images are also in a directory.
    fn remember(&self, hash: &str, png: Arc<[u8]>) {
        let bounded = self.directory().is_some();
        let mut memory = self.memory.lock().expect("never left poisoned");
        memory.remove(hash);
        memory.bytes += png.len();
        memory
            .images
            .insert(hash.to_owned(), (png, SystemTime::now()));
        memory.used(hash);
        while bounded && memory.bytes > MEMORY_BYTES && memory.order.len() > 1 {
            let oldest = memory.order.front().expect("more than one").clone();
            memory.remove(&oldest);
        }
    }

    /// The PNG of the image `hash`, if it is kept and is what its hash says.
    pub fn get(&self, hash: &str) -> Option<Arc<[u8]>> {
        tomori_protocol::hash_bytes(hash)?;
        {
            let mut memory = self.memory.lock().expect("never left poisoned");
            if let Some((png, _)) = memory.images.get(hash) {
                let png = png.clone();
                memory.used(hash);
                return Some(png);
            }
        }
        let directory = self.directory()?;
        let file = fs::File::open(Self::file(&directory, hash)).ok()?;
        let mut png = Vec::new();
        file.take(u64::from(IMAGE_BYTES) + 1)
            .read_to_end(&mut png)
            .ok()?;
        let found: [u8; 32] = Sha256::digest(&png).into();
        if png.len() > IMAGE_BYTES as usize || hash_text(&found) != hash {
            log::warn!("The kept image {hash} is not what its name says, so it is not sent.");
            return None;
        }
        let png: Arc<[u8]> = png.into();
        self.remember(hash, png.clone());
        Some(png)
    }

    /// Remove the images not in `shown`, the images of every panel the server keeps, that are older than [`GRACE`], if
    /// `shown` changed since the last sweep.
    pub fn sweep(&self, shown: BTreeSet<String>) {
        {
            let mut swept = self.swept.lock().expect("never left poisoned");
            if swept.as_ref() == Some(&shown) {
                return;
            }
            *swept = Some(shown.clone());
        }
        self.sweep_older(&shown, GRACE);
    }

    /// Remove the images not in `shown` that are older than `grace`.
    fn sweep_older(&self, shown: &BTreeSet<String>, grace: Duration) {
        let now = SystemTime::now();
        let old = |time: SystemTime| now.duration_since(time).is_ok_and(|age| age >= grace);
        {
            let mut memory = self.memory.lock().expect("never left poisoned");
            let unshown: Vec<String> = memory
                .images
                .iter()
                .filter(|(hash, (_, time))| !shown.contains(*hash) && old(*time))
                .map(|(hash, _)| hash.clone())
                .collect();
            for hash in unshown {
                memory.remove(&hash);
            }
        }
        let Some(directory) = self.directory() else {
            return;
        };
        let Ok(entries) = fs::read_dir(&directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            // What a write left behind, as when the server stopped part way through one: its temporary file is named after the image, and ends in `.tmp`.
            let Some(hash) = name.to_str().and_then(|name| {
                name.strip_suffix(".png").or(name
                    .strip_suffix(".tmp")
                    .and_then(|name| name.split('.').next()))
            }) else {
                continue;
            };
            if tomori_protocol::hash_bytes(hash).is_none() || shown.contains(hash) {
                continue;
            }
            let modified = entry.metadata().and_then(|metadata| metadata.modified());
            if modified.is_ok_and(old) {
                match fs::remove_file(entry.path()) {
                    Ok(()) => log::debug!("Removed the image {hash}, which no panel shows."),
                    Err(error) => log::warn!("Could not remove the image {hash}: {error}."),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
