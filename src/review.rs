use crate::mtp_worker::{MtpMediaKind, RemoteSession, classify_media_name};
use color_eyre::eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Decision {
    #[default]
    Unreviewed,
    Reject,
    Keep,
}
impl Decision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unreviewed => "Unreviewed",
            Self::Reject => "Reject",
            Self::Keep => "Keep",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Jpeg,
    Raw,
    Video,
    Other,
}
impl From<MtpMediaKind> for Kind {
    fn from(kind: MtpMediaKind) -> Self {
        match kind {
            MtpMediaKind::Jpeg => Self::Jpeg,
            MtpMediaKind::Raw => Self::Raw,
            MtpMediaKind::Video => Self::Video,
            MtpMediaKind::Heif => Self::Other,
        }
    }
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Raw => "RAW",
            Self::Video => "Video",
            Self::Other => "Other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub id: String,
    pub key: String,
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    pub path: Option<PathBuf>,
    pub decision: Decision,
}
#[derive(Clone, Debug)]
pub struct Shot {
    pub id: String,
    pub name: String,
    pub assets: Vec<Asset>,
}
impl Shot {
    pub fn review_kind(&self, filter: Option<Kind>) -> Kind {
        filter.unwrap_or_else(|| {
            if self.has(Kind::Jpeg) {
                Kind::Jpeg
            } else {
                self.assets.first().map_or(Kind::Other, |a| a.kind)
            }
        })
    }
    pub fn preview(&self) -> Option<&Path> {
        self.assets
            .iter()
            .find(|a| a.kind == Kind::Jpeg)
            .and_then(|a| a.path.as_deref())
    }
    pub fn conflict(&self) -> bool {
        [Kind::Jpeg, Kind::Raw]
            .iter()
            .any(|kind| self.assets.iter().filter(|a| a.kind == *kind).count() > 1)
    }
    pub fn has(&self, kind: Kind) -> bool {
        self.assets.iter().any(|a| a.kind == kind)
    }
    pub fn decision(&self, kind: Kind, linked: bool) -> Decision {
        let kind = if linked && kind == Kind::Raw && self.has(Kind::Jpeg) {
            Kind::Jpeg
        } else {
            kind
        };
        self.assets
            .iter()
            .find(|a| a.kind == kind)
            .map_or(Decision::Unreviewed, |a| a.decision)
    }
}

