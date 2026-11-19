//! A sixel decoder for the graphics side channel.
//!
//! Only the raster subset terminals use is implemented: colour registers (RGB
//! and HLS), raster attributes, `$` and `-` motion, and `!` repeats. The
//! decoder returns straight RGBA, with undrawn pixels left transparent so the
//! pane background shows through.

/// Canvas limits. A sixel at the axis limit costs 64 MB of RGBA, so the pixel
/// count is capped as well.
const MAX_AXIS: usize = 4096;
const MAX_PIXELS: usize = 8 * 1024 * 1024;

pub struct SixelImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Decode the payload that follows the `q` in a sixel DCS.
pub fn decode(data: &[u8]) -> Option<SixelImage> {
    let mut canvas = Canvas::default();
    let mut colors: [[u8; 4]; 256] = [[0, 0, 0, 255]; 256];
    let mut color = 0u8;
    let mut x = 0usize;
    let mut y = 0usize;
    let mut declared: Option<(usize, usize)> = None;
    let mut index = 0usize;
    while index < data.len() {
        let byte = data[index];
        index += 1;
        match byte {
            b'"' => {
                let (params, next) = read_params(data, index);
                index = next;
                let width = params.get(2).copied().unwrap_or(0);
                let height = params.get(3).copied().unwrap_or(0);
                if width > 0 && height > 0 {
                    declared = Some((width as usize, height as usize));
                }
            }
            b'#' => {
                let (params, next) = read_params(data, index);
                index = next;
                let Some(register) = params.first().copied() else {
                    continue;
                };
                color = register.min(255) as u8;
                if params.len() < 2 {
                    continue;
                }
                let (red, green, blue) = match params.get(1).copied() {
                    Some(2) => (
                        percent(params.get(2).copied().unwrap_or(0)),
                        percent(params.get(3).copied().unwrap_or(0)),
                        percent(params.get(4).copied().unwrap_or(0)),
                    ),
                    Some(1) => hls_to_rgb(
                        params.get(2).copied().unwrap_or(0),
                        params.get(3).copied().unwrap_or(0),
                        params.get(4).copied().unwrap_or(0),
                    ),
                    _ => (0, 0, 0),
                };
                colors[register.min(255) as usize] = [red, green, blue, 255];
            }
            b'!' => {
                let (params, next) = read_params(data, index);
                index = next;
                let repeat = params
                    .first()
                    .copied()
                    .unwrap_or(1)
                    .max(1)
                    .min(MAX_AXIS as u32);
                if let Some(next_byte) = data.get(index).copied() {
                    index += 1;
                    if (0x3f..=0x7e).contains(&next_byte) {
                        for _ in 0..repeat {
                            draw(&mut canvas, &colors, color, x, y, next_byte);
                            x += 1;
                        }
                    }
                }
            }
            b'$' => x = 0,
            b'-' => {
                x = 0;
                y += 6;
            }
            b'0'..=b'9' | b';' => {
                // A stray parameter run after the header; skip it.
                let (_, next) = read_params(data, index - 1);
                index = next;
            }
            0x3f..=0x7e => {
                draw(&mut canvas, &colors, color, x, y, byte);
                x += 1;
            }
            _ => {}
        }
        if x > MAX_AXIS || y > MAX_AXIS {
            return None;
        }
    }
    let (width, height) = match declared {
        Some((width, height)) => (width.min(MAX_AXIS), height.min(MAX_AXIS)),
        None => (canvas.width, canvas.height),
    };
    if width == 0 || height == 0 {
        return None;
    }
    canvas.crop(width, height)
}

fn percent(value: u32) -> u8 {
    ((value.min(100) * 255) / 100) as u8
}

