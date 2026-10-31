// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SVG to PNG, for Word's fallback image and for Google Docs.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, ImageHrefResolver, fontdb};

/// Largest raster edge in pixels, to bound memory.
const MAX_EDGE: f32 = 4_096.0;

fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            for data in typst_assets::fonts() {
                db.load_font_data(data.to_vec());
            }
            db.set_serif_family("Libertinus Serif");
            db.set_sans_serif_family("DejaVu Sans Mono");
            db.set_monospace_family("DejaVu Sans Mono");
            Arc::new(db)
        })
        .clone()
}

/// Renders an SVG to PNG at `scale` times its size. Returns `None` for an
/// SVG that does not parse or has no area. External references in the SVG
/// are never followed: only inline `data:` images are drawn.
#[must_use]
pub fn rasterize_svg(svg: &[u8], scale: f32) -> Option<Vec<u8>> {
    let options = usvg::Options {
        fontdb: fonts(),
        resources_dir: None,
        image_href_resolver: ImageHrefResolver {
            resolve_data: ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(svg, &options).ok()?;
    let size = tree.size();
    let scale = scale
        .min(MAX_EDGE / size.width().max(1.0))
        .min(MAX_EDGE / size.height().max(1.0));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // bounded by MAX_EDGE
    let (width, height) = (
        (size.width() * scale).ceil().max(1.0) as u32,
        (size.height() * scale).ceil().max(1.0) as u32,
    );
    let mut pixmap = Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_svg_to_png() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#4F5B92"/></svg>"##;
        let png = rasterize_svg(svg, 2.0).unwrap_or_default();
        assert!(png.starts_with(b"\x89PNG"));
        // IHDR width is 80 at scale 2.
        assert_eq!(png.get(16..20), Some(&80u32.to_be_bytes()[..]));
    }

    #[test]
    fn never_reads_files_named_by_the_svg() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="10" height="10" xlink:href="/etc/passwd"/></svg>"#;
        assert!(rasterize_svg(svg, 1.0).is_some());
    }

    #[test]
    fn rejects_non_svg() {
        assert!(rasterize_svg(b"not an svg", 1.0).is_none());
    }
}
