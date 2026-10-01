//! Opt-in fixtures/benchmarks. No camera, source deletion, or user settings.
#[cfg(test)]
mod tests {
    use crate::image_cache::{Key, decode};
    use std::path::PathBuf;
    use std::time::Instant;
    fn fixtures() -> PathBuf {
        let root = PathBuf::from("target/review-fixtures");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..8 {
            let path = root.join(format!("DSCF{i:04}.JPG"));
            if path.exists() {
                continue;
            }
            let photo = image::RgbImage::from_fn(7728, 5152, |x, y| {
                let square = if ((x + i * 7) / 40 + (y + i * 11) / 40) % 2 == 0 {
                    180
                } else {
                    30
                };
                let noise = ((x.wrapping_mul(1664525) ^ y.wrapping_mul(1013904223)) >> 20) % 12;
                image::Rgb([
                    (square + noise) as u8,
                    ((x / 32 + i * 9) % 220 + 20) as u8,
                    ((y / 24) % 210 + 30) as u8,
                ])
            });
            let photo = if i % 2 == 1 {
                image::imageops::blur(&photo, 1.6)
            } else {
                photo
            };
            let file = std::fs::File::create(&path).unwrap();
            image::codecs::jpeg::JpegEncoder::new_with_quality(file, 92)
                .encode_image(&photo)
                .unwrap();
            std::fs::write(
                root.join(format!("DSCF{i:04}.RAF")),
                format!("Synthetic RAW companion {i}"),
            )
            .unwrap();
        }
        std::fs::write(root.join("video.MOV"), b"Synthetic video companion").unwrap();
        root
    }
    #[test]
    #[ignore = "generates persistent 40 MP UI fixtures inside ignored target/"]
    fn generate_review_fixtures() {
        println!("fixtures: {}", fixtures().canonicalize().unwrap().display());
    }
    #[test]
    #[ignore = "run in release; measures generated JPEGs or MTP_CULL_BENCH_SOURCE"]
    fn decode_benchmark() {
        let root = std::env::var_os("MTP_CULL_BENCH_SOURCE")
            .map(PathBuf::from)
            .unwrap_or_else(fixtures);
        let paths = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("jpg"))
            })
            .take(12)
            .collect::<Vec<_>>();
        assert!(!paths.is_empty());
        println!(
            "decoder={} files={}",
            if cfg!(feature = "turbo") {
                "libjpeg-turbo SIMD"
            } else {
                "image/zune"
            },
            paths.len()
        );
        for (name, edge, analyze) in [
            ("thumbnail", Some(192), false),
            ("fit", Some(2304), false),
            ("native", None, false),
            ("native+focus", None, true),
        ] {
            let mut times = Vec::new();
            let mut sizes = Vec::new();
            for path in &paths {
                let start = Instant::now();
                let picture = decode(&Key {
                    path: path.clone(),
                    edge,
                    analyze,
                })
                .unwrap();
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                sizes.push(picture.image.size);
                std::hint::black_box(picture);
            }
            times.sort_by(f64::total_cmp);
            println!(
                "{name}: median={:.2}ms p95={:.2}ms max={:.2}ms output={:?}",
                times[times.len() / 2],
                times[(times.len() * 95 / 100).min(times.len() - 1)],
                times[times.len() - 1],
                sizes[0]
            );
        }
    }
}
