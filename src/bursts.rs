use chrono::NaiveDateTime;
use image::DynamicImage;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Default)]
pub struct Feature {
    pub time: Option<NaiveDateTime>,
    pub hash: u64,
}
pub fn capture_time(bytes: Option<Vec<u8>>) -> Option<NaiveDateTime> {
    let exif = exif::Reader::new().read_raw(bytes?).ok()?;
    let value = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;
    let exif::Value::Ascii(values) = &value.value else {
        return None;
    };
    let text = std::str::from_utf8(values.first()?)
        .ok()?
        .trim_end_matches('\0');
    NaiveDateTime::parse_from_str(text, "%Y:%m:%d %H:%M:%S").ok()
}
pub fn similarity(image: &DynamicImage) -> u64 {
    let tiny = image
        .resize_exact(9, 8, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut hash = 0;
    for y in 0..8 {
        for x in 0..8 {
            hash = (hash << 1) | u64::from(tiny.get_pixel(x, y)[0] > tiny.get_pixel(x + 1, y)[0]);
        }
    }
    hash
}
/// Conservative adjacent capture-time grouping. Missing timestamps never group.
pub fn group(
    shots: &[crate::review::Shot],
    features: &HashMap<PathBuf, Feature>,
    window_ms: i64,
    distance: u32,
) -> HashMap<usize, u32> {
    let mut ordered = shots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let p = s.preview()?;
            let f = features.get(p)?;
            Some((i, p, f, f.time?))
        })
        .collect::<Vec<_>>();
    ordered.sort_by_key(|s| s.3);
    let mut groups = HashMap::new();
    let mut id = 0;
    for pair in ordered.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.1.parent() != b.1.parent()
            || (b.3 - a.3).num_milliseconds() > window_ms
            || (a.2.hash ^ b.2.hash).count_ones() > distance
        {
            continue;
        }
        let group = *groups.entry(a.0).or_insert_with(|| {
            id += 1;
            id
        });
        groups.insert(b.0, group);
    }
    groups
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouping_requires_time_similarity_and_same_folder() {
        use crate::review::{Asset, Decision, Kind, Shot};
        let time =
            NaiveDateTime::parse_from_str("2026:09:30 12:00:00", "%Y:%m:%d %H:%M:%S").unwrap();
        let mut features = HashMap::new();
        let shots = (0..4)
            .map(|i| {
                let path = PathBuf::from(format!("camera/{i}.jpg"));
                features.insert(
                    path.clone(),
                    Feature {
                        time: Some(time + chrono::Duration::seconds(if i < 3 { i } else { 10 })),
                        hash: if i == 2 { u64::MAX } else { 0 },
                    },
                );
                Shot {
                    id: i.to_string(),
                    name: i.to_string(),
                    assets: vec![Asset {
                        id: i.to_string(),
                        key: i.to_string(),
                        name: i.to_string(),
                        kind: Kind::Jpeg,
                        size: 1,
                        path: Some(path),
                        decision: Decision::Unreviewed,
                    }],
                }
            })
            .collect::<Vec<_>>();
        let groups = group(&shots, &features, 2000, 8);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[&0], groups[&1]);
        assert!(!groups.contains_key(&2));
        assert!(!groups.contains_key(&3));
    }
}
