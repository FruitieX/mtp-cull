use blake3::Hash;
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    pub sharpness: Option<f64>,
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
    Reject,
    Keep,
    Unrated,
    ToggleZoom,
    ToggleCompare,
}

impl Action {
    pub const ALL: [Self; 7] = [
        Self::Previous,
        Self::Next,
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
            ",
        )?;
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
                let fingerprint = fingerprint_file(&path)?;
                Ok(Asset {
                    sharpness: (kind == MediaKind::Jpeg)
                        .then(|| sharpness_score(&path).ok())
                        .flatten(),
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
        });
    }

    Ok(Session {
        primary_directory,
        raw_directory,
        shots,
    })
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

fn sharpness_score(path: &Path) -> Result<f64> {
    let image = image::ImageReader::open(path)?.decode()?;
    let grayscale = image.thumbnail(1_024, 1_024).to_luma8();
    let width = grayscale.width();
    let height = grayscale.height();
    if width < 3 || height < 3 {
        return Ok(0.0);
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
    Ok(energy / ((width - 2) * (height - 2)) as f64)
}

#[cfg(test)]
mod tests {
    use super::{Database, Decision, MediaKind, load_session};
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
}
