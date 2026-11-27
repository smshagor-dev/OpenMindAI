//! Bounded, in-memory rendering for project-owned research figures.
use crate::app_error::AppError;

const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u64 = 16_000_000;

pub(crate) struct RenderedFigure {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub(crate) fn render_png(
    mime: &str,
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<RenderedFigure, AppError> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width)
            .checked_mul(u64::from(height))
            .filter(|p| *p <= MAX_PIXELS)
            .is_none()
    {
        return Err(fail("Figure dimensions exceed the rendering limit."));
    }
    let (png, out_width, out_height) = match mime {
        "image/svg+xml" => {
            // usvg does not enable system fonts or external resource loading in
            // this build. A fresh database is populated only with bundled fonts.
            let mut options = resvg::usvg::Options::default();
            options.font_family = "DejaVu Sans".to_string();
            options
                .fontdb_mut()
                .load_font_data(include_bytes!("../assets/fonts/DejaVuSans-Regular.ttf").to_vec());
            options
                .fontdb_mut()
                .load_font_data(include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf").to_vec());
            options.fontdb_mut().set_sans_serif_family("DejaVu Sans");
            options.fontdb_mut().set_serif_family("DejaVu Sans");
            options.fontdb_mut().set_cursive_family("DejaVu Sans");
            options.fontdb_mut().set_fantasy_family("DejaVu Sans");
            options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
            options.image_href_resolver.resolve_string = Box::new(|_, _| None);
            let tree = resvg::usvg::Tree::from_data(bytes, &options)
                .map_err(|_| fail("SVG could not be rendered with the bundled font set or uses unsupported resources."))?;
            let size = tree.size();
            let factor = (MAX_SIDE as f32 / size.width())
                .min(MAX_SIDE as f32 / size.height())
                .min(1.0);
            let w = (size.width() * factor).ceil().max(1.0) as u32;
            let h = (size.height() * factor).ceil().max(1.0) as u32;
            if w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS {
                return Err(fail("Rendered SVG exceeds the pixel budget."));
            }
            let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
                .ok_or_else(|| fail("Could not allocate bounded SVG render buffer."))?;
            let transform = resvg::tiny_skia::Transform::from_scale(factor, factor);
            resvg::render(&tree, transform, &mut pixmap.as_mut());
            (
                pixmap
                    .encode_png()
                    .map_err(|_| fail("Could not encode rendered SVG as PNG."))?,
                w,
                h,
            )
        }
        "image/png" | "image/jpeg" => {
            let decoded = image::load_from_memory(bytes)
                .map_err(|_| fail("Registered raster figure is invalid."))?;
            if decoded.width() != width
                || decoded.height() != height
                || u64::from(width) * u64::from(height) > MAX_PIXELS
            {
                return Err(fail(
                    "Raster figure dimensions do not match the registered asset.",
                ));
            }
            let rgba = decoded.to_rgba8();
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(rgba)
                .write_to(&mut encoded, image::ImageFormat::Png)
                .map_err(|_| fail("Could not normalize raster figure to PNG."))?;
            (encoded.into_inner(), width, height)
        }
        _ => return Err(fail("Unsupported registered figure format.")),
    };
    Ok(RenderedFigure {
        png,
        width: out_width,
        height: out_height,
    })
}

fn fail(message: &str) -> AppError {
    AppError::ArtifactGenerationFailed(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_renders_with_bundled_text_fonts_and_transparent_pixels() {
        let plain = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32"><rect x="0" y="0" width="64" height="32" fill="#eeeeee"/></svg>"##;
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32"><rect x="0" y="0" width="64" height="32" fill="#eeeeee"/><text x="2" y="20" font-family="sans-serif" font-size="14">Label</text></svg>"##;
        let rendered = render_png("image/svg+xml", svg, 64, 32).unwrap();
        let plain = render_png("image/svg+xml", plain, 64, 32).unwrap();
        assert_eq!((rendered.width, rendered.height), (64, 32));
        let image = image::load_from_memory(&rendered.png).unwrap().to_rgba8();
        assert!(image.pixels().any(|p| p[3] > 0));
        let plain = image::load_from_memory(&plain.png).unwrap().to_rgba8();
        assert_ne!(
            image.as_raw(),
            plain.as_raw(),
            "SVG labels must render using the bundled fonts"
        );
        let transparent = render_png("image/svg+xml", br##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12"><circle cx="6" cy="6" r="3" fill="#000"/></svg>"##, 12, 12).unwrap();
        let transparent = image::load_from_memory(&transparent.png)
            .unwrap()
            .to_rgba8();
        assert!(transparent.pixels().any(|pixel| pixel[3] == 0));
    }
    #[test]
    fn rejects_invalid_and_over_budget_renders() {
        assert!(render_png("image/svg+xml", b"<svg", 1, 1).is_err());
        assert!(render_png("image/png", b"bad", 50_000, 50_000).is_err());
    }
    #[test]
    fn normalizes_png_and_jpeg_sources_to_valid_png() {
        let pixels = image::RgbaImage::from_fn(32, 16, |x, y| {
            image::Rgba([(x * 7) as u8, (y * 13) as u8, 120, 255])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels.clone())
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let rendered_png = render_png("image/png", &png.into_inner(), 32, 16).unwrap();
        assert_eq!((rendered_png.width, rendered_png.height), (32, 16));
        assert!(rendered_png.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(pixels).to_rgb8())
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let rendered_jpeg = render_png("image/jpeg", &jpeg.into_inner(), 32, 16).unwrap();
        assert_eq!((rendered_jpeg.width, rendered_jpeg.height), (32, 16));
        assert!(rendered_jpeg.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
