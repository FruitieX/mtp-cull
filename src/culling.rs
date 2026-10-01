use blake3::Hash;
use chrono::{NaiveDate, NaiveDateTime};
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use directories::ProjectDirs;
use eframe::egui;
use exif::{DateTime as ExifDateTime, In, Reader, Tag, Value};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Unrated,
    Reject,
    Keep,
}

impl Decision {
    fn as_i64(self) -> i64 {
        match self {
            Self::Unrated => 0,
            Self::Reject => 1,
            Self::Keep => 2,
        }
    }

    fn from_i64(value: i64) -> Result<Self> {
        match value {
            0 => Ok(Self::Unrated),
            1 => Ok(Self::Reject),
            2 => Ok(Self::Keep),
            _ => Err(eyre!("invalid culling decision {value}")),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unrated => "Unrated",
            Self::Reject => "Reject",
            Self::Keep => "Keep",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Jpeg,
    Heif,
    Raw,
    Video,
}

impl MediaKind {
    fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "heic" | "heif" => Some(Self::Heif),
            "dng" | "raf" | "raw" | "crw" | "cr2" | "cr3" | "arw" | "srf" | "sr2" | "rw2"
            | "nef" | "nrw" => Some(Self::Raw),
            "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" => Some(Self::Video),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Heif => "HEIF",
            Self::Raw => "RAW",
            Self::Video => "Video",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub capture_time: Option<NaiveDateTime>,
    pub orientation: Option<u16>,
    pub sharpness: Option<f64>,
    pub similarity_hash: Option<u64>,
    fingerprint: Hash,
}

impl Asset {
    fn fingerprint(&self) -> &[u8] {
        self.fingerprint.as_bytes()
    }
}

#[derive(Clone, Debug)]
pub struct Shot {
    pub stem: String,
    pub assets: Vec<Asset>,
    pub decision: Decision,
    pub burst: Option<BurstMembership>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BurstMembership {
    pub id: u32,
    pub distance_to_previous: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct BurstSettings {
    pub window_seconds: f64,
    pub similarity_threshold: u32,
}

impl Default for BurstSettings {
    fn default() -> Self {
        Self {
            window_seconds: 2.0,
            similarity_threshold: 8,
        }
    }
}

impl Shot {
    pub fn jpeg(&self) -> Option<&Asset> {
        self.assets
            .iter()
            .find(|asset| asset.kind == MediaKind::Jpeg)
    }

    pub fn has_conflict(&self) -> bool {
        self.assets
            .iter()
            .filter(|asset| asset.kind == MediaKind::Jpeg)
            .count()
            > 1
            || self
                .assets
                .iter()
                .filter(|asset| asset.kind == MediaKind::Raw)
                .count()
                > 1
    }

    pub fn sharpness(&self) -> Option<f64> {
        self.jpeg().and_then(|asset| asset.sharpness)
    }

    pub fn capture_time(&self) -> Option<NaiveDateTime> {
        self.assets.iter().find_map(|asset| asset.capture_time)
    }
}

#[derive(Debug)]
pub struct Session {
    pub primary_directory: PathBuf,
    pub raw_directory: Option<PathBuf>,
    pub shots: Vec<Shot>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Action {
    Previous,
    Next,
    NextUnrated,
    Reject,
    Keep,
    Unrated,
    ToggleZoom,
    ToggleCompare,
}

impl Action {
    pub const ALL: [Self; 8] = [
        Self::Previous,
        Self::Next,
        Self::NextUnrated,
        Self::Reject,
        Self::Keep,
        Self::Unrated,
        Self::ToggleZoom,
        Self::ToggleCompare,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Previous => "previous",
            Self::Next => "next",
            Self::NextUnrated => "next_unrated",
            Self::Reject => "reject",
            Self::Keep => "keep",
            Self::Unrated => "unrated",
            Self::ToggleZoom => "toggle_zoom",
            Self::ToggleCompare => "toggle_compare",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Previous => "Previous shot",
            Self::Next => "Next shot",
            Self::NextUnrated => "Next unrated shot",
            Self::Reject => "Reject",
            Self::Keep => "Keep",
            Self::Unrated => "Unrated",
            Self::ToggleZoom => "Fit / 100%",
            Self::ToggleCompare => "A/B alternate",
        }
    }

    pub fn default_key(self) -> &'static str {
        match self {
            Self::Previous => "ArrowLeft",
            Self::Next => "ArrowRight",
            Self::NextUnrated => "Tab",
            Self::Reject => "1",
            Self::Keep => "2",
            Self::Unrated => "0",
            Self::ToggleZoom => "Z",
            Self::ToggleCompare => "C",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Database {
    path: PathBuf,
}

impl Database {
    pub fn open() -> Result<Self> {
        let directories = ProjectDirs::from("com", "fruit", "mtp-cull")
            .ok_or_else(|| eyre!("could not determine the application data directory"))?;
        let directory = directories.data_local_dir();
        std::fs::create_dir_all(directory)
            .wrap_err_with(|| format!("failed to create {}", directory.display()))?;
        Self::at(directory.join("cull.sqlite3"))
    }

    fn at(path: PathBuf) -> Result<Self> {
        let database = Self { path };
        database.connection()?;
        Ok(database)
    }

    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)
            .wrap_err_with(|| format!("failed to open {}", self.path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS decisions (
                fingerprint BLOB PRIMARY KEY NOT NULL,
                decision INTEGER NOT NULL CHECK (decision IN (0, 1, 2)),
                updated_at INTEGER NOT NULL
            ) STRICT;
            CREATE TABLE IF NOT EXISTS keybindings (
                action TEXT PRIMARY KEY NOT NULL,
                key_name TEXT NOT NULL
            ) STRICT;
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            ) STRICT;
            CREATE TABLE IF NOT EXISTS analysis (
                fingerprint BLOB PRIMARY KEY NOT NULL,
                capture_at TEXT,
                orientation INTEGER,
                sharpness REAL,
                similarity INTEGER,
                algorithm TEXT NOT NULL
            ) STRICT;
            CREATE TABLE IF NOT EXISTS file_fingerprints (
                path TEXT PRIMARY KEY NOT NULL,
                file_size INTEGER NOT NULL,
                modified_at INTEGER,
                fingerprint BLOB NOT NULL
            ) STRICT;
            ",
        )?;
        if !has_column(&connection, "analysis", "similarity")? {
            connection.execute("ALTER TABLE analysis ADD COLUMN similarity INTEGER", [])?;
        }
        Ok(connection)
    }

    pub fn decision_for(&self, fingerprints: impl Iterator<Item = Hash>) -> Result<Decision> {
        let connection = self.connection()?;
        let mut decision = None;
        for fingerprint in fingerprints {
            let stored = connection
                .query_row(
                    "SELECT decision FROM decisions WHERE fingerprint = ?1",
                    [fingerprint.as_bytes()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .map(Decision::from_i64)
                .transpose()?;
            match (decision, stored) {
                (_, None) => return Ok(Decision::Unrated),
                (None, Some(value)) => decision = Some(value),
                (Some(previous), Some(value)) if previous == value => {}
                (Some(_), Some(_)) => return Ok(Decision::Unrated),
            }
        }
        Ok(decision.unwrap_or(Decision::Unrated))
    }

    pub fn set_decision(&self, shot: &Shot, decision: Decision) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        for asset in &shot.assets {
            transaction.execute(
                "
                INSERT INTO decisions (fingerprint, decision, updated_at)
                VALUES (?1, ?2, unixepoch())
                ON CONFLICT(fingerprint) DO UPDATE SET
                    decision = excluded.decision,
                    updated_at = excluded.updated_at
                ",
                params![asset.fingerprint(), decision.as_i64()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Return a content fingerprint, reusing it when the file's cheap filesystem
    /// identity is unchanged. This keeps reopening a large album fast without
    /// weakening the content check used before recycling files.
    pub fn fingerprint_for_path(&self, path: &Path) -> Result<Hash> {
        let metadata = std::fs::metadata(path)
            .wrap_err_with(|| format!("failed to stat {}", path.display()))?;
        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_nanos()).ok());
        let file_size = i64::try_from(metadata.len())
            .wrap_err_with(|| format!("file is too large to index: {}", path.display()))?;
        let path_text = path.to_string_lossy().into_owned();

        let connection = self.connection()?;
        let cached = connection
            .query_row(
                "SELECT file_size, modified_at, fingerprint FROM file_fingerprints WHERE path = ?1",
                [path_text.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((cached_size, cached_modified, bytes)) = cached
            && cached_size == file_size
            && cached_modified == modified_at
            && modified_at.is_some()
            && bytes.len() == blake3::OUT_LEN
        {
            let mut fingerprint = [0_u8; blake3::OUT_LEN];
            fingerprint.copy_from_slice(&bytes);
            return Ok(Hash::from_bytes(fingerprint));
        }

        let fingerprint = fingerprint_file(path)?;
        connection.execute(
            "
            INSERT INTO file_fingerprints (path, file_size, modified_at, fingerprint)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(path) DO UPDATE SET
                file_size = excluded.file_size,
                modified_at = excluded.modified_at,
                fingerprint = excluded.fingerprint
            ",
            params![path_text, file_size, modified_at, fingerprint.as_bytes()],
        )?;
        Ok(fingerprint)
    }

    pub fn keybindings(&self) -> Result<BTreeMap<Action, String>> {
        let connection = self.connection()?;
        let mut bindings = Action::ALL
            .into_iter()
            .map(|action| (action, action.default_key().to_owned()))
            .collect::<BTreeMap<_, _>>();
        let mut statement = connection.prepare("SELECT action, key_name FROM keybindings")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (action, key) = row?;
            if let Some(action) = Action::ALL.into_iter().find(|item| item.id() == action) {
                bindings.insert(action, key);
            }
        }
        Ok(bindings)
    }

    pub fn set_keybinding(&self, action: Action, key: &str) -> Result<()> {
        self.connection()?.execute(
            "
            INSERT INTO keybindings (action, key_name) VALUES (?1, ?2)
            ON CONFLICT(action) DO UPDATE SET key_name = excluded.key_name
            ",
            params![action.id(), key],
        )?;
        Ok(())
    }

    pub fn burst_settings(&self) -> Result<BurstSettings> {
        let connection = self.connection()?;
        let window_seconds = setting_value(&connection, "burst_window_seconds")?
            .and_then(|value| value.parse().ok())
            .filter(|value: &f64| (0.1..=60.0).contains(value))
            .unwrap_or_else(|| BurstSettings::default().window_seconds);
        let similarity_threshold = setting_value(&connection, "burst_similarity_threshold")?
            .and_then(|value| value.parse().ok())
            .filter(|value: &u32| *value <= 64)
            .unwrap_or_else(|| BurstSettings::default().similarity_threshold);
        Ok(BurstSettings {
            window_seconds,
            similarity_threshold,
        })
    }

    pub fn set_burst_settings(&self, settings: BurstSettings) -> Result<()> {
        let connection = self.connection()?;
        for (key, value) in [
            ("burst_window_seconds", settings.window_seconds.to_string()),
            (
                "burst_similarity_threshold",
                settings.similarity_threshold.to_string(),
            ),
        ] {
            connection.execute(
                "
                INSERT INTO settings (key, value) VALUES (?1, ?2)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value
                ",
                params![key, value],
            )?;
        }
        Ok(())
    }

    fn analysis_for(&self, fingerprint: Hash) -> Result<Option<Analysis>> {
        self.connection()?
            .query_row(
                "SELECT capture_at, orientation, sharpness, similarity FROM analysis WHERE fingerprint = ?1 AND algorithm = 'exif-v1-tenengrad-v1'",
                [fingerprint.as_bytes()],
                |row| {
                    let capture_at = row.get::<_, Option<String>>(0)?;
                    let capture_time = capture_at
                        .as_deref()
                        .map(|value| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
                        .transpose()
                        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error)))?;
                    Ok(Analysis {
                        capture_time,
                        orientation: row.get::<_, Option<u16>>(1)?,
                        sharpness: row.get::<_, Option<f64>>(2)?,
                        similarity_hash: row.get::<_, Option<i64>>(3)?.map(|hash| hash as u64),
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn store_analysis(&self, fingerprint: Hash, analysis: &Analysis) -> Result<()> {
        self.connection()?.execute(
            "
            INSERT INTO analysis (fingerprint, capture_at, orientation, sharpness, similarity, algorithm)
            VALUES (?1, ?2, ?3, ?4, ?5, 'exif-v1-tenengrad-v1')
            ON CONFLICT(fingerprint) DO UPDATE SET
                capture_at = excluded.capture_at,
                orientation = excluded.orientation,
                sharpness = excluded.sharpness,
                similarity = excluded.similarity,
                algorithm = excluded.algorithm
            ",
            params![
                fingerprint.as_bytes(),
                analysis
                    .capture_time
                    .map(|time| time.format("%Y-%m-%dT%H:%M:%S").to_string()),
                analysis.orientation,
                analysis.sharpness,
                analysis.similarity_hash.map(|hash| hash as i64),
            ],
        )?;
        Ok(())
    }
}

struct Analysis {
    capture_time: Option<NaiveDateTime>,
    orientation: Option<u16>,
    sharpness: Option<f64>,
    similarity_hash: Option<u64>,
}

pub fn load_session(
    database: &Database,
    primary_directory: PathBuf,
    raw_directory: Option<PathBuf>,
) -> Result<Session> {
    let mut grouped = BTreeMap::<String, Vec<(PathBuf, MediaKind)>>::new();
    let mut directories = vec![primary_directory.clone()];
    if let Some(raw_directory) = &raw_directory
        && raw_directory != &primary_directory
    {
        directories.push(raw_directory.clone());
    }

    for directory in directories {
        for entry in std::fs::read_dir(&directory)
            .wrap_err_with(|| format!("failed to read {}", directory.display()))?
        {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            let Some(kind) = MediaKind::from_path(&path) else {
                continue;
            };
            let Some(stem) = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
            else {
                continue;
            };
            grouped.entry(stem).or_default().push((path, kind));
        }
    }

    let mut shots = Vec::with_capacity(grouped.len());
    for (stem, mut files) in grouped {
        files.sort_by(|left, right| left.0.cmp(&right.0));
        let assets = files
            .into_iter()
            .map(|(path, kind)| {
                let fingerprint = database.fingerprint_for_path(&path)?;
                let analysis = match database.analysis_for(fingerprint)? {
                    Some(analysis) => analysis,
                    None => {
                        let analysis = analyze(&path, kind);
                        database.store_analysis(fingerprint, &analysis)?;
                        analysis
                    }
                };
                Ok(Asset {
                    capture_time: analysis.capture_time,
                    orientation: analysis.orientation,
                    sharpness: analysis.sharpness,
                    similarity_hash: analysis.similarity_hash,
                    path,
                    kind,
                    fingerprint,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let decision = database.decision_for(assets.iter().map(|asset| asset.fingerprint))?;
        shots.push(Shot {
            stem,
            assets,
            decision,
            burst: None,
        });
    }
    sort_by_capture_time(&mut shots);
    apply_burst_groups(&mut shots, BurstSettings::default());

    Ok(Session {
        primary_directory,
        raw_directory,
        shots,
    })
}

pub fn apply_burst_groups(shots: &mut [Shot], settings: BurstSettings) {
    let mut next_burst_id = 1;
    for shot in &mut *shots {
        shot.burst = None;
    }
    for index in 1..shots.len() {
        let previous = &shots[index - 1];
        let current = &shots[index];
        let Some(previous_time) = previous.capture_time() else {
            continue;
        };
        let Some(current_time) = current.capture_time() else {
            continue;
        };
        let Some(previous_hash) = previous.jpeg().and_then(|asset| asset.similarity_hash) else {
            continue;
        };
        let Some(current_hash) = current.jpeg().and_then(|asset| asset.similarity_hash) else {
            continue;
        };
        let elapsed = current_time
            .signed_duration_since(previous_time)
            .num_milliseconds()
            .abs() as f64
            / 1_000.0;
        let distance = (current_hash ^ previous_hash).count_ones();
        if elapsed > settings.window_seconds || distance > settings.similarity_threshold {
            continue;
        }
        let id = previous.burst.map_or_else(
            || {
                let id = next_burst_id;
                next_burst_id += 1;
                id
            },
            |membership| membership.id,
        );
        if shots[index - 1].burst.is_none() {
            shots[index - 1].burst = Some(BurstMembership {
                id,
                distance_to_previous: 0,
            });
        }
        shots[index].burst = Some(BurstMembership {
            id,
            distance_to_previous: distance,
        });
    }
}

pub fn sort_by_capture_time(shots: &mut [Shot]) {
    shots.sort_by(
        |left, right| match (left.capture_time(), right.capture_time()) {
            (Some(left_time), Some(right_time)) => {
                left_time.cmp(&right_time).then(left.stem.cmp(&right.stem))
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left.stem.cmp(&right.stem),
        },
    );
}

pub fn apply_rejects(session: &Session) -> Result<usize> {
    let rejected = session
        .shots
        .iter()
        .filter(|shot| shot.decision == Decision::Reject)
        .flat_map(|shot| &shot.assets)
        .collect::<Vec<_>>();
    for asset in &rejected {
        if fingerprint_file(&asset.path)? != asset.fingerprint {
            bail!(
                "{} changed after this session was loaded; reload before recycling",
                asset.path.display()
            );
        }
    }
    let paths = rejected.iter().map(|asset| &asset.path).collect::<Vec<_>>();
    if !paths.is_empty() {
        trash::delete_all(paths).map_err(|error| eyre!(error))?;
    }
    Ok(rejected.len())
}

fn fingerprint_file(path: &Path) -> Result<Hash> {
    let file = File::open(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file)?;
    Ok(hasher.finalize())
}

fn analyze(path: &Path, kind: MediaKind) -> Analysis {
    let (capture_time, orientation) = exif_metadata(path).unwrap_or((None, None));
    let jpeg_analysis = (kind == MediaKind::Jpeg)
        .then(|| analyze_jpeg(path, orientation).ok())
        .flatten();
    Analysis {
        capture_time,
        orientation,
        sharpness: jpeg_analysis.map(|analysis| analysis.0),
        similarity_hash: jpeg_analysis.map(|analysis| analysis.1),
    }
}

fn exif_metadata(path: &Path) -> Result<(Option<NaiveDateTime>, Option<u16>)> {
    let file = File::open(path)?;
    let exif = Reader::new().read_from_container(&mut std::io::BufReader::new(file))?;
    let capture_time = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .into_iter()
        .find_map(|tag| exif.get_field(tag, In::PRIMARY))
        .and_then(|field| match &field.value {
            Value::Ascii(values) => values.first(),
            _ => None,
        })
        .and_then(|value| ExifDateTime::from_ascii(value).ok())
        .and_then(exif_datetime_to_chrono);
    let orientation = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (1..=8).contains(value));
    Ok((capture_time, orientation))
}

fn exif_datetime_to_chrono(datetime: ExifDateTime) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(
        datetime.year.into(),
        datetime.month.into(),
        datetime.day.into(),
    )?
    .and_hms_nano_opt(
        datetime.hour.into(),
        datetime.minute.into(),
        datetime.second.into(),
        datetime.nanosecond.unwrap_or(0),
    )
}

pub fn decode_jpeg_preview(
    path: &Path,
    orientation: Option<u16>,
    max_edge: Option<u32>,
) -> Result<egui::ColorImage> {
    let image = decode_jpeg(path, orientation)?;
    let image = match max_edge {
        Some(edge) => image.thumbnail(edge, edge),
        None => image,
    };
    let rgba = image.into_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

fn analyze_jpeg(path: &Path, orientation: Option<u16>) -> Result<(f64, u64)> {
    let image = decode_jpeg(path, orientation)?;
    let grayscale = image.thumbnail(1_024, 1_024).to_luma8();
    let width = grayscale.width();
    let height = grayscale.height();
    if width < 3 || height < 3 {
        return Ok((0.0, difference_hash(&image)));
    }

    let mut energy = 0.0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let left = grayscale.get_pixel(x - 1, y)[0] as f64;
            let right = grayscale.get_pixel(x + 1, y)[0] as f64;
            let top = grayscale.get_pixel(x, y - 1)[0] as f64;
            let bottom = grayscale.get_pixel(x, y + 1)[0] as f64;
            energy += (right - left).mul_add(right - left, (bottom - top) * (bottom - top));
        }
    }
    Ok((
        energy / ((width - 2) * (height - 2)) as f64,
        difference_hash(&image),
    ))
}

fn decode_jpeg(path: &Path, orientation: Option<u16>) -> Result<image::DynamicImage> {
    use image::ImageDecoder;

    let mut decoder = image::ImageReader::open(path)?.into_decoder()?;
    let embedded_orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    if let Some(orientation) = orientation
        .and_then(|value| u8::try_from(value).ok())
        .and_then(image::metadata::Orientation::from_exif)
    {
        image.apply_orientation(orientation);
    } else {
        image.apply_orientation(embedded_orientation);
    }
    Ok(image)
}

fn difference_hash(image: &image::DynamicImage) -> u64 {
    let grayscale = image.thumbnail_exact(9, 8).into_luma8();
    let mut hash = 0_u64;
    for y in 0..8 {
        for x in 0..8 {
            hash = (hash << 1)
                | u64::from(grayscale.get_pixel(x, y)[0] > grayscale.get_pixel(x + 1, y)[0]);
        }
    }
    hash
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for name in columns {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn setting_value(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{
        Analysis, Asset, BurstSettings, Database, Decision, MediaKind, Shot, apply_burst_groups,
        decode_jpeg_preview, load_session,
    };
    use chrono::{NaiveDate, NaiveDateTime};
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb};
    use std::fs;

    #[test]
    fn groups_jpeg_and_raw_by_exact_stem() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("DSCF0001.JPG"), b"jpeg").unwrap();
        fs::write(directory.path().join("DSCF0001.RAF"), b"raw").unwrap();
        fs::write(directory.path().join("DSCF0002.JPG"), b"jpeg2").unwrap();
        let database = Database::at(directory.path().join("cull.sqlite3")).unwrap();

        let session = load_session(&database, directory.path().to_owned(), None).unwrap();

        assert_eq!(session.shots.len(), 2);
        assert_eq!(session.shots[0].assets.len(), 2);
        assert_eq!(session.shots[0].assets[0].kind, MediaKind::Jpeg);
        assert_eq!(session.shots[0].assets[1].kind, MediaKind::Raw);
    }

    #[test]
    fn persists_decisions_by_content_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("DSCF0001.JPG");
        fs::write(&path, b"jpeg").unwrap();
        let database = Database::at(directory.path().join("cull.sqlite3")).unwrap();
        let session = load_session(&database, directory.path().to_owned(), None).unwrap();
        database
            .set_decision(&session.shots[0], Decision::Keep)
            .unwrap();
        fs::rename(&path, directory.path().join("renamed.JPG")).unwrap();

        let reloaded = load_session(&database, directory.path().to_owned(), None).unwrap();
        assert_eq!(reloaded.shots[0].decision, Decision::Keep);
    }

    #[test]
    fn caches_fingerprint_for_unchanged_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("photo.JPG");
        fs::write(&path, b"photo bytes").unwrap();
        let database = Database::at(directory.path().join("cull.sqlite3")).unwrap();

        let first = database.fingerprint_for_path(&path).unwrap();
        let second = database.fingerprint_for_path(&path).unwrap();

        assert_eq!(first, second);
        let connection = database.connection().unwrap();
        let cached: Vec<u8> = connection
            .query_row(
                "SELECT fingerprint FROM file_fingerprints WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cached, first.as_bytes());
    }

    #[test]
    fn persists_capture_metadata_and_sharpness_by_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::at(directory.path().join("cull.sqlite3")).unwrap();
        let fingerprint = blake3::hash(b"photo");
        let capture_time = NaiveDate::from_ymd_opt(2026, 7, 11)
            .unwrap()
            .and_hms_opt(12, 30, 45)
            .unwrap();

        database
            .store_analysis(
                fingerprint,
                &Analysis {
                    capture_time: Some(capture_time),
                    orientation: Some(6),
                    sharpness: Some(123.4),
                    similarity_hash: Some(42),
                },
            )
            .unwrap();
        let analysis = database.analysis_for(fingerprint).unwrap().unwrap();

        assert_eq!(analysis.capture_time, Some(capture_time));
        assert_eq!(analysis.orientation, Some(6));
        assert_eq!(analysis.sharpness, Some(123.4));
    }

    #[test]
    fn applies_exif_orientation_to_preview_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("portrait.jpg");
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(2, 3, Rgb([10, 20, 30])));
        image.save_with_format(&path, ImageFormat::Jpeg).unwrap();

        let preview = decode_jpeg_preview(&path, Some(6), None).unwrap();

        assert_eq!(preview.size, [3, 2]);
    }

    #[test]
    fn groups_only_similar_adjacent_shots_inside_the_time_window() {
        let capture = NaiveDate::from_ymd_opt(2026, 7, 11)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let mut shots = vec![
            shot_with_hash("one", capture, 0),
            shot_with_hash("two", capture + chrono::Duration::seconds(1), 0b11),
            shot_with_hash("three", capture + chrono::Duration::seconds(2), u64::MAX),
            shot_with_hash("four", capture + chrono::Duration::seconds(10), 0),
        ];

        apply_burst_groups(
            &mut shots,
            BurstSettings {
                window_seconds: 2.0,
                similarity_threshold: 3,
            },
        );

        assert_eq!(shots[0].burst.unwrap().id, shots[1].burst.unwrap().id);
        assert_eq!(shots[1].burst.unwrap().distance_to_previous, 2);
        assert!(shots[2].burst.is_none());
        assert!(shots[3].burst.is_none());
    }

    fn shot_with_hash(stem: &str, capture_time: NaiveDateTime, similarity_hash: u64) -> Shot {
        Shot {
            stem: stem.to_owned(),
            assets: vec![Asset {
                path: stem.into(),
                kind: MediaKind::Jpeg,
                capture_time: Some(capture_time),
                orientation: None,
                sharpness: None,
                similarity_hash: Some(similarity_hash),
                fingerprint: blake3::hash(stem.as_bytes()),
            }],
            decision: Decision::Unrated,
            burst: None,
        }
    }
}
