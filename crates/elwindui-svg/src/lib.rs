//! SVG loading — `usvg`-backed parsing/normalization into `elwindui_core::graphics::VectorImage`.
//! `elwindui-core` itself never depends on `usvg` or knows the SVG file format (SVG読み込み・ベク
//! ター描画対応 実装指示書§1.5); this crate is the only place that boundary is crossed.

mod convert;
mod convert_filter;
mod convert_paint;
mod convert_path;
mod error;
mod loader;
mod resolver;

pub use error::{SvgError, SvgFeature, SvgLimitKind, SvgWarning};
pub use loader::{SvgLimits, SvgLoadOptions, SvgLoadResult, SvgLoader};
pub use resolver::{
    ResolvedSvgResource, SvgBaseUri, SvgResourceError, SvgResourcePolicy, SvgResourceResolver,
};

use elwindui_core::graphics::VectorImage;
use std::path::Path;

/// Loads a `VectorImage` from an SVG file on disk using default [`SvgLoadOptions`] — a shorthand
/// for `SvgLoader::new(SvgLoadOptions::default()).load_file(path)`.
pub fn load_svg_file(path: impl AsRef<Path>) -> Result<VectorImage, SvgError> {
    SvgLoader::new(SvgLoadOptions::default()).load_file(path)
}

/// Loads a `VectorImage` from raw SVG (or gzip-compressed SVGZ) bytes using default
/// [`SvgLoadOptions`].
pub fn load_svg_bytes(bytes: &[u8]) -> Result<VectorImage, SvgError> {
    SvgLoader::new(SvgLoadOptions::default()).load_bytes(bytes)
}