/// Sixel HLS uses degrees for hue and percentages for lightness and saturation.
fn hls_to_rgb(hue: u32, lightness: u32, saturation: u32) -> (u8, u8, u8) {
    let h = (hue.min(360) as f32) / 60.0;
    let l = (lightness.min(100) as f32) / 100.0;
    let s = (saturation.min(100) as f32) / 100.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let channel = |value: f32| ((value + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (channel(r), channel(g), channel(b))
}

fn draw(canvas: &mut Canvas, colors: &[[u8; 4]; 256], color: u8, x: usize, y: usize, byte: u8) {
    let bits = byte - 0x3f;
    for bit in 0..6 {
        if bits & (1 << bit) != 0 {
            canvas.set(x, y + bit, colors[usize::from(color)]);
        }
    }
}

/// Read `;`-separated decimal parameters starting at `index`.
fn read_params(data: &[u8], mut index: usize) -> (Vec<u32>, usize) {
    let mut params = Vec::new();
    let mut value = 0u32;
    let mut seen = false;
    while index < data.len() {
        match data[index] {
            b'0'..=b'9' => {
                value = value
                    .saturating_mul(10)
                    .saturating_add(u32::from(data[index] - b'0'));
                seen = true;
                index += 1;
            }
            b';' => {
                params.push(value);
                value = 0;
                seen = false;
                index += 1;
            }
            _ => break,
        }
    }
    if seen {
        params.push(value);
    }
    (params, index)
}

#[derive(Default)]
struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 4]>,
}

impl Canvas {
    fn set(&mut self, x: usize, y: usize, pixel: [u8; 4]) {
        if x >= MAX_AXIS || y >= MAX_AXIS {
            return;
        }
        if x >= self.width || y >= self.height {
            let width = self.width.max(x + 1);
            let height = self.height.max(y + 1);
            if width * height > MAX_PIXELS {
                return;
            }
            let mut pixels = vec![[0, 0, 0, 0]; width * height];
            for row in 0..self.height {
                let source = row * self.width;
                let target = row * width;
                pixels[target..target + self.width]
                    .copy_from_slice(&self.pixels[source..source + self.width]);
            }
            self.pixels = pixels;
            self.width = width;
            self.height = height;
        }
        self.pixels[y * self.width + x] = pixel;
    }

    fn crop(self, width: usize, height: usize) -> Option<SixelImage> {
        if width == 0 || height == 0 || width * height > MAX_PIXELS {
            return None;
        }
        let mut pixels = vec![0u8; width * height * 4];
        let source_width = self.width.min(width);
        for row in 0..self.height.min(height) {
            for column in 0..source_width {
                let source = self.pixels[row * self.width + column];
                let target = (row * width + column) * 4;
                pixels[target..target + 4].copy_from_slice(&source);
            }
        }
        Some(SixelImage {
            width: width as u32,
            height: height as u32,
            pixels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_one_sixel_character_then_a_full_column_of_six_pixels_is_painted() {
        let image = decode(b"#0;2;100;0;0#0~").expect("image");
        assert_eq!((image.width, image.height), (1, 6));
        assert_eq!(&image.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&image.pixels[20..24], &[255, 0, 0, 255]);
    }

    #[test]
    fn given_a_repeat_then_the_character_covers_several_columns() {
        let image = decode(b"#0;2;0;0;100#0!4~").expect("image");
        assert_eq!((image.width, image.height), (4, 6));
        assert_eq!(&image.pixels[0..4], &[0, 0, 255, 255]);
        assert_eq!(&image.pixels[12..16], &[0, 0, 255, 255]);
    }

    #[test]
    fn given_a_carriage_return_then_the_next_character_restarts_the_row() {
        let image = decode(b"#0;2;100;0;0#0~$#1;2;0;100;0#1~").expect("image");
        assert_eq!((image.width, image.height), (1, 6));
        assert_eq!(&image.pixels[0..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn given_a_newline_then_the_next_band_starts_six_pixels_lower() {
        let image = decode(b"#0;2;100;0;0#0~-~").expect("image");
        assert_eq!((image.width, image.height), (1, 12));
        assert_eq!(&image.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&image.pixels[24..28], &[255, 0, 0, 255]);
    }

    #[test]
    fn given_raster_attributes_then_the_declared_size_is_used() {
        let image = decode(b"\"1;1;3;9#0;2;0;0;0#0~").expect("image");
        assert_eq!((image.width, image.height), (3, 9));
    }

    #[test]
    fn given_hls_red_then_the_converted_colour_is_red() {
        let image = decode(b"#0;1;0;50;100#0~").expect("image");
        assert_eq!(&image.pixels[0..3], &[255, 0, 0]);
    }

    #[test]
    fn given_no_pixels_then_decoding_fails() {
        assert!(decode(b"#0;2;0;0;0").is_none());
        assert!(decode(b"").is_none());
    }
}
