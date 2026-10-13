use gpui_kit::{Image, ImageFormat};
use std::sync::Arc;

pub struct SpriteAtlas {
    pub frames: Vec<Arc<Image>>,
    pub icon_rgba: Vec<u8>,
}

impl SpriteAtlas {
    pub fn load() -> Result<Self, String> {
        Self::from_png(include_bytes!("../assets/lamar-cat.png"))
    }

    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        let sheet = image::load_from_memory(bytes)
            .map_err(|error| error.to_string())?
            .into_rgba8();
        let (width, height) = sheet.dimensions();
        if width != height || !(9..=4096).contains(&width) {
            return Err("sprite sheet must be square and at most 4096 pixels".into());
        }
        let mut crops = Vec::with_capacity(9);
        for index in 0..9 {
            let col = index % 3;
            let row = index / 3;
            let start_x = col * width / 3;
            let start_y = row * height / 3;
            let cell = image::imageops::crop_imm(
                &sheet,
                start_x,
                start_y,
                (col + 1) * width / 3 - start_x,
                (row + 1) * height / 3 - start_y,
            )
            .to_image();
            let mut bounds = None;
            for (x, y, pixel) in cell.enumerate_pixels() {
                if pixel.0[3] >= 20 {
                    let (min_x, min_y, max_x, max_y) = bounds.unwrap_or((x, y, x, y));
                    bounds = Some((min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)));
                }
            }
            let (min_x, min_y, max_x, max_y) =
                bounds.ok_or("sprite sheet contains an empty frame")?;
            crops.push(
                image::imageops::crop_imm(
                    &cell,
                    min_x,
                    min_y,
                    max_x - min_x + 1,
                    max_y - min_y + 1,
                )
                .to_image(),
            );
        }
        let max_width = crops.iter().map(image::RgbaImage::width).max().unwrap_or(1);
        let max_height = crops
            .iter()
            .map(image::RgbaImage::height)
            .max()
            .unwrap_or(1);
        let scale = (220.0 / f64::from(max_width)).min(220.0 / f64::from(max_height));
        let mut frames = Vec::with_capacity(9);
        let mut icon_rgba = Vec::new();
        for (index, crop) in crops.into_iter().enumerate() {
            let scaled = image::imageops::resize(
                &crop,
                (f64::from(crop.width()) * scale).round() as u32,
                (f64::from(crop.height()) * scale).round() as u32,
                image::imageops::FilterType::Nearest,
            );
            let mut aligned = image::RgbaImage::new(256, 256);
            image::imageops::overlay(
                &mut aligned,
                &scaled,
                i64::from((256 - scaled.width()) / 2),
                i64::from(236 - scaled.height()),
            );
            if index == 0 {
                icon_rgba =
                    image::imageops::resize(&aligned, 32, 32, image::imageops::FilterType::Nearest)
                        .into_raw();
            }
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(aligned)
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|error| error.to_string())?;
            frames.push(Arc::new(Image::from_bytes(
                ImageFormat::Png,
                png.into_inner(),
            )));
        }
        Ok(Self { frames, icon_rgba })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_sheet_produces_nine_transparent_aligned_frames() {
        let atlas = SpriteAtlas::load().expect("original sprite sheet");
        assert_eq!(atlas.frames.len(), 9);
        for frame in &atlas.frames {
            let pixels = image::load_from_memory(frame.bytes())
                .expect("frame")
                .into_rgba8();
            assert_eq!(pixels.dimensions(), (256, 256));
            assert_eq!(pixels.get_pixel(0, 0).0[3], 0);
            assert!(pixels.pixels().any(|pixel| pixel.0[3] > 128));
            let last_row = (0..256)
                .rev()
                .find(|&y| (0..256).any(|x| pixels.get_pixel(x, y).0[3] > 128))
                .expect("visible");
            assert!(
                (230..=239).contains(&last_row),
                "baseline drift: {last_row}"
            );
        }
    }

    #[test]
    fn invalid_sheet_is_rejected_before_window_creation() {
        assert!(SpriteAtlas::from_png(b"not a png").is_err());
        let blank = image::RgbaImage::new(300, 300);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(blank)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(SpriteAtlas::from_png(bytes.get_ref()).is_err());
    }
}
