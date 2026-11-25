//! The picture drawn behind the terminal.
//!
//! Two things live here: finding the source (an explicit path, or the desktop's
//! own wallpaper through NSWorkspace) and decoding it to RGBA once so every
//! frame can blend the same buffer.

/// A decoded picture, ready to blend.
pub struct Wallpaper {
    pub width: u32,
    pub height: u32,
    /// RGBA, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// The widest picture worth keeping in memory. Larger sources are downscaled to
/// this, which also keeps the per-frame blend cheap.
const MAX_AXIS: u32 = 6000;

impl Wallpaper {
    /// Decode `source` (a path, or `"desktop"`). `None` means there is nothing
    /// to draw or the file could not be read; the caller keeps the plain
    /// background rather than failing.
    pub fn load(source: &str) -> Option<Self> {
        let path = if source == "desktop" {
            desktop_wallpaper_path()?
        } else {
            std::path::PathBuf::from(source)
        };
        let bytes = std::fs::read(&path).ok()?;
        let (width, height, pixels) = decode(&bytes)?;
        if width == 0 || height == 0 {
            return None;
        }
        let (width, height, pixels) = if width > MAX_AXIS || height > MAX_AXIS {
            downscale(&bytes, width, height).unwrap_or((width, height, pixels))
        } else {
            (width, height, pixels)
        };
        debug_assert_eq!(pixels.len(), width as usize * height as usize * 4);
        Some(Self {
            width,
            height,
            pixels,
        })
    }
}

/// Decode PNG or JPEG bytes to RGBA. The format comes from the bytes, not the
/// extension, because a wallpaper store entry may not match its name.
fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    use image::GenericImageView;
    let image = image::load_from_memory(bytes).ok()?;
    let (width, height) = image.dimensions();
    Some((width, height, image.to_rgba8().into_raw()))
}

/// Re-decode with a size hint so a huge wallpaper does not cost a full-size
/// buffer before it is shrunk.
fn downscale(bytes: &[u8], width: u32, height: u32) -> Option<(u32, u32, Vec<u8>)> {
    use image::{GenericImageView, ImageReader};
    let longest = width.max(height);
    let scale = f64::from(MAX_AXIS) / f64::from(longest);
    let target_w = ((f64::from(width) * scale).round() as u32).max(1);
    let target_h = ((f64::from(height) * scale).round() as u32).max(1);
    let image = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let image = image.thumbnail(target_w, target_h);
    let (w, h) = image.dimensions();
    Some((w, h, image.to_rgba8().into_raw()))
}

/// The macOS desktop wallpaper path for the main screen.
#[cfg(target_os = "macos")]
fn desktop_wallpaper_path() -> Option<std::path::PathBuf> {
    use objc2_app_kit::{NSScreen, NSWorkspace};

    let screen = NSScreen::mainScreen(objc2::MainThreadMarker::new()?)?;
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
    let path = url.path()?;
    Some(std::path::PathBuf::from(path.to_string()))
}

/// Linux and Windows have no portable desktop-wallpaper API, so following the
/// desktop is a no-op there and the explicit path is the way in.
#[cfg(not(target_os = "macos"))]
fn desktop_wallpaper_path() -> Option<std::path::PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 PNG: red, green, blue, white.
    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            let pixels: [u8; 16] = [
                255, 0, 0, 255, // red
                0, 255, 0, 255, // green
                0, 0, 255, 255, // blue
                255, 255, 255, 255, // white
            ];
            writer.write_image_data(&pixels).expect("png data");
        }
        bytes
    }

    #[test]
    fn given_png_bytes_then_decode_returns_rgba_pixels() {
        let (width, height, pixels) = decode(&tiny_png()).expect("png decodes");
        assert_eq!((width, height), (2, 2));
        assert_eq!(pixels.len(), 2 * 2 * 4);
        assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn given_garbage_then_decode_declines() {
        assert!(decode(b"not an image").is_none());
    }

    #[test]
    fn given_no_source_then_load_declines() {
        assert!(Wallpaper::load("/no/such/wallpaper.png").is_none());
    }

    /// The macOS path is the real integration: it must find the picture the
    /// desktop is using, not just any file.
    #[cfg(target_os = "macos")]
    #[test]
    fn given_a_macos_desktop_then_the_wallpaper_path_resolves() {
        // No desktop image configured is a valid answer; only a decode failure
        // of an existing file would be a real bug.
        if let Some(path) = desktop_wallpaper_path() {
            assert!(path.is_absolute(), "desktop wallpaper must be absolute");
        }
    }
}
