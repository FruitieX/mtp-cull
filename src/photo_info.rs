//! Capture details shown next to the open photo's name.
use chrono::NaiveDateTime;
use exif::{Exif, In, Tag, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhotoInfo {
    pub time: Option<NaiveDateTime>,
    pub exposure: Option<f64>,
    pub aperture: Option<f64>,
    pub iso: Option<u32>,
    pub focal_length: Option<f64>,
    pub focal_length_35mm: Option<u32>,
    pub exposure_bias: Option<f64>,
    pub lens: Option<String>,
}

impl PhotoInfo {
    pub fn parse(bytes: Option<Vec<u8>>) -> Self {
        bytes
            .and_then(|bytes| exif::Reader::new().read_raw(bytes).ok())
            .map_or_else(Self::default, |exif| Self::from_exif(&exif))
    }

    fn from_exif(exif: &Exif) -> Self {
        let field = |tag| exif.get_field(tag, In::PRIMARY).map(|f| &f.value);
        let text = |tag| match field(tag)? {
            Value::Ascii(values) => {
                let text = std::str::from_utf8(values.first()?).ok()?;
                let text = text.trim_end_matches('\0').trim();
                (!text.is_empty()).then(|| text.to_owned())
            }
            _ => None,
        };
        let number = |tag| match field(tag)? {
            Value::Rational(v) => v.first().filter(|r| r.denom != 0).map(|r| r.to_f64()),
            Value::SRational(v) => v.first().filter(|r| r.denom != 0).map(|r| r.to_f64()),
            _ => None,
        };
        let uint = |tag| field(tag)?.get_uint(0).filter(|&v| v > 0);
        Self {
            time: text(Tag::DateTimeOriginal)
                .or_else(|| text(Tag::DateTime))
                .and_then(|t| NaiveDateTime::parse_from_str(&t, "%Y:%m:%d %H:%M:%S").ok()),
            exposure: number(Tag::ExposureTime).filter(|&t| t > 0.0),
            aperture: number(Tag::FNumber).filter(|&f| f > 0.0),
            iso: uint(Tag::PhotographicSensitivity),
            focal_length: number(Tag::FocalLength).filter(|&f| f > 0.0),
            focal_length_35mm: uint(Tag::FocalLengthIn35mmFilm),
            exposure_bias: number(Tag::ExposureBiasValue),
            lens: text(Tag::LensModel),
        }
    }

    /// One line such as `2026-10-12 14:03:22 · 1/250 s · f/2.8 · ISO 400 · 23 mm`.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(time) = self.time {
            parts.push(time.format("%Y-%m-%d %H:%M:%S").to_string());
        }
        if let Some(t) = self.exposure {
            parts.push(if t >= 0.5 {
                format!("{} s", trim(t, 1))
            } else {
                format!("1/{} s", (1.0 / t).round())
            });
        }
        if let Some(f) = self.aperture {
            parts.push(format!("f/{}", trim(f, 1)));
        }
        if let Some(iso) = self.iso {
            parts.push(format!("ISO {iso}"));
        }
        if let Some(focal) = self.focal_length {
            let focal = focal.round() as u32;
            parts.push(match self.focal_length_35mm {
                Some(full) if full != focal => format!("{focal} mm ({full} mm eq.)"),
                _ => format!("{focal} mm"),
            });
        }
        if let Some(ev) = self.exposure_bias
            && ev.abs() >= 0.05
        {
            parts.push(format!("{ev:+.1} EV"));
        }
        if let Some(lens) = &self.lens {
            parts.push(lens.clone());
        }
        parts.join(" · ")
    }
}

fn trim(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_formats_common_camera_values() {
        let info = PhotoInfo {
            time: NaiveDateTime::parse_from_str("2026:10:12 14:03:22", "%Y:%m:%d %H:%M:%S").ok(),
            exposure: Some(1.0 / 250.0),
            aperture: Some(2.8),
            iso: Some(400),
            focal_length: Some(23.0),
            focal_length_35mm: Some(35),
            exposure_bias: Some(2.0 / 3.0),
            lens: Some("XF23mmF2 R WR".into()),
        };
        assert_eq!(
            info.summary(),
            "2026-10-12 14:03:22 · 1/250 s · f/2.8 · ISO 400 · 23 mm (35 mm eq.) · +0.7 EV · XF23mmF2 R WR"
        );
        let long = PhotoInfo {
            exposure: Some(2.5),
            aperture: Some(8.0),
            exposure_bias: Some(0.0),
            ..Default::default()
        };
        assert_eq!(long.summary(), "2.5 s · f/8");
        assert_eq!(PhotoInfo::default().summary(), "");
        assert_eq!(PhotoInfo::parse(None), PhotoInfo::default());
    }
}