/// Loads a `VectorImage` from SVG source text using default [`SvgLoadOptions`].
pub fn load_svg_str(source: &str) -> Result<VectorImage, SvgError> {
    SvgLoader::new(SvgLoadOptions::default()).load_str(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_RECT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100">
        <rect x="10" y="10" width="30" height="40" fill="#ff0000"/>
    </svg>"##;

    #[test]
    fn load_str_produces_a_vector_image() {
        let image = load_svg_str(SIMPLE_RECT_SVG).unwrap();
        assert_eq!(image.intrinsic_size().width, 100.0);
        assert_eq!(image.intrinsic_size().height, 100.0);
        assert!(!image.root().children.is_empty());
    }

    #[test]
    fn load_bytes_matches_load_str() {
        let image = load_svg_bytes(SIMPLE_RECT_SVG.as_bytes()).unwrap();
        assert_eq!(image.intrinsic_size().width, 100.0);
    }

    #[test]
    fn malformed_xml_is_a_parse_error() {
        let result = load_svg_str("<svg><rect></svg-broken");
        assert!(result.is_err());
    }

    #[test]
    fn loader_returns_vector_image_not_arc_wrapped() {
        // Compile-time assertion: `SvgLoader::load_str` must return `VectorImage`, not
        // `Arc<VectorImage>` (実装指示書§1.2/§12) — this line would fail to type-check otherwise.
        let loader = SvgLoader::new(SvgLoadOptions::default());
        let image: VectorImage = loader.load_str(SIMPLE_RECT_SVG).unwrap();
        let _clone: VectorImage = image.clone();
    }

    #[test]
    fn load_file_with_diagnostics_returns_warnings_slice() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("elwindui-svg-test-{}.svg", std::process::id()));
        std::fs::write(&path, SIMPLE_RECT_SVG).unwrap();
        let loader = SvgLoader::new(SvgLoadOptions::default());
        let result = loader.load_file_with_diagnostics(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.image.intrinsic_size().width, 100.0);
        let _warnings: &[SvgWarning] = &result.warnings;
    }

    #[test]
    fn external_file_reference_is_denied_by_default_policy() {
        let dir = std::env::temp_dir();
        let png_path = dir.join(format!("elwindui-svg-test-{}.png", std::process::id()));
        std::fs::write(&png_path, b"not a real png").unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="{}" width="10" height="10"/></svg>"#,
            png_path.display()
        );
        let loader = SvgLoader::new(SvgLoadOptions::default());
        // Default policy denies external file references outright — the SVG still parses (usvg
        // just drops the unresolvable <image>), it simply has no image content.
        let image = loader.load_str(&svg).unwrap();
        std::fs::remove_file(&png_path).unwrap();
        assert!(image.root().children.is_empty());
    }

    #[test]
    fn same_directory_policy_allows_sibling_file_and_denies_traversal() {
        let dir = std::env::temp_dir().join(format!("elwindui-svg-samedir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png_path = dir.join("sibling.png");
        // A minimal valid 1x1 PNG (enough for usvg's own format sniffing to accept it).
        std::fs::write(
            &png_path,
            [
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
                0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
                0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
                0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
            ],
        )
        .unwrap();
        let svg_path = dir.join("test.svg");
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="sibling.png" width="10" height="10"/></svg>"#;
        std::fs::write(&svg_path, svg).unwrap();

        let options = SvgLoadOptions {
            resource_policy: SvgResourcePolicy::SameDirectory,
            ..SvgLoadOptions::default()
        };
        let loader = SvgLoader::new(options);
        let image = loader.load_file(&svg_path).unwrap();
        assert!(!image.root().children.is_empty());

        let traversal_svg_path = dir.join("traversal.svg");
        let traversal_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="../../../../../etc/passwd" width="10" height="10"/></svg>"#;
        std::fs::write(&traversal_svg_path, traversal_svg).unwrap();
        let loader = SvgLoader::new(SvgLoadOptions {
            resource_policy: SvgResourcePolicy::SameDirectory,
            ..SvgLoadOptions::default()
        });
        let traversal_image = loader.load_file(&traversal_svg_path).unwrap();
        assert!(traversal_image.root().children.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn source_bytes_limit_is_enforced() {
        let big_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><!-- {} --></svg>"#,
            "x".repeat(1000)
        );
        let options = SvgLoadOptions {
            limits: SvgLimits {
                max_source_bytes: 100,
                ..SvgLimits::default()
            },
            ..SvgLoadOptions::default()
        };
        let loader = SvgLoader::new(options);
        let result = loader.load_bytes(big_svg.as_bytes());
        assert!(matches!(
            result,
            Err(SvgError::ResourceLimitExceeded {
                kind: SvgLimitKind::SourceBytes,
                ..
            })
        ));
    }

    #[test]
    fn max_nodes_limit_is_enforced() {
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">"#);
        for i in 0..50 {
            svg.push_str(&format!(r#"<rect x="{i}" y="{i}" width="1" height="1"/>"#));
        }
        svg.push_str("</svg>");

        let options = SvgLoadOptions {
            limits: SvgLimits {
                max_nodes: 10,
                ..SvgLimits::default()
            },
            ..SvgLoadOptions::default()
        };
        let result = SvgLoader::new(options).load_str(&svg);
        assert!(matches!(
            result,
            Err(SvgError::ResourceLimitExceeded {
                kind: SvgLimitKind::Nodes,
                ..
            })
        ));

        // The same document loads fine under a generous limit.
        let generous = SvgLoader::new(SvgLoadOptions::default()).load_str(&svg);
        assert!(generous.is_ok());
    }

    #[test]
    fn max_group_depth_limit_is_enforced() {
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#);
        for _ in 0..30 {
            svg.push_str(r#"<g transform="translate(0.01,0.01)">"#);
        }
        svg.push_str(r#"<rect width="1" height="1"/>"#);
        for _ in 0..30 {
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");

        let options = SvgLoadOptions {
            limits: SvgLimits {
                max_group_depth: 5,
                ..SvgLimits::default()
            },
            ..SvgLoadOptions::default()
        };
        let result = SvgLoader::new(options).load_str(&svg);
        assert!(matches!(
            result,
            Err(SvgError::ResourceLimitExceeded {
                kind: SvgLimitKind::GroupDepth,
                ..
            })
        ));
    }

    #[test]
    fn svgz_decompressed_bytes_limit_caps_a_decompression_bomb() {
        use std::io::Write;
        // A long run of a single repeated comment character compresses extremely well — a stand-in
        // for a real decompression-bomb payload without needing gigabytes of actual test fixture
        // data.
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><!--"#);
        svg.push_str(&"x".repeat(2_000_000));
        svg.push_str("--></svg>");

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(svg.as_bytes()).unwrap();
        let gzipped = encoder.finish().unwrap();
        assert!(gzipped.len() < 10_000, "fixture should compress far smaller than its decompressed size");

        let options = SvgLoadOptions {
            limits: SvgLimits {
                max_decompressed_bytes: 1_000_000,
                ..SvgLimits::default()
            },
            ..SvgLoadOptions::default()
        };
        let result = SvgLoader::new(options).load_bytes(&gzipped);
        assert!(matches!(
            result,
            Err(SvgError::ResourceLimitExceeded {
                kind: SvgLimitKind::DecompressedBytes,
                ..
            })
        ));

        // The same payload loads fine once decompressed size fits under the limit.
        let generous = SvgLoader::new(SvgLoadOptions::default()).load_bytes(&gzipped);
        assert!(generous.is_ok());
    }

    #[test]
    fn oversized_embedded_data_url_image_is_skipped_not_loaded() {
        use super::base64_stub::encode_base64;
        let payload = vec![0u8; 200_000];
        let encoded = encode_base64(&payload);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="data:image/png;base64,{encoded}" width="10" height="10"/></svg>"#
        );

        let options = SvgLoadOptions {
            limits: SvgLimits {
                max_embedded_image_bytes: 1_000,
                ..SvgLimits::default()
            },
            ..SvgLoadOptions::default()
        };
        // Oversized data URLs are dropped by usvg's own resolver returning `None` — the SVG still
        // parses successfully, just with no image content, rather than erroring the whole document.
        let image = SvgLoader::new(options).load_str(&svg).unwrap();
        assert!(image.root().children.is_empty());
    }

    #[test]
    fn non_utf8_bytes_is_a_parse_error() {
        let bytes: &[u8] = &[0xFF, 0xFE, 0x00, 0x01, 0x02];
        let result = SvgLoader::new(SvgLoadOptions::default()).load_bytes(bytes);
        assert!(matches!(result, Err(SvgError::Parse { .. })));
    }

    #[test]
    fn malformed_data_url_does_not_panic_and_yields_no_image() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="data:image/png;base64,!!!not-valid-base64!!!" width="10" height="10"/></svg>"#;
        let image = SvgLoader::new(SvgLoadOptions::default()).load_str(svg).unwrap();
        assert!(image.root().children.is_empty());
    }

    #[test]
    fn non_finite_geometry_does_not_panic() {
        // `NaN`/`Infinity` are not valid SVG path-data tokens; usvg either drops the malformed path
        // or the whole malformed element rather than propagating a non-finite coordinate — this
        // test's only real assertion is "loading this does not panic".
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path d="M NaN Infinity L 5 5"/></svg>"#;
        let _ = SvgLoader::new(SvgLoadOptions::default()).load_str(svg);
    }

    #[test]
    fn circular_same_directory_reference_is_bounded_by_nested_svg_depth() {
        let dir = std::env::temp_dir().join(format!("elwindui-svg-circular-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a_path = dir.join("a.svg");
        let b_path = dir.join("b.svg");
        std::fs::write(
            &a_path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="b.svg" width="10" height="10"/></svg>"#,
        )
        .unwrap();
        std::fs::write(
            &b_path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="a.svg" width="10" height="10"/></svg>"#,
        )
        .unwrap();

        let options = SvgLoadOptions {
            resource_policy: SvgResourcePolicy::SameDirectory,
            limits: SvgLimits {
                max_nested_svg_depth: 4,
                ..SvgLimits::default()
            },
            ..SvgLoadOptions::default()
        };
        // The point of this test is that loading terminates at all (a naive implementation would
        // recurse a→b→a→b… until the process stack overflows) — succeeding with the cycle broken
        // off by the depth cap, rather than any particular resulting tree shape.
        let result = SvgLoader::new(options).load_file(&a_path);
        assert!(result.is_ok());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A tiny, dependency-free base64 encoder for one test fixture above — pulling in a real `base64`
/// crate for a single oversized-payload test isn't worth a new dependency.
#[cfg(test)]
mod base64_stub {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode_base64(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0];
            let b1 = *chunk.get(1).unwrap_or(&0);
            let b2 = *chunk.get(2).unwrap_or(&0);
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(b2 & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }
}