#[derive(Clone, Debug)]
pub enum Source {
    Local { root: PathBuf, raw: Option<PathBuf> },
    Mtp { device: String, folder: String },
}
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub source: Source,
    pub shots: Vec<Shot>,
}
impl Session {
    pub fn from_remote(remote: RemoteSession) -> Self {
        let id = identity(&format!(
            "mtp\0{}\0{}",
            remote.device_id, remote.source_folder_id
        ));
        let shots = remote
            .shots
            .into_iter()
            .map(|s| Shot {
                id: s.id,
                name: s.stem,
                assets: s
                    .assets
                    .into_iter()
                    .map(|a| Asset {
                        id: a.object_id,
                        key: a.cache_key,
                        name: a.name,
                        kind: a.kind.into(),
                        size: a.size,
                        path: a.preview_path,
                        decision: if a.kind == MtpMediaKind::Video {
                            Decision::Keep
                        } else {
                            Decision::Unreviewed
                        },
                    })
                    .collect(),
            })
            .collect();
        Self {
            id,
            source: Source::Mtp {
                device: remote.device_id,
                folder: remote.source_folder_id,
            },
            shots,
        }
    }
    pub fn selected_ids(&self, settings: &Settings) -> Vec<String> {
        self.shots
            .iter()
            .filter(|shot| !shot.conflict())
            .flat_map(|shot| {
                shot.assets
                    .iter()
                    .filter(move |a| {
                        if a.kind == Kind::Video {
                            return settings.include_videos && a.decision == Decision::Keep;
                        }
                        shot.decision(a.kind, settings.link_raw) == Decision::Keep
                    })
                    .map(|a| a.id.clone())
            })
            .collect()
    }
    pub fn selections(&self) -> HashMap<String, Decision> {
        self.shots
            .iter()
            .flat_map(|s| s.assets.iter().map(|a| (a.key.clone(), a.decision)))
            .collect()
    }
    pub fn restore(&mut self, decisions: &HashMap<String, Decision>) {
        for asset in self.shots.iter_mut().flat_map(|s| &mut s.assets) {
            if let Some(decision) = decisions.get(&asset.key) {
                asset.decision = *decision;
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub colourblind: bool,
    pub link_raw: bool,
    pub include_videos: bool,
    pub auto_advance: bool,
    pub group_bursts: bool,
    pub burst_window_ms: i64,
    pub burst_distance: u32,
    pub cpu_cache_mib: usize,
    pub gpu_cache_mib: usize,
    pub disk_cache_gib: u64,
    pub reel_grid: bool,
    pub reel_position: crate::reel::Position,
    pub reel_width: f32,
    pub reel_height: f32,
    pub reel_thumbnail_size: f32,
    pub reel_scroll_speed: f32,
    pub sampling: crate::viewer::Sampling,
    pub shortcut_defaults_version: u8,
    pub picture_root: String,
    pub raw_root: String,
    pub video_root: String,
    pub album: String,
    pub bindings: BTreeMap<String, String>,
    pub recent_sources: Vec<crate::recent_sources::RecentSource>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            colourblind: false,
            link_raw: true,
            include_videos: true,
            auto_advance: false,
            group_bursts: false,
            burst_window_ms: 2000,
            burst_distance: 8,
            cpu_cache_mib: 8192,
            gpu_cache_mib: 512,
            disk_cache_gib: 32,
            reel_grid: false,
            reel_position: crate::reel::Position::Bottom,
            reel_width: 300.0,
            reel_height: 181.0,
            reel_thumbnail_size: 160.0,
            reel_scroll_speed: 2.0,
            sampling: crate::viewer::Sampling::default(),
            shortcut_defaults_version: 1,
            picture_root: String::new(),
            raw_root: String::new(),
            video_root: String::new(),
            album: String::new(),
            bindings: BTreeMap::new(),
            recent_sources: Vec::new(),
        }
    }
}

pub fn identity(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex().to_string()
}
pub fn local_identity(path: &Path, metadata: &std::fs::Metadata) -> String {
    let stamp = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    identity(&format!("{}\0{}\0{stamp}", path.display(), metadata.len()))
}

pub fn scan_local(root: PathBuf, raw: Option<PathBuf>) -> Result<Session> {
    let root = root
        .canonicalize()
        .wrap_err("could not open source folder")?;
    let raw = raw.map(|p| p.canonicalize()).transpose()?;
    let id = identity(&format!(
        "local\0{}\0{}",
        root.display(),
        raw.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    ));
    let mut groups: BTreeMap<String, Shot> = BTreeMap::new();
    let mut pending = vec![(root.clone(), root.clone())];
    if let Some(raw) = &raw
        && raw != &root
    {
        pending.push((raw.clone(), raw.clone()));
    }
    while let Some((dir, base)) = pending.pop() {
        for entry in
            std::fs::read_dir(&dir).wrap_err_with(|| format!("could not read {}", dir.display()))?
        {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push((entry.path(), base.clone()));
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some((stem, kind)) = classify_media_name(&name) else {
                continue;
            };
            let relative = dir.strip_prefix(&base).unwrap_or(&dir);
            let group = format!("{}\0{stem}", relative.display());
            let shot_id = identity(&format!("{id}\0{group}"));
            let path = entry.path();
            let asset_id = local_identity(&path, &metadata);
            groups
                .entry(group)
                .or_insert_with(|| Shot {
                    id: shot_id,
                    name: stem,
                    assets: Vec::new(),
                })
                .assets
                .push(Asset {
                    key: asset_id.clone(),
                    id: asset_id,
                    name,
                    kind: kind.into(),
                    size: metadata.len(),
                    path: Some(path),
                    decision: if kind == MtpMediaKind::Video {
                        Decision::Keep
                    } else {
                        Decision::Unreviewed
                    },
                });
        }
    }
    Ok(Session {
        id,
        source: Source::Local { root, raw },
        shots: groups.into_values().collect(),
    })
}

type Change = (String, Decision, Decision);
#[derive(Default)]
pub struct History {
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
}
impl History {
    pub fn apply(
        &mut self,
        session: &mut Session,
        indices: &[usize],
        kind: Option<Kind>,
        linked: bool,
        decision: Decision,
    ) {
        let mut changes = Vec::new();
        for &index in indices {
            if let Some(shot) = session.shots.get_mut(index) {
                let kind = shot.review_kind(kind);
                let kind = if linked && kind == Kind::Raw && shot.has(Kind::Jpeg) {
                    Kind::Jpeg
                } else {
                    kind
                };
                for a in shot.assets.iter_mut().filter(|a| a.kind == kind) {
                    if a.decision != decision {
                        changes.push((a.id.clone(), a.decision, decision));
                        a.decision = decision;
                    }
                }
            }
        }
        if !changes.is_empty() {
            self.undo.push(changes);
            self.redo.clear();
        }
    }
    pub fn undo(&mut self, session: &mut Session) {
        if let Some(changes) = self.undo.pop() {
            replay(session, &changes, false);
            self.redo.push(changes);
        }
    }
    pub fn redo(&mut self, session: &mut Session) {
        if let Some(changes) = self.redo.pop() {
            replay(session, &changes, true);
            self.undo.push(changes);
        }
    }
}
fn replay(session: &mut Session, changes: &[Change], forward: bool) {
    let changes: HashMap<_, _> = changes
        .iter()
        .map(|(id, before, after)| (id, if forward { *after } else { *before }))
        .collect();
    for asset in session.shots.iter_mut().flat_map(|s| &mut s.assets) {
        if let Some(decision) = changes.get(&asset.id) {
            asset.decision = *decision;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_media_decisions_include_raw_only_and_video_and_bulk_undo() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.JPG", "a.RAF", "b.RAF", "clip.MOV"] {
            std::fs::write(dir.path().join(name), b"fixture").unwrap();
        }
        let mut session = scan_local(dir.path().to_owned(), None).unwrap();
        let indices = (0..session.shots.len()).collect::<Vec<_>>();
        let mut history = History::default();
        history.apply(
            &mut session,
            &indices,
            Some(Kind::Video),
            true,
            Decision::Reject,
        );
        history.apply(&mut session, &indices, None, true, Decision::Keep);
        assert_eq!(session.selected_ids(&Settings::default()).len(), 4);
        history.undo(&mut session);
        assert!(session.selected_ids(&Settings::default()).is_empty());
    }
    #[test]
    fn metadata_index_handles_five_hundred_pairs_without_decoding() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..500 {
            for ext in ["JPG", "RAF"] {
                std::fs::write(
                    dir.path().join(format!("DSCF{i:04}.{ext}")),
                    b"not an image",
                )
                .unwrap();
            }
        }
        let session = scan_local(dir.path().to_owned(), None).unwrap();
        assert_eq!(session.shots.len(), 500);
        assert!(
            session
                .shots
                .iter()
                .all(|s| s.has(Kind::Jpeg) && s.has(Kind::Raw) && !s.conflict())
        );
    }
    #[test]
    fn linking_filters_and_undo_preserve_independent_choices() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.JPG", "a.RAF", "clip.MOV"] {
            std::fs::write(dir.path().join(name), b"bytes").unwrap();
        }
        let mut session = scan_local(dir.path().to_owned(), None).unwrap();
        let index = session
            .shots
            .iter()
            .position(|s| s.has(Kind::Jpeg))
            .unwrap();
        let mut history = History::default();
        history.apply(
            &mut session,
            &[index],
            Some(Kind::Raw),
            false,
            Decision::Reject,
        );
        history.apply(
            &mut session,
            &[index],
            Some(Kind::Jpeg),
            true,
            Decision::Keep,
        );
        let mut settings = Settings::default();
        assert_eq!(session.selected_ids(&settings).len(), 3);
        settings.link_raw = false;
        assert_eq!(session.selected_ids(&settings).len(), 2);
        history.undo(&mut session);
        assert_eq!(session.selected_ids(&settings).len(), 1);
        history.redo(&mut session);
        assert_eq!(session.selected_ids(&settings).len(), 2);
    }
    #[test]
    fn repeated_stems_in_different_folders_are_not_paired() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("one")).unwrap();
        std::fs::create_dir(dir.path().join("two")).unwrap();
        std::fs::write(dir.path().join("one/a.JPG"), b"jpeg").unwrap();
        std::fs::write(dir.path().join("two/a.RAF"), b"raw").unwrap();
        let session = scan_local(dir.path().to_owned(), None).unwrap();
        assert_eq!(session.shots.len(), 2);
    }
}
