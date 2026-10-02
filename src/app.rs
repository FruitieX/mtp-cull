use crate::image_cache::{ImageCache, Key};
use crate::import_presets::{self, ImportPreset};
use crate::imports::{ImportOperation, ImportUpdate};
use crate::mtp_worker::{ImportPaths, MtpDevice, MtpEvent, MtpRequest, MtpWorker, SourceFolder};
use crate::quick_import;
use crate::recent_sources::{self, RecentSource};
use crate::reel::Position;
use crate::review::{self, Decision, History, Kind, Session, Settings, Source};
use crate::review_commands::{self, COMMANDS, Command};
use crate::review_store::{SavedReview, Store};
use crate::theme;
use crate::viewer::{Canvas, Mode, ViewImage};
use chrono::{Local, NaiveDate};
use color_eyre::eyre::Result;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{atomic::Ordering, mpsc};
use std::time::{Duration, Instant};

pub fn init(args: &crate::cli::UiArgs) -> Result<()> {
    let initial = args.source.as_ref().map(|source| {
        (
            PathBuf::from(source),
            args.raw_source.as_ref().map(PathBuf::from),
        )
    });
    let viewport = egui::ViewportBuilder::default()
        .with_icon(egui::IconData {
            rgba: include_bytes!("../assets/icon.rgba").to_vec(),
            width: 64,
            height: 64,
        })
        .with_title("mtp-cull · Camera review")
        .with_maximized(true);
    #[cfg(feature = "ui-smoke")]
    let viewport = if std::env::var_os("MTP_CULL_SMOKE_DIR").is_some() {
        let size = std::env::var("MTP_CULL_SMOKE_SIZE")
            .ok()
            .and_then(|value| {
                let (width, height) = value.split_once('x')?;
                Some(egui::vec2(width.parse().ok()?, height.parse().ok()?))
            })
            .filter(|size| size.x >= 640.0 && size.y >= 480.0)
            .unwrap_or(egui::vec2(2560.0, 1440.0));
        viewport.with_maximized(false).with_inner_size(size)
    } else {
        viewport
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "mtp-cull",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            let mut app = App::new()?;
            if let Some(state) = &cc.wgpu_render_state {
                app.canvas.set_gpu(state.clone());
            }
            if let Some((root, raw)) = initial {
                let (sender, receiver) = mpsc::channel();
                app.loader = Some(receiver);
                std::thread::spawn(move || {
                    let _ =
                        sender.send(review::scan_local(root, raw).map_err(|e| format!("{e:#}")));
                });
            }
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}

#[path = "filmstrip.rs"]
mod filmstrip;

struct App {
    #[cfg(feature = "ui-smoke")]
    smoke: Option<smoke::Smoke>,
    store: Store,
    settings: Settings,
    draft_settings: Settings,
    session: Option<Session>,
    history: History,
    cache: ImageCache,
    canvas: Canvas,
    selected: usize,
    reel_selection: crate::reel::Selection,
    reel_follow: bool,
    reel_viewport: egui::Rect,
    reel_clip: egui::Rect,
    reel_layout: Option<(usize, egui::Vec2)>,
    #[cfg(feature = "ui-smoke")]
    reel_cells: Vec<(usize, egui::Rect)>,
    #[cfg(feature = "ui-smoke")]
    reel_menu_items: Vec<(Command, egui::Rect)>,
    #[cfg(feature = "ui-smoke")]
    reel_panel_bounds: egui::Rect,
    #[cfg(feature = "ui-smoke")]
    reel_size_slider: egui::Rect,
    pinned: Option<usize>,
    media_filter: Option<Kind>,
    decision_filter: Option<Decision>,
    visible: Vec<usize>,
    demands: Vec<(Key, u8)>,
    thumbnails: HashMap<Key, egui::TextureHandle>,
    thumbnail_ticks: HashMap<Key, u64>,
    thumbnail_tick: u64,
    features: HashMap<PathBuf, crate::bursts::Feature>,
    burst_groups: HashMap<usize, u32>,
    loader: Option<mpsc::Receiver<std::result::Result<Session, String>>>,
    import: Option<ImportOperation>,
    worker: Option<MtpWorker>,
    devices: Vec<MtpDevice>,
    folders: Vec<SourceFolder>,
    device: Option<String>,
    folder: Option<String>,
    mtp_pending: HashSet<String>,
    staging_request: Option<(Vec<String>, bool, u64)>,
    staging_paused: bool,
    staging_active: bool,
    staging_text: String,
    camera_progress: f32,
    camera_busy: bool,
    camera_picker: bool,
    pending_camera: Option<(RecentSource, bool)>,
    presets_path: PathBuf,
    presets: Vec<ImportPreset>,
    presets_open: bool,
    preset_from_review: bool,
    preset_edit: Option<(Option<usize>, ImportPreset)>,
    preset_error: Option<String>,
    quick_import: Option<quick_import::Run>,
    settings_open: bool,
    settings_tab: u8,
    import_open: bool,
    #[cfg(feature = "ui-smoke")]
    import_area: Option<(egui::Rect, egui::LayerId)>,
    #[cfg(feature = "ui-smoke")]
    import_ui_state: (bool, bool),
    help_open: bool,
    palette_open: bool,
    palette_query: String,
    palette_selection: usize,
    palette_focus: bool,
    date: String,
    message: Option<String>,
    error: Option<String>,
    last_direction: isize,
    frame_ms: f64,
}
impl App {
    fn new() -> Result<Self> {
        Self::with_store(Store::open()?, import_presets::config_path()?)
    }
    fn with_store(store: Store, presets_path: PathBuf) -> Result<Self> {
        let settings = store.settings()?;
        let cache = ImageCache::new(settings.cpu_cache_mib * 1024 * 1024);
        let (presets, preset_error) = match import_presets::load(&presets_path) {
            Ok(presets) => (presets, None),
            Err(error) => (Vec::new(), Some(format!("{error:#}"))),
        };
        Ok(Self {
            #[cfg(feature = "ui-smoke")]
            smoke: smoke::Smoke::from_environment()?,
            store,
            draft_settings: settings.clone(),
            settings,
            session: None,
            history: History::default(),
            cache,
            canvas: Canvas::default(),
            selected: 0,
            reel_selection: crate::reel::Selection::default(),
            reel_follow: true,
            reel_viewport: egui::Rect::NOTHING,
            reel_clip: egui::Rect::NOTHING,
            reel_layout: None,
            #[cfg(feature = "ui-smoke")]
            reel_cells: Vec::new(),
            #[cfg(feature = "ui-smoke")]
            reel_menu_items: Vec::new(),
            #[cfg(feature = "ui-smoke")]
            reel_panel_bounds: egui::Rect::NOTHING,
            #[cfg(feature = "ui-smoke")]
            reel_size_slider: egui::Rect::NOTHING,
            pinned: None,
            media_filter: Some(Kind::Jpeg),
            decision_filter: None,
            visible: Vec::new(),
            demands: Vec::new(),
            thumbnails: HashMap::new(),
            thumbnail_ticks: HashMap::new(),
            thumbnail_tick: 0,
            features: HashMap::new(),
            burst_groups: HashMap::new(),
            loader: None,
            import: None,
            worker: None,
            devices: Vec::new(),
            folders: Vec::new(),
            device: None,
            folder: None,
            mtp_pending: HashSet::new(),
            staging_request: None,
            staging_paused: false,
            staging_active: false,
            staging_text: String::new(),
            camera_progress: 0.0,
            camera_busy: false,
            camera_picker: false,
            pending_camera: None,
            presets_path,
            presets,
            presets_open: false,
            preset_from_review: false,
            preset_edit: None,
            preset_error,
            quick_import: None,
            settings_open: false,
            settings_tab: 0,
            import_open: false,
            #[cfg(feature = "ui-smoke")]
            import_area: None,
            #[cfg(feature = "ui-smoke")]
            import_ui_state: (false, false),
            help_open: false,
            palette_open: false,
            palette_query: String::new(),
            palette_selection: 0,
            palette_focus: false,
            date: Local::now().date_naive().to_string(),
            message: None,
            error: None,
            last_direction: 1,
            frame_ms: 0.0,
        })
    }
    fn save(&self) {
        self.store.save_settings(&self.settings);
        if let Some(session) = &self.session {
            let key = |index: usize| {
                session
                    .shots
                    .get(index)
                    .and_then(|s| {
                        s.assets
                            .iter()
                            .find(|a| a.kind == Kind::Jpeg)
                            .or_else(|| s.assets.first())
                    })
                    .map(|a| a.key.clone())
            };
            self.store.save_review(
                &session.id,
                SavedReview {
                    decisions: session.selections(),
                    selected: key(self.selected),
                    pinned: self.pinned.and_then(key),
                    center: Some([self.canvas.viewport.center.x, self.canvas.viewport.center.y]),
                    zoom: self.canvas.viewport.zoom,
                    alignment: [
                        self.canvas.viewport.alignment.x,
                        self.canvas.viewport.alignment.y,
                    ],
                    mode: self.canvas.mode.label().into(),
                    threshold: self.canvas.threshold,
                    opacity: self.canvas.opacity,
                },
            );
        }
    }
    fn install(&mut self, mut session: Session) {
        let mut reconciliation = String::new();
        let carried=self.session.as_ref().filter(|old|old.id!=session.id && matches!((&old.source,&session.source),(Source::Local{root:a,..},Source::Local{root:b,..}) if a==b)).map(Session::selections);
        let mut saved = None;
        match self.store.review(&session.id) {
            Ok(review) => {
                let decisions = &review.decisions;
                let current = session.selections();
                let missing = decisions
                    .keys()
                    .filter(|key| !current.contains_key(*key))
                    .count();
                session.restore(decisions);
                if missing > 0 {
                    reconciliation = format!(
                        " {missing} previous asset records could not be matched; review changed files again."
                    );
                }
                saved = Some(review);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        if let Some(carried) = carried {
            session.restore(&carried);
        }
        self.remember_source(&session.source);
        self.session = Some(session);
        self.selected = 0;
        self.reel_selection = crate::reel::Selection::default();
        self.reel_follow = true;
        self.pinned = None;
        self.history = History::default();
        self.cache.clear();
        self.canvas.reset();
        if let Some(saved) = saved {
            let session = self.session.as_ref().unwrap();
            let index = |key: &Option<String>| {
                key.as_ref().and_then(|key| {
                    session
                        .shots
                        .iter()
                        .position(|s| s.assets.iter().any(|a| a.key == *key))
                })
            };
            self.selected = index(&saved.selected).unwrap_or(0);
            self.pinned = index(&saved.pinned);
            if let Some(center) = saved.center {
                self.canvas.viewport.center = egui::vec2(center[0], center[1]);
            }
            self.canvas.viewport.zoom = saved.zoom;
            self.canvas.viewport.alignment = egui::vec2(saved.alignment[0], saved.alignment[1]);
            self.canvas.mode = match saved.mode.as_str() {
                "Side by side" => Mode::SideBySide,
                "Vertical wipe" => Mode::Wipe,
                _ => Mode::Single,
            };
            if saved.threshold > 0 {
                self.canvas.threshold = saved.threshold;
                self.canvas.opacity = saved.opacity;
            }
        }
        self.thumbnails.clear();
        self.thumbnail_ticks.clear();
        self.features.clear();
        self.burst_groups.clear();
        self.mtp_pending.clear();
        self.staging_request = None;
        self.staging_paused = false;
        self.refresh_visible();
        let ephemeral = self
            .session
            .as_ref()
            .unwrap()
            .shots
            .iter()
            .flat_map(|s| &s.assets)
            .filter(|a| a.key.starts_with("ephemeral-"))
            .count();
        if ephemeral > 0 {
            reconciliation.push_str(" Camera timestamps are unavailable for some files; their decisions will require review on reconnect.");
        }
        self.message = (!reconciliation.is_empty()).then_some(reconciliation);
    }
    fn refresh_visible(&mut self) {
        let previous = self.selected;
        self.visible = self
            .session
            .as_ref()
            .map(|session| {
                session
                    .shots
                    .iter()
                    .enumerate()
                    .filter(|(_, shot)| {
                        self.media_filter.is_none_or(|kind| shot.has(kind))
                            && self.decision_filter.is_none_or(|decision| {
                                shot.decision(
                                    shot.review_kind(self.media_filter),
                                    self.settings.link_raw,
                                ) == decision
                            })
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default();
        if !self.visible.contains(&self.selected)
            && let Some(&first) = self.visible.first()
        {
            self.selected = first;
        }
        self.reel_selection.retain_visible(&self.visible);
        if self.visible.contains(&self.selected)
            && (self.selected != previous
                || (self.reel_selection.indices.is_empty() && self.reel_follow))
        {
            self.reel_selection.single(self.selected);
        }
        self.reel_follow = true;
    }
    fn navigate(&mut self, direction: isize) {
        self.last_direction = direction.signum();
        if let Some(position) = self.visible.iter().position(|i| *i == self.selected) {
            let next = position
                .saturating_add_signed(direction)
                .min(self.visible.len().saturating_sub(1));
            self.selected = self.visible[next];
            self.reel_selection.single(self.selected);
            self.reel_follow = true;
            self.canvas.active_a = false;
        }
    }
    fn navigate_row(&mut self, direction: isize) {
        if !self.settings.reel_grid {
            self.navigate(direction);
            return;
        }
        let columns = self.reel_layout.map_or(1, |(columns, _)| columns.max(1));
        if let Some(position) = self.visible.iter().position(|i| *i == self.selected) {
            let row = position / columns;
            let rows = self.visible.len().div_ceil(columns);
            if (direction < 0 && row > 0) || (direction > 0 && row + 1 < rows) {
                self.navigate(direction * columns as isize);
            }
        }
    }
    fn set_reel_position(&mut self, position: Position) {
        self.settings.reel_position = position;
        self.draft_settings.reel_position = position;
        self.reel_layout = None;
        self.reel_follow = true;
        self.store.save_settings(&self.settings);
    }
    fn resize_reel_thumbnails(&mut self, delta: f32) {
        if self.settings.reel_grid {
            self.settings.reel_thumbnail_size = (self.settings.reel_thumbnail_size + delta).clamp(
                crate::reel::MIN_THUMBNAIL_WIDTH,
                crate::reel::MAX_THUMBNAIL_WIDTH,
            );
            self.draft_settings.reel_thumbnail_size = self.settings.reel_thumbnail_size;
            self.store.save_settings(&self.settings);
        }
    }
    fn decide(&mut self, decision: Decision, bulk: bool) {
        if self.visible.is_empty() {
            return;
        }
        let indices = if bulk {
            self.visible.clone()
        } else if !self.canvas.active_a && !self.reel_selection.indices.is_empty() {
            self.reel_selection.indices.iter().copied().collect()
        } else {
            vec![if self.canvas.active_a {
                self.pinned.unwrap_or(self.selected)
            } else {
                self.selected
            }]
        };
        if let Some(session) = &mut self.session {
            self.history.apply(
                session,
                &indices,
                self.media_filter,
                self.settings.link_raw,
                decision,
            );
        }
        self.save();
        if self.settings.auto_advance && indices.len() == 1 && !bulk && !self.canvas.active_a {
            self.navigate(1);
        }
        self.refresh_visible();
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::Bursts => {
                self.settings.group_bursts = !self.settings.group_bursts;
                self.save();
            }
            Command::NextBurst | Command::PreviousBurst => {
                let current = self.burst_groups.get(&self.selected).copied();
                let direction = if command == Command::NextBurst { 1 } else { -1 };
                while let Some(position) = self.visible.iter().position(|i| *i == self.selected) {
                    if (direction == 1 && position + 1 == self.visible.len())
                        || (direction == -1 && position == 0)
                    {
                        break;
                    }
                    self.navigate(direction);
                    if self.burst_groups.get(&self.selected).copied() != current
                        || current.is_none()
                    {
                        break;
                    }
                }
            }
            Command::FilterUnreviewed
            | Command::FilterKeep
            | Command::FilterReject
            | Command::FilterAll => {
                self.decision_filter = match command {
                    Command::FilterUnreviewed => Some(Decision::Unreviewed),
                    Command::FilterKeep => Some(Decision::Keep),
                    Command::FilterReject => Some(Decision::Reject),
                    _ => None,
                };
                self.refresh_visible();
            }
            Command::PanLeft | Command::PanRight | Command::PanUp | Command::PanDown => {
                let delta = match command {
                    Command::PanLeft => egui::vec2(-0.02, 0.0),
                    Command::PanRight => egui::vec2(0.02, 0.0),
                    Command::PanUp => egui::vec2(0.0, -0.02),
                    _ => egui::vec2(0.0, 0.02),
                };
                self.canvas.viewport.center += delta;
            }
            Command::CenterRegion => {
                self.canvas.center_region_on_load = true;
            }
            Command::RegionLeft
            | Command::RegionRight
            | Command::RegionUp
            | Command::RegionDown => {
                let delta = match command {
                    Command::RegionLeft => egui::vec2(-16.0, 0.0),
                    Command::RegionRight => egui::vec2(16.0, 0.0),
                    Command::RegionUp => egui::vec2(0.0, -16.0),
                    _ => egui::vec2(0.0, 16.0),
                };
                self.canvas.region = self.canvas.region.map(|r| r.translate(delta));
            }
            Command::RegionGrow | Command::RegionShrink => {
                self.canvas.region = self.canvas.region.map(|r| {
                    egui::Rect::from_center_size(
                        r.center(),
                        (r.size()
                            * if command == Command::RegionGrow {
                                1.2
                            } else {
                                1.0 / 1.2
                            })
                        .max(egui::vec2(4.0, 4.0)),
                    )
                });
            }
            Command::ThresholdUp => self.canvas.threshold = self.canvas.threshold.saturating_add(2),
            Command::ThresholdDown => {
                self.canvas.threshold = self.canvas.threshold.saturating_sub(2).max(1)
            }
            Command::OpacityUp => self.canvas.opacity = self.canvas.opacity.saturating_add(16),
            Command::OpacityDown => self.canvas.opacity = self.canvas.opacity.saturating_sub(16),
            Command::AlignLeft | Command::AlignRight | Command::AlignUp | Command::AlignDown => {
                let delta = match command {
                    Command::AlignLeft => egui::vec2(-8.0, 0.0),
                    Command::AlignRight => egui::vec2(8.0, 0.0),
                    Command::AlignUp => egui::vec2(0.0, -8.0),
                    _ => egui::vec2(0.0, 8.0),
                };
                self.canvas.viewport.alignment += delta;
            }
            Command::Close => {
                if self.import.is_some() || self.camera_busy || self.loader.is_some() {
                    return;
                }
                self.save();
                self.send(MtpRequest::CloseSession);
                self.session = None;
                self.loader = None;
                self.cache.clear();
                self.canvas.reset();
                self.thumbnails.clear();
                self.visible.clear();
                self.reel_selection = crate::reel::Selection::default();
                self.staging_text.clear();
                self.staging_active = false;
                self.staging_request = None;
            }
            Command::RawFolder => {
                if !self.can_open_source() {
                    return;
                }
                if let Some(Session {
                    source: Source::Local { root, raw },
                    ..
                }) = &self.session
                {
                    let root = root.clone();
                    let dialog = rfd::FileDialog::new()
                        .set_title("Companion RAW folder")
                        .set_directory(raw.as_ref().unwrap_or(&root));
                    if let Some(raw) = dialog.pick_folder() {
                        self.open_local(root, Some(raw));
                    }
                }
            }
            Command::Open => {
                if !self.can_open_source() {
                    return;
                }
                let mut dialog =
                    rfd::FileDialog::new().set_title("JPEG album or camera staging folder");
                if let Some(root) =
                    self.settings
                        .recent_sources
                        .iter()
                        .find_map(|source| match source {
                            RecentSource::Local { root, .. } => Some(root),
                            _ => None,
                        })
                {
                    dialog = dialog.set_directory(root);
                }
                if let Some(root) = dialog.pick_folder() {
                    self.open_local(root, None);
                }
            }
            Command::Camera => {
                let recent = self
                    .settings
                    .recent_sources
                    .iter()
                    .find(|source| matches!(source, RecentSource::Camera { .. }))
                    .cloned();
                self.open_camera(recent.map(|source| (source, false)));
            }
            Command::Previous => self.navigate(-1),
            Command::Next => self.navigate(1),
            Command::RowUp => self.navigate_row(-1),
            Command::RowDown => self.navigate_row(1),
            Command::NextUnreviewed => {
                if let Some(session) = &self.session
                    && let Some(index) = self
                        .visible
                        .iter()
                        .copied()
                        .cycle()
                        .skip_while(|i| *i != self.selected)
                        .skip(1)
                        .take(self.visible.len())
                        .find(|i| {
                            session.shots[*i].decision(
                                session.shots[*i].review_kind(self.media_filter),
                                self.settings.link_raw,
                            ) == Decision::Unreviewed
                        })
                {
                    self.selected = index;
                    self.reel_selection.single(index);
                    self.reel_follow = true;
                }
            }
            Command::Reject => self.decide(Decision::Reject, false),
            Command::Keep => self.decide(Decision::Keep, false),
            Command::Clear => self.decide(Decision::Unreviewed, false),
            Command::ToggleKeep => {
                let index = if self.canvas.active_a {
                    self.pinned.unwrap_or(self.selected)
                } else {
                    self.selected
                };
                let keep = self
                    .session
                    .as_ref()
                    .and_then(|s| s.shots.get(index))
                    .is_some_and(|s| {
                        s.decision(s.review_kind(self.media_filter), self.settings.link_raw)
                            == Decision::Keep
                    });
                self.decide(
                    if keep {
                        Decision::Unreviewed
                    } else {
                        Decision::Keep
                    },
                    false,
                );
            }
            Command::Zoom => {
                self.canvas.viewport.zoom = if self.canvas.viewport.zoom.is_some() {
                    None
                } else {
                    Some(1.0)
                }
            }
            Command::Pin => {
                self.pinned = Some(self.selected);
                self.navigate(1);
                if self.canvas.mode == Mode::Single {
                    self.canvas.mode = Mode::SideBySide;
                }
            }
            Command::Compare => {
                if self.pinned.is_none() {
                    self.pinned = Some(self.selected);
                    self.navigate(1);
                }
                self.canvas.mode = self.canvas.mode.next();
            }
            Command::Blink => {}
            Command::Swap => {
                let source = self
                    .session
                    .as_ref()
                    .and_then(|s| s.shots.get(self.selected))
                    .and_then(|s| s.preview())
                    .and_then(|p| self.cache.source_size(p));
                if let Some(source) = source {
                    let alignment = self.canvas.viewport.alignment;
                    self.canvas.region = self.canvas.region.map(|r| r.translate(alignment));
                    self.canvas
                        .viewport
                        .swap(egui::vec2(source[0] as f32, source[1] as f32));
                }
                if let Some(a) = self.pinned.replace(self.selected) {
                    self.selected = a;
                    self.reel_selection.single(a);
                    self.reel_follow = true;
                    self.canvas.active_a = !self.canvas.active_a;
                }
            }
            Command::ActivePane => {
                if self.pinned.is_some() {
                    self.canvas.active_a = !self.canvas.active_a;
                }
            }
            Command::Peaking => self.canvas.peaking = !self.canvas.peaking,
            Command::Region => self.canvas.region_tool = !self.canvas.region_tool,
            Command::ClearRegion => {
                self.canvas.region = None;
                self.canvas.region_tool = false;
                self.canvas.center_region_on_load = false;
            }
            Command::DividerLeft => self.canvas.divider = (self.canvas.divider - 0.05).max(0.0),
            Command::DividerRight => self.canvas.divider = (self.canvas.divider + 0.05).min(1.0),
            Command::DividerCenter => self.canvas.divider = 0.5,
            Command::ResetAlignment => self.canvas.viewport.alignment = egui::Vec2::ZERO,
            Command::Undo | Command::Redo => {
                if let Some(session) = &mut self.session {
                    if command == Command::Undo {
                        self.history.undo(session);
                    } else {
                        self.history.redo(session);
                    }
                }
                self.save();
                self.refresh_visible();
            }
            Command::Jpeg => {
                self.media_filter = Some(Kind::Jpeg);
                self.refresh_visible();
            }
            Command::Raw => {
                self.media_filter = Some(Kind::Raw);
                self.refresh_visible();
            }
            Command::Video => {
                self.media_filter = Some(Kind::Video);
                self.refresh_visible();
            }
            Command::All => {
                self.media_filter = None;
                self.refresh_visible();
            }
            Command::BulkKeep => self.decide(Decision::Keep, true),
            Command::BulkReject => self.decide(Decision::Reject, true),
            Command::SelectAll => {
                self.reel_selection.indices = self.visible.iter().copied().collect();
                self.canvas.active_a = false;
            }
            Command::DeselectAll => self.reel_selection = crate::reel::Selection::default(),
            Command::ToggleReelSelection => {
                if self.visible.contains(&self.selected) {
                    self.reel_selection
                        .click(self.selected, &self.visible, true, false);
                    self.canvas.active_a = false;
                }
            }
            Command::ReelMode => {
                self.settings.reel_grid = !self.settings.reel_grid;
                self.draft_settings.reel_grid = self.settings.reel_grid;
                self.reel_follow = true;
                self.store.save_settings(&self.settings);
            }
            Command::ReelPosition => self.set_reel_position(self.settings.reel_position.next()),
            Command::ReelSmaller => self.resize_reel_thumbnails(-20.0),
            Command::ReelLarger => self.resize_reel_thumbnails(20.0),
            Command::Import => {
                if self.import.is_none() && !self.camera_busy && self.loader.is_none() {
                    self.import_open = true;
                }
            }
            Command::Settings => {
                self.draft_settings = self.settings.clone();
                self.settings_open = true;
            }
            Command::Help => self.help_open = !self.help_open,
            Command::Palette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
                self.palette_selection = 0;
                self.palette_focus = true;
            }
            Command::Retry => {
                self.cache.retry();
                self.mtp_pending.clear();
                self.send(MtpRequest::Retry);
                self.staging_paused = false;
                self.staging_request = None;
            }
            Command::Pause => {
                self.staging_paused = !self.staging_paused;
                self.staging_request = None;
            }
            Command::Presets => {
                self.presets_open = true;
                if self.presets.is_empty() && self.preset_error.is_none() {
                    self.preset_edit = Some((None, ImportPreset::default()));
                }
            }
            Command::Cancel => {
                if self.quick_import.take().is_some() {
                    self.send(MtpRequest::CloseSession);
                    self.camera_busy = false;
                    self.message =
                        Some("Import cancelled; completed destination files are preserved".into());
                }
                let modal = self.settings_open
                    || self.presets_open
                    || self.import_open
                    || self.help_open
                    || self.palette_open
                    || self.camera_picker;
                if !modal || self.camera_busy || self.import.is_some() {
                    if let Some(import) = &self.import {
                        import.cancel.store(true, Ordering::Relaxed);
                    }
                    self.staging_paused = true;
                    if self.camera_picker && self.camera_busy {
                        self.send(MtpRequest::CloseSession);
                        self.camera_busy = false;
                    } else {
                        self.send(MtpRequest::Cancel);
                    }
                }
                self.settings_open = false;
                self.presets_open = false;
                self.preset_edit = None;
                self.import_open = false;
                self.help_open = false;
                self.palette_open = false;
                self.camera_picker = false;
                self.pending_camera = None;
            }
        }
    }
    fn send(&mut self, request: MtpRequest) {
        if let Some(worker) = &self.worker
            && let Err(error) = worker.send(request)
        {
            self.error = Some(error.to_string());
            self.camera_busy = false;
            self.quick_import = None;
        }
    }
    fn can_run_preset(&self) -> bool {
        self.can_open_source() && self.session.is_none()
    }
    fn start_preset(&mut self, preset: &ImportPreset) {
        if !self.can_run_preset() {
            return;
        }
        let run = match quick_import::Run::new(preset) {
            Ok(run) => run,
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                return;
            }
        };
        if self.worker.is_none() {
            match MtpWorker::spawn() {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.error = Some(format!("{error:#}"));
                    return;
                }
            }
        }
        self.send(MtpRequest::CloseSession);
        self.camera_picker = false;
        self.pending_camera = None;
        self.staging_text.clear();
        self.staging_active = false;
        self.camera_busy = true;
        self.camera_progress = 0.0;
        self.message = Some(format!("{}: looking for device...", run.name));
        self.error = None;
        self.quick_import = Some(run);
        self.send(MtpRequest::ListDevices);
    }
    fn handle_quick_import(&mut self, event: &MtpEvent) {
        let Some(run) = &mut self.quick_import else {
            return;
        };
        match run.event(event) {
            Ok(quick_import::Action::None) => {}
            Ok(quick_import::Action::Request(request)) => {
                self.camera_progress = 0.0;
                self.message = Some(format!(
                    "{}: {}...",
                    run.name,
                    if matches!(request, MtpRequest::ListFiles { .. }) {
                        "listing files"
                    } else {
                        "copying files"
                    }
                ));
                self.send(request);
            }
            Ok(quick_import::Action::Finished(result)) => {
                let name = run.name.clone();
                self.quick_import = None;
                self.camera_busy = false;
                self.message = Some(format!(
                    "{name}: copied {} files; {} already present; {} failed",
                    result.copied_files,
                    result.skipped_files,
                    result.errors.len()
                ));
                if !result.errors.is_empty() {
                    self.error = Some(
                        result
                            .errors
                            .iter()
                            .map(|error| format!("{}: {}", error.source_path, error.message))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                self.send(MtpRequest::CloseSession);
            }
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                self.message = Some("Could not start preset import".into());
                self.quick_import = None;
                self.camera_busy = false;
                self.send(MtpRequest::CloseSession);
            }
        }
    }
    fn can_open_source(&self) -> bool {
        self.import.is_none() && !self.camera_busy && self.loader.is_none()
    }
    fn open_local(&mut self, root: PathBuf, raw: Option<PathBuf>) {
        if !self.can_open_source() {
            return;
        }
        self.save();
        self.send(MtpRequest::CloseSession);
        self.camera_picker = false;
        self.pending_camera = None;
        self.staging_text.clear();
        self.staging_active = false;
        self.error = None;
        let (sender, receiver) = mpsc::channel();
        self.loader = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(review::scan_local(root, raw).map_err(|e| format!("{e:#}")));
        });
    }
    fn open_recent(&mut self, source: RecentSource) {
        match source {
            RecentSource::Local { root, raw } => self.open_local(root, raw),
            camera => self.open_camera(Some((camera, true))),
        }
    }
    fn open_camera(&mut self, recent: Option<(RecentSource, bool)>) {
        if !self.can_open_source() {
            return;
        }
        if self.worker.is_none() {
            match MtpWorker::spawn() {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.error = Some(error.to_string());
                    return;
                }
            }
        }
        self.devices.clear();
        self.folders.clear();
        self.device = None;
        self.folder = None;
        self.pending_camera = recent;
        self.camera_picker = true;
        self.camera_busy = true;
        self.camera_progress = 0.0;
        self.error = None;
        self.message = Some("Looking for connected cameras...".into());
        self.send(MtpRequest::ListDevices);
    }
    fn select_camera(&mut self, device_id: String) {
        self.device = Some(device_id.clone());
        self.folder = None;
        self.folders.clear();
        self.camera_busy = true;
        self.camera_progress = 0.0;
        self.message = Some("Reading camera folders...".into());
        self.send(MtpRequest::ListSourceFolders { device_id });
    }
    fn start_camera_review(&mut self) {
        if self.camera_busy {
            return;
        }
        let (Some(device_id), Some(source_folder_id)) = (self.device.clone(), self.folder.clone())
        else {
            return;
        };
        if !self
            .folders
            .iter()
            .any(|folder| folder.id == source_folder_id)
        {
            return;
        }
        self.save();
        self.camera_busy = true;
        self.camera_progress = 0.0;
        self.error = None;
        self.message = Some("Indexing camera...".into());
        self.send(MtpRequest::StartSession {
            device_id,
            source_folder_id,
        });
    }
    fn remember_source(&mut self, source: &Source) {
        let recent = match source {
            Source::Local { root, raw } => RecentSource::Local {
                root: root.clone(),
                raw: raw.clone(),
            },
            Source::Mtp { device, folder } => {
                let Some(device) = self.devices.iter().find(|item| item.id == *device) else {
                    return;
                };
                let Some(folder) = self.folders.iter().find(|item| item.id == *folder) else {
                    return;
                };
                if let Some((previous, _)) = self.pending_camera.take()
                    && previous
                        .device(&self.devices)
                        .is_some_and(|item| item.id == device.id)
                    && previous
                        .folder(&self.folders)
                        .is_some_and(|item| item.id == folder.id)
                {
                    self.settings
                        .recent_sources
                        .retain(|item| item != &previous);
                }
                RecentSource::Camera {
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    folder_id: folder.id.clone(),
                    folder_path: folder.path.clone(),
                }
            }
        };
        recent_sources::remember(&mut self.settings.recent_sources, recent);
        self.draft_settings.recent_sources = self.settings.recent_sources.clone();
        self.store.save_settings(&self.settings);
    }
    fn recent_sources_ui(&mut self, ui: &mut egui::Ui, limit: usize, centered: bool) {
        let sources: Vec<_> = self
            .settings
            .recent_sources
            .iter()
            .take(limit)
            .cloned()
            .collect();
        let mut chosen = None;
        for source in sources {
            let button = egui::Button::new(source.label()).truncate();
            let response = if centered {
                ui.add_enabled_ui(self.can_open_source(), |ui| {
                    ui.add_sized(egui::vec2(ui.available_width().min(440.0), 30.0), button)
                })
                .inner
            } else {
                ui.add_enabled(self.can_open_source(), button)
            };
            if response.on_hover_text(source.detail()).clicked() {
                chosen = Some(source);
            }
            if centered {
                ui.add_space(4.0);
            }
        }
        if let Some(source) = chosen {
            self.open_recent(source);
            if !centered {
                ui.close();
            }
        }
    }
    fn poll(&mut self) {
        if let Some(loader) = &self.loader {
            match loader.try_recv() {
                Ok(Ok(session)) => {
                    self.loader = None;
                    self.install(session);
                }
                Ok(Err(error)) => {
                    self.loader = None;
                    self.error = Some(error);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loader = None;
                    self.error = Some("Folder indexer stopped".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut events = Vec::new();
        if let Some(worker) = &self.worker {
            while let Some(event) = worker.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            self.handle_quick_import(&event);
            match event {
                MtpEvent::FilesListed(_) | MtpEvent::CopyFinished(_) => {}
                MtpEvent::Devices(devices) => {
                    if self.camera_picker {
                        self.devices = devices;
                        self.camera_busy = false;
                        let suggested = self
                            .pending_camera
                            .as_ref()
                            .and_then(|(source, _)| source.device(&self.devices))
                            .or_else(|| {
                                (self.pending_camera.is_none() && self.devices.len() == 1)
                                    .then(|| &self.devices[0])
                            })
                            .map(|device| device.id.clone());
                        if let Some(device) = suggested {
                            self.select_camera(device);
                        } else {
                            self.message = Some(
                                "Connect your camera, then refresh or choose a connected device."
                                    .into(),
                            );
                        }
                    }
                }
                MtpEvent::SourceFolders { device_id, folders } => {
                    if self.camera_picker && self.device.as_ref() == Some(&device_id) {
                        self.folders = folders;
                        self.camera_busy = false;
                        if let Some((source, auto_start)) = &self.pending_camera {
                            if let Some(folder) = source.folder(&self.folders) {
                                self.folder = Some(folder.id.clone());
                                self.message =
                                    Some("Previously used camera folder selected.".into());
                                if *auto_start {
                                    self.start_camera_review();
                                }
                            } else {
                                self.message = Some(
                                    "Previous folder is unavailable. Choose a camera folder below."
                                        .into(),
                                );
                            }
                        } else {
                            self.message =
                                Some("Choose a camera folder, then start review.".into());
                        }
                    }
                }
                MtpEvent::SessionScanned(session) => {
                    self.camera_busy = false;
                    self.camera_picker = false;
                    self.install(Session::from_remote(session));
                }
                MtpEvent::PreviewCached {
                    shot_id,
                    preview_path,
                } => {
                    self.mtp_pending.remove(&shot_id);
                    if let Some(session) = &mut self.session
                        && let Some(shot) = session.shots.iter_mut().find(|s| s.id == shot_id)
                    {
                        for asset in shot.assets.iter_mut().filter(|a| a.kind == Kind::Jpeg) {
                            asset.path = preview_path.clone();
                        }
                    }
                }
                MtpEvent::PreviewFailed { shot_id, message } => {
                    self.mtp_pending.remove(&shot_id);
                    self.error = Some(message);
                }
                MtpEvent::ImportFinished {
                    copied,
                    skipped_existing,
                } => {
                    self.camera_busy = false;
                    self.message = Some(format!(
                        "Imported {copied} files; {skipped_existing} already present"
                    ));
                }
                MtpEvent::Error { operation, message } => {
                    if self.quick_import.take().is_some() {
                        self.send(MtpRequest::CloseSession);
                        self.message = Some(
                            "Preset import failed; completed destination files are preserved"
                                .into(),
                        );
                    }
                    self.camera_busy = false;
                    self.error = Some(format!("{operation}: {message}"));
                }
                MtpEvent::Progress {
                    operation,
                    name,
                    done,
                    total,
                } => {
                    self.camera_progress = if total == 0 {
                        1.0
                    } else {
                        done as f32 / total as f32
                    };
                    if operation != "staging" {
                        self.message = Some(format!("{operation}: {name}"));
                    }
                }
                MtpEvent::Staging {
                    ready,
                    total,
                    bytes,
                    paused,
                } => {
                    self.staging_active = ready < total && !paused;
                    self.staging_text = format!(
                        "{ready}/{total} JPEGs staged · {:.1} GiB{}",
                        bytes as f64 / 1073741824.0,
                        if paused { " · paused" } else { "" }
                    );
                }
            }
        }
        if let Some(import) = &mut self.import {
            let mut finished = None;
            while let Ok(update) = import.receiver.try_recv() {
                match update {
                    ImportUpdate::Progress { name, done, total } => {
                        import.text = name;
                        import.progress = if total == 0 {
                            1.0
                        } else {
                            done as f32 / total as f32
                        };
                    }
                    ImportUpdate::Finished(result) => finished = Some(result),
                }
            }
            if let Some(result) = finished {
                self.import = None;
                match result {
                    Ok((copied, skipped)) => {
                        self.message = Some(format!(
                            "Imported {copied} files; {skipped} already present"
                        ))
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        }
        if let Some(error) = self.store.error() {
            self.error = Some(error);
        }
    }
    fn shortcut(&self, spec: &review_commands::Spec) -> Option<egui::KeyboardShortcut> {
        review_commands::parse(
            self.settings
                .bindings
                .get(spec.id)
                .map_or(spec.key, String::as_str),
        )
    }
    fn button_text(&self, label: &str, command: Command) -> String {
        COMMANDS.iter().find(|s| s.command == command).map_or_else(
            || label.into(),
            |s| {
                format!(
                    "{label} [{}]",
                    self.settings
                        .bindings
                        .get(s.id)
                        .map_or(s.key, String::as_str)
                )
            },
        )
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.settings_open
            || self.presets_open
            || self.import_open
            || self.camera_picker
            || self.help_open
            || self.palette_open
        {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.command(Command::Cancel);
            }
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let commands: Vec<_> = COMMANDS
            .iter()
            .filter(|s| s.command != Command::Blink)
            .filter_map(|s| {
                self.shortcut(s)
                    .filter(|key| ctx.input_mut(|i| review_commands::consume(&mut i.events, key)))
                    .map(|_| s.command)
            })
            .collect();
        for command in commands {
            self.command(command);
        }
    }
    fn view_image(&self, index: usize, edge: u32) -> ViewImage {
        let shot = self.session.as_ref().and_then(|s| s.shots.get(index));
        let path = shot.and_then(|s| s.preview()).map(|p| p.to_owned());
        let decision = shot.map_or(Decision::Unreviewed, |shot| {
            shot.decision(shot.review_kind(self.media_filter), self.settings.link_raw)
        });
        ViewImage {
            name: shot.map_or_else(|| "No image".into(), |s| s.name.clone()),
            status_color: theme::DecisionPalette::new(self.settings.colourblind).color(decision),
            status_label: decision.label(),
            key: path.clone().map(|p| {
                if self.canvas.native() {
                    if self.canvas.needs_focus() {
                        Key::focus(p)
                    } else {
                        Key::native(p)
                    }
                } else {
                    Key::fit(p, edge)
                }
            }),
            fallback: path.map(|p| Key::fit(p, edge)),
        }
    }
    fn plan_demands(&mut self, edge: u32) {
        let Some(session) = &self.session else {
            return;
        };
        let selected_position = self
            .visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0);
        let mut indices = Vec::new();
        if let Some(a) = self.pinned {
            indices.push((a, 0));
        }
        indices.push((self.selected, 0));
        for offset in 1..=8 {
            for direction in [self.last_direction, -self.last_direction] {
                let position = selected_position.saturating_add_signed(direction * offset);
                if let Some(index) = self.visible.get(position) {
                    indices.push((
                        *index,
                        if direction == self.last_direction {
                            1
                        } else {
                            2
                        },
                    ));
                }
            }
        }
        let mut fetch = Vec::new();
        for (index, priority) in indices {
            if let Some(shot) = session.shots.get(index) {
                if let Some(path) = shot.preview() {
                    self.demands
                        .push((Key::fit(path.to_owned(), edge), priority));
                    // Native buffers are large. Keep neighbors inside the configured tier.
                    let native_slots =
                        (self.settings.cpu_cache_mib * 1024 * 1024 * 3 / 4 / (160 * 1024 * 1024))
                            .saturating_sub(2)
                            / 2;
                    let focus_neighbor = self.canvas.needs_focus()
                        && (priority == 0
                            || self
                                .visible
                                .iter()
                                .position(|i| *i == index)
                                .is_some_and(|p| p.abs_diff(selected_position) <= 2));
                    if self.canvas.native()
                        && !focus_neighbor
                        && (priority == 0
                            || self
                                .visible
                                .iter()
                                .position(|i| *i == index)
                                .is_some_and(|p| {
                                    p.abs_diff(selected_position) <= native_slots.min(8)
                                }))
                    {
                        self.demands.push((Key::native(path.to_owned()), priority));
                    }
                    if focus_neighbor {
                        self.demands.push((Key::focus(path.to_owned()), priority));
                    }
                }
                if matches!(session.source, Source::Mtp { .. }) && shot.has(Kind::Jpeg) {
                    fetch.push(shot.id.clone());
                }
            }
        }
        if self.settings.group_bursts {
            let before = self.features.len();
            for shot in &session.shots {
                if let Some(path) = shot.preview() {
                    let key = Key::fit(path.to_owned(), 512);
                    if let Some(feature) = self.cache.feature(&key) {
                        self.features.insert(path.to_owned(), feature);
                    }
                    if !self.features.contains_key(path) {
                        self.demands.push((key, 4));
                    }
                }
            }
            if before != self.features.len() || self.burst_groups.is_empty() {
                self.burst_groups = crate::bursts::group(
                    &session.shots,
                    &self.features,
                    self.settings.burst_window_ms,
                    self.settings.burst_distance,
                );
            }
        } else {
            self.burst_groups.clear();
        }
        let request = (
            fetch,
            self.staging_paused || self.camera_busy,
            self.settings.disk_cache_gib * 1024 * 1024 * 1024,
        );
        if matches!(session.source, Source::Mtp { .. })
            && self.staging_request.as_ref() != Some(&request)
        {
            self.send(MtpRequest::SetPriority {
                shot_ids: request.0.clone(),
                paused: request.1,
                disk_limit: request.2,
            });
            self.staging_request = Some(request);
        }
    }
    fn command_button(&mut self, ui: &mut egui::Ui, label: &str, command: Command) {
        let response = if let Some(icon) = theme::command_icon(command) {
            theme::icon_button(ui, label, icon)
        } else {
            ui.button(label)
        };
        if response
            .on_hover_text(self.button_text(label, command))
            .clicked()
        {
            self.command(command);
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(25.0, 25.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 7, theme::ACCENT_BG);
            painter.rect_stroke(
                rect.shrink(6.0),
                3,
                egui::Stroke::new(1.5, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(rect.center(), 3.0, egui::Stroke::new(1.5, theme::ACCENT));
            ui.label(egui::RichText::new("mtp-cull").size(17.0).strong());
            ui.add_space(14.0);
            self.command_button(ui, "Open folder", Command::Open);
            self.command_button(ui, "Camera", Command::Camera);
            if !self.settings.recent_sources.is_empty() {
                ui.menu_button("Recent", |ui| {
                    self.recent_sources_ui(ui, recent_sources::LIMIT, false);
                    ui.separator();
                    if ui.button("Clear recent sources").clicked() {
                        self.settings.recent_sources.clear();
                        self.draft_settings.recent_sources.clear();
                        self.store.save_settings(&self.settings);
                        ui.close();
                    }
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("More", |ui| {
                    for (label, command) in [
                        ("Settings", Command::Settings),
                        ("Import presets", Command::Presets),
                        ("Keyboard shortcuts", Command::Help),
                        ("Command palette", Command::Palette),
                    ] {
                        if ui.button(self.button_text(label, command)).clicked() {
                            self.command(command);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("Close session"))
                        .clicked()
                    {
                        self.command(Command::Close);
                        ui.close();
                    }
                });
                if ui
                    .add_enabled(
                        self.session.is_some() && !self.camera_busy && self.import.is_none(),
                        egui::Button::new(
                            egui::RichText::new("Import selected")
                                .color(theme::BACKGROUND)
                                .strong(),
                        )
                        .fill(theme::ACCENT)
                        .stroke(egui::Stroke::NONE),
                    )
                    .on_hover_text(self.button_text(
                        "Copy selected originals to your destinations",
                        Command::Import,
                    ))
                    .clicked()
                {
                    self.command(Command::Import);
                }
            });
        });
        if self.session.is_none() {
            return;
        }
        ui.add_space(5.0);
        let palette = theme::DecisionPalette::new(self.settings.colourblind);
        ui.add_enabled_ui(self.session.is_some(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                theme::group(ui, |ui| {
                    for (label, command, color, fill) in [
                        ("Keep", Command::Keep, palette.keep, palette.keep_bg),
                        ("Reject", Command::Reject, palette.reject, palette.reject_bg),
                    ] {
                        let binding = COMMANDS
                            .iter()
                            .find(|s| s.command == command)
                            .map(|s| {
                                self.settings
                                    .bindings
                                    .get(s.id)
                                    .map_or(s.key, String::as_str)
                            })
                            .unwrap_or_default();
                        let response = ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("     {label}"))
                                        .color(color)
                                        .strong(),
                                )
                                .fill(fill)
                                .stroke(egui::Stroke::NONE)
                                .shortcut_text(binding),
                            )
                            .on_hover_text(self.button_text(label, command));
                        theme::button_icon(
                            ui,
                            &response,
                            theme::command_icon(command).unwrap(),
                            color,
                        );
                        if response.clicked() {
                            self.command(command);
                        }
                    }
                    self.command_button(ui, "Unreviewed", Command::Clear);
                    self.command_button(ui, "Undo", Command::Undo);
                });
                theme::group(ui, |ui| {
                    self.command_button(ui, "Pin A", Command::Pin);
                    ui.add_enabled_ui(self.pinned.is_some(), |ui| {
                        self.command_button(ui, "Swap", Command::Swap)
                    });
                });
                theme::group(ui, |ui| {
                    for (mode, label) in [
                        (Mode::Single, "Single"),
                        (Mode::SideBySide, "Side by side"),
                        (Mode::Wipe, "Wipe"),
                    ] {
                        let response =
                            theme::tab(ui, self.canvas.mode == mode, &format!("     {label}"))
                                .on_hover_text(
                                    self.button_text("Cycle comparison mode", Command::Compare),
                                );
                        theme::button_icon(
                            ui,
                            &response,
                            match mode {
                                Mode::Single => theme::Icon::Single,
                                Mode::SideBySide => theme::Icon::Compare,
                                Mode::Wipe => theme::Icon::Wipe,
                            },
                            theme::TEXT,
                        );
                        if response.clicked() {
                            if mode != Mode::Single && self.pinned.is_none() {
                                self.command(Command::Pin);
                            }
                            self.canvas.mode = mode;
                        }
                    }
                });
                theme::group(ui, |ui| {
                    let label = match self.canvas.viewport.zoom {
                        Some(zoom) => format!("{:.0}%", zoom * 100.0),
                        None => "Fit".into(),
                    };
                    self.command_button(ui, &label, Command::Zoom);
                    ui.menu_button(
                        if self.canvas.peaking || self.canvas.region_tool {
                            "Focus •"
                        } else {
                            "Focus"
                        },
                        |ui| {
                            ui.set_min_width(230.0);
                            ui.strong("Focus inspection");
                            ui.checkbox(&mut self.canvas.peaking, "Sharpness overlay")
                                .on_hover_text(
                                    self.button_text("Toggle focus peaking", Command::Peaking),
                                );
                            ui.checkbox(&mut self.canvas.region_tool, "Compare a region")
                                .on_hover_text(
                                    self.button_text("Toggle comparison region", Command::Region),
                                );
                            ui.separator();
                            ui.add(
                                egui::Slider::new(&mut self.canvas.threshold, 1..=100)
                                    .text("Threshold"),
                            );
                            ui.add(
                                egui::Slider::new(&mut self.canvas.opacity, 20..=240)
                                    .text("Opacity"),
                            );
                            ui.separator();
                            self.command_button(ui, "Clear region", Command::ClearRegion);
                            ui.label(
                                egui::RichText::new("Hold the blink shortcut to alternate A / B.")
                                    .small()
                                    .color(theme::MUTED),
                            )
                            .on_hover_text(self.button_text("Hold to blink", Command::Blink));
                        },
                    );
                });
            });
        });
    }
    fn filters(&mut self, ui: &mut egui::Ui, position: Position) {
        if position.is_side() {
            ui.horizontal(|ui| self.media_filters(ui));
            ui.horizontal_wrapped(|ui| {
                self.decision_filter_control(ui);
                self.reel_layout_controls(ui, position);
            });
            ui.horizontal_wrapped(|ui| self.reel_counts(ui));
            if self.settings.reel_grid {
                ui.horizontal(|ui| self.reel_thumbnail_control(ui));
            }
        } else {
            ui.horizontal(|ui| {
                self.media_filters(ui);
                self.decision_filter_control(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.reel_layout_controls(ui, position);
                    if self.settings.reel_grid {
                        self.reel_thumbnail_control(ui);
                    }
                    self.reel_counts(ui);
                });
            });
        }
    }
    fn media_filters(&mut self, ui: &mut egui::Ui) {
        theme::group(ui, |ui| {
            for (kind, label) in [
                (Some(Kind::Jpeg), "JPEG"),
                (Some(Kind::Raw), "RAW"),
                (Some(Kind::Video), "Video"),
                (None, "All"),
            ] {
                if theme::tab(ui, self.media_filter == kind, label).clicked() {
                    self.media_filter = kind;
                    self.refresh_visible();
                }
            }
        });
    }
    fn decision_filter_control(&mut self, ui: &mut egui::Ui) {
        let before = self.decision_filter;
        egui::ComboBox::from_id_salt("decision-filter")
            .selected_text(match self.decision_filter {
                None => "All decisions",
                Some(Decision::Keep) => "Kept",
                Some(Decision::Reject) => "Rejected",
                Some(Decision::Unreviewed) => "Unreviewed",
            })
            .width(125.0)
            .show_ui(ui, |ui| {
                for (filter, label) in [
                    (None, "All decisions"),
                    (Some(Decision::Unreviewed), "Unreviewed"),
                    (Some(Decision::Keep), "Kept"),
                    (Some(Decision::Reject), "Rejected"),
                ] {
                    ui.selectable_value(&mut self.decision_filter, filter, label);
                }
            });
        if self.decision_filter != before {
            self.refresh_visible();
        }
    }
    fn reel_layout_controls(&mut self, ui: &mut egui::Ui, position: Position) {
        if theme::icon_button(
            ui,
            if !self.settings.reel_grid {
                "Grid"
            } else if position.is_side() {
                "Strip"
            } else {
                "Row"
            },
            if self.settings.reel_grid {
                theme::Icon::Row
            } else {
                theme::Icon::Grid
            },
        )
        .on_hover_text(
            self.button_text("Switch between a single strip and grid", Command::ReelMode),
        )
        .clicked()
        {
            self.command(Command::ReelMode);
        }
        ui.menu_button("Reel", |ui| {
            ui.label("Position");
            for next in Position::ALL {
                if ui
                    .selectable_label(self.settings.reel_position == next, next.label())
                    .clicked()
                {
                    self.set_reel_position(next);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(self.button_text("Move reel", Command::ReelPosition));
    }
    fn reel_thumbnail_control(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 45.0).clamp(100.0, 150.0);
            let response = ui
                .add(
                    egui::Slider::new(
                        &mut self.settings.reel_thumbnail_size,
                        crate::reel::MIN_THUMBNAIL_WIDTH..=crate::reel::MAX_THUMBNAIL_WIDTH,
                    )
                    .show_value(false)
                    .text("Size"),
                )
                .on_hover_text(format!(
                    "Preferred thumbnail width. Grids fill the reel width.\n{}\n{}",
                    self.button_text("Smaller", Command::ReelSmaller),
                    self.button_text("Larger", Command::ReelLarger)
                ));
            #[cfg(feature = "ui-smoke")]
            {
                self.reel_size_slider = response.rect;
            }
            if response.changed() {
                self.draft_settings.reel_thumbnail_size = self.settings.reel_thumbnail_size;
            }
            if response.drag_stopped() {
                // Return mouse users to the culling shortcuts after adjusting size.
                // Tab-focused keyboard users can still edit the slider with arrows.
                response.surrender_focus();
            }
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                self.store.save_settings(&self.settings);
            }
        });
    }
    fn reel_counts(&self, ui: &mut egui::Ui) {
        let palette = theme::DecisionPalette::new(self.settings.colourblind);
        if self.reel_selection.indices.len() > 1 {
            ui.label(
                egui::RichText::new(format!("{} selected", self.reel_selection.indices.len()))
                    .color(palette.selection),
            );
        }
        let position = self
            .visible
            .iter()
            .position(|i| *i == self.selected)
            .map_or(0, |p| p + 1);
        ui.label(
            egui::RichText::new(format!("{position} / {}", self.visible.len())).color(theme::MUTED),
        );
        if let Some(session) = &self.session {
            let kept = session
                .shots
                .iter()
                .filter(|shot| {
                    shot.decision(shot.review_kind(self.media_filter), self.settings.link_raw)
                        == Decision::Keep
                })
                .count();
            ui.label(egui::RichText::new(format!("{kept} kept")).color(palette.keep));
        }
    }
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        if !self.camera_busy
            && self.import.is_none()
            && let Some(message) = &self.message
        {
            ui.add(
                egui::Label::new(egui::RichText::new(message).small().color(theme::MUTED))
                    .truncate(),
            )
            .on_hover_text(message);
        }
        if let Some(error) = self.error.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(theme::REJECT, error);
                if ui.small_button("Dismiss").clicked() {
                    self.error = None;
                }
            });
        }
        let progress = if let Some(import) = &self.import {
            Some((import.progress, import.text.clone()))
        } else if self.camera_busy {
            Some((
                self.camera_progress,
                self.message
                    .clone()
                    .unwrap_or_else(|| "Connecting to camera…".into()),
            ))
        } else {
            None
        };
        if let Some((value, text)) = progress {
            ui.horizontal(|ui| {
                ui.add(
                    egui::ProgressBar::new(value)
                        .desired_width((ui.available_width() - 90.0).max(80.0))
                        .desired_height(18.0)
                        .text(text),
                );
                self.command_button(ui, "Cancel", Command::Cancel);
            });
        }
        ui.horizontal(|ui| {
            let source = self.session.as_ref().map(|session| match &session.source {
                Source::Local { root, raw } => (
                    root.file_name()
                        .unwrap_or(root.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    format!(
                        "{}{}",
                        root.display(),
                        raw.as_ref()
                            .map(|p| format!(" · RAW {}", p.display()))
                            .unwrap_or_default()
                    ),
                ),
                Source::Mtp { device, folder } => {
                    let name = self
                        .devices
                        .iter()
                        .find(|d| d.id == *device)
                        .map_or("Camera", |d| d.name.as_str());
                    let path = self
                        .folders
                        .iter()
                        .find(|f| f.id == *folder)
                        .map_or("Selected folder", |f| f.path.as_str());
                    (name.into(), format!("{name} · {path}"))
                }
            });
            if self.loader.is_some() {
                ui.spinner();
                ui.weak("Opening photos…");
            } else if let Some((name, detail)) = source {
                ui.add(
                    egui::Label::new(egui::RichText::new(name).small().color(theme::MUTED))
                        .truncate(),
                )
                .on_hover_text(detail);
                if !self.staging_text.is_empty() {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(&self.staging_text).small().color(
                        if self.staging_active {
                            theme::ACCENT
                        } else {
                            theme::MUTED
                        },
                    ));
                }
            } else {
                ui.weak("Ready to review");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("Performance", |ui| {
                    ui.set_min_width(230.0);
                    ui.strong("Preview cache");
                    ui.label(format!(
                        "CPU {:.0} MiB · GPU {:.0} MiB",
                        self.cache.bytes() as f64 / 1048576.0,
                        self.canvas.bytes() as f64 / 1048576.0
                    ));
                    ui.label(format!(
                        "UI {:.2} ms · mean decode {:.1} ms",
                        self.frame_ms,
                        self.cache.stats.decode_ms / self.cache.stats.decodes.max(1) as f64
                    ));
                });
                let notice = self
                    .message
                    .as_deref()
                    .unwrap_or("Changes save automatically. Use the shortcuts to review faster.");
                if self.session.is_some() {
                    ui.label(
                        egui::RichText::new("Decisions saved")
                            .small()
                            .color(theme::MUTED),
                    )
                    .on_hover_text(notice);
                }
                if self.staging_active || self.staging_paused {
                    self.command_button(
                        ui,
                        if self.staging_paused {
                            "Resume staging"
                        } else {
                            "Pause staging"
                        },
                        Command::Pause,
                    );
                }
                if self.worker.is_some() && !self.camera_busy {
                    self.command_button(ui, "Retry", Command::Retry);
                }
            });
        });
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.settings_open {
            let mut open = true;
            egui::Window::new("Settings")
                .open(&mut open)
                .default_width(650.0)
                .default_height(560.0)
                .min_height(320.0)
                .max_height((ctx.content_rect().height() - 100.0).max(240.0))
                .show(ctx, |ui| {
                    theme::group(ui, |ui| {
                        for (tab, label) in [(0, "Review"), (1, "Performance"), (2, "Shortcuts")] {
                            if theme::tab(ui, self.settings_tab == tab, label).clicked() { self.settings_tab = tab; }
                        }
                    });
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .id_salt(("settings-content", self.settings_tab))
                        .auto_shrink([false, false])
                        .max_height((ui.available_height() - 90.0).clamp(80.0, 500.0))
                        .show(ui, |ui| {
                            match self.settings_tab {
                                0 => {
                                    ui.label(egui::RichText::new("Decision colours").strong());
                                    ui.checkbox(&mut self.draft_settings.colourblind, "Colourblind-friendly colours");
                                    let palette = theme::DecisionPalette::new(self.draft_settings.colourblind);
                                    ui.horizontal(|ui| {
                                        for decision in [Decision::Keep, Decision::Reject, Decision::Unreviewed] {
                                            ui.colored_label(palette.color(decision), decision.label());
                                        }
                                    });
                                    ui.label(egui::RichText::new("When enabled, Keep is blue, Reject orange and Unreviewed gray. Editing selection uses white outlines. Labels and decision icons remain visible.").small().color(theme::MUTED));
                                    ui.separator();
                                    ui.checkbox(&mut self.draft_settings.link_raw, "Link JPEG selections to matching RAW files");
                                    ui.label(egui::RichText::new("Independent RAW choices return when linking is turned off.").small().color(theme::MUTED));
                                    ui.checkbox(&mut self.draft_settings.include_videos, "Include videos in import");
                                    ui.checkbox(&mut self.draft_settings.auto_advance, "Advance after deciding candidate B");
                                    ui.add_space(8.0);
                                    ui.separator();
                                    ui.checkbox(&mut self.draft_settings.group_bursts, "Group similar captures into bursts");
                                    ui.add(egui::Slider::new(&mut self.draft_settings.burst_window_ms, 100..=10000).text("Burst interval (ms)"));
                                    ui.add(egui::Slider::new(&mut self.draft_settings.burst_distance, 0..=32).text("Similarity distance"));
                                    ui.separator();
                                    ui.label(egui::RichText::new("Film reel").strong());
                                    ui.horizontal(|ui| {
                                        ui.label("Position");
                                        egui::ComboBox::from_id_salt("reel-position-setting")
                                            .selected_text(self.draft_settings.reel_position.label())
                                            .show_ui(ui, |ui| {
                                                for position in Position::ALL {
                                                    ui.selectable_value(&mut self.draft_settings.reel_position, position, position.label());
                                                }
                                            });
                                    });
                                    ui.checkbox(&mut self.draft_settings.reel_grid, "Show a grid instead of a single strip");
                                    ui.add(egui::Slider::new(&mut self.draft_settings.reel_thumbnail_size, crate::reel::MIN_THUMBNAIL_WIDTH..=crate::reel::MAX_THUMBNAIL_WIDTH).text("Preferred thumbnail width"));
                                    ui.label(egui::RichText::new("Grids fill the reel width while keeping thumbnails near this size. Adjust Size above the reel to change it while reviewing.").small().color(theme::MUTED));
                                    ui.add(egui::Slider::new(&mut self.draft_settings.reel_scroll_speed, 0.25..=8.0).text("Reel scroll speed"));
                                    ui.label(egui::RichText::new("Drag the edge beside the viewer to resize. The single strip scrolls horizontally at the bottom and vertically at either side. Ctrl-click selects a batch; Shift-click selects a range.").small().color(theme::MUTED));
                                    if let Some(session) = &self.session {
                                        ui.label(egui::RichText::new(format!("{} assets selected for import with these settings", session.selected_ids(&self.draft_settings).len())).small().color(theme::MUTED));
                                    }
                                }
                                1 => {
                                    ui.label(egui::RichText::new("Preview memory and camera staging").strong());
                                    ui.label(egui::RichText::new("Larger caches keep more nearby images ready for inspection.").small().color(theme::MUTED));
                                    ui.add_space(8.0);
                                    ui.add(egui::Slider::new(&mut self.draft_settings.cpu_cache_mib, 256..=16384).text("CPU cache (MiB)"));
                                    ui.add(egui::Slider::new(&mut self.draft_settings.gpu_cache_mib, 128..=2048).text("GPU cache (MiB)"));
                                    ui.add(egui::Slider::new(&mut self.draft_settings.disk_cache_gib, 1..=128).text("Staging quota (GiB)"));
                                    ui.label(egui::RichText::new("Staged JPEG originals stay on disk for the next review. Older inactive caches can be evicted to make room.").small().color(theme::MUTED));
                                    ui.separator();
                                    ui.label(egui::RichText::new("Image sampling").strong());
                                    egui::ComboBox::from_id_salt("preview-sampling")
                                        .selected_text(self.draft_settings.sampling.label())
                                        .show_ui(ui, |ui| {
                                            for sampling in [crate::viewer::Sampling::Smooth, crate::viewer::Sampling::Linear, crate::viewer::Sampling::Nearest] {
                                                ui.selectable_value(&mut self.draft_settings.sampling, sampling, sampling.label());
                                            }
                                        });
                                    ui.label(egui::RichText::new("Smooth uses mipmaps to reduce moiré when zoomed out (about 33% extra image texture memory). Linear blends neighboring pixels using less memory, but can show moiré when zoomed out. Nearest neighbor shows pixels without interpolation.").small().color(theme::MUTED));
                                }
                                _ => {
                                    ui.label(egui::RichText::new("Edit a binding, then save. Conflicts are highlighted below.").small().color(theme::MUTED));
                                    egui::Grid::new("shortcut-settings").num_columns(2).spacing(egui::vec2(16.0, 8.0)).striped(true).show(ui, |ui| {
                                        for spec in COMMANDS {
                                            ui.label(spec.label);
                                            let binding = self.draft_settings.bindings.entry(spec.id.into()).or_insert_with(|| spec.key.into());
                                            ui.add(egui::TextEdit::singleline(binding).desired_width(180.0));
                                            ui.end_row();
                                        }
                                    });
                                }
                            }
                        });
                    ui.separator();
                    let validation = review_commands::validate(&self.draft_settings.bindings);
                    if let Err(error) = &validation { ui.colored_label(theme::REJECT, error); }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(validation.is_ok(), egui::Button::new(egui::RichText::new("Save changes").color(theme::BACKGROUND).strong()).fill(theme::ACCENT)).clicked() {
                            self.settings = self.draft_settings.clone();
                            self.save();
                            self.refresh_visible();
                            self.settings_open = false;
                            self.burst_groups.clear();
                        }
                        if ui.button("Cancel").clicked() { self.settings_open = false; }
                    });
                });
            if !open {
                self.settings_open = false;
            }
        }
        if self.help_open {
            let mut open = true;
            egui::Window::new("Keyboard and comparison help").open(&mut open).show(ctx,|ui|{
                ui.label("Wheel: zoom at cursor · Drag: shared pan · Alt+drag B: manual alignment");
                ui.label("Pin A, browse B; Tab selects which pane receives Keep/Reject.");
                ui.label("Ctrl-click reel thumbnails for a batch; Shift-click for a range. Ctrl+A selects visible images. Keep/Reject/Unreviewed apply to the batch, or to pinned A when A is active.");
                ui.label("Drag the reel edge to resize; Ctrl+G switches strip/grid. Ctrl+Shift+G cycles Bottom/Left/Right placement. Wheel scrolls the reel without Shift. Settings > Review sets placement and scroll speed; Performance sets sampling.");
                ui.label("Region mode: drag a crop. Focus scores are hints, not automatic decisions.");
                egui::ScrollArea::vertical().max_height(500.0).show(ui,|ui|{
                    for spec in COMMANDS{ui.horizontal(|ui|{ui.monospace(self.settings.bindings.get(spec.id).map_or(spec.key,String::as_str));ui.label(spec.label);});}
                });
            });
            self.help_open = open;
        }
        if self.palette_open {
            let mut chosen = None;
            let mut open = true;
            egui::Window::new("Commands")
                .open(&mut open)
                .show(ctx, |ui| {
                    let (submit, down, up) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::ArrowDown),
                            i.key_pressed(egui::Key::ArrowUp),
                        )
                    });
                    let response = ui.text_edit_singleline(&mut self.palette_query);
                    if self.palette_focus {
                        response.request_focus();
                        self.palette_focus = false;
                    }
                    if response.changed() {
                        self.palette_selection = 0;
                    }
                    let matching = COMMANDS
                        .iter()
                        .filter(|s| {
                            s.label
                                .to_lowercase()
                                .contains(&self.palette_query.to_lowercase())
                        })
                        .collect::<Vec<_>>();
                    if down {
                        self.palette_selection = self.palette_selection.saturating_add(1);
                    }
                    if up {
                        self.palette_selection = self.palette_selection.saturating_sub(1);
                    }
                    self.palette_selection =
                        self.palette_selection.min(matching.len().saturating_sub(1));
                    if submit {
                        chosen = matching.get(self.palette_selection).map(|s| s.command);
                    }
                    ui.small("Type to filter · Up/Down to choose · Enter to run · Escape to close");
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| {
                            for (index, spec) in matching.iter().enumerate() {
                                if ui
                                    .selectable_label(
                                        index == self.palette_selection,
                                        format!(
                                            "{}    {}",
                                            spec.label,
                                            self.settings
                                                .bindings
                                                .get(spec.id)
                                                .map_or(spec.key, String::as_str)
                                        ),
                                    )
                                    .clicked()
                                {
                                    chosen = Some(spec.command);
                                }
                            }
                        });
                });
            self.palette_open = open;
            if let Some(command) = chosen {
                self.palette_open = false;
                self.command(command);
            }
        }
        self.camera_dialog(ctx);
        self.import_dialog(ctx);
        self.presets_dialog(ctx);
    }
    fn camera_dialog(&mut self, ctx: &egui::Context) {
        if !self.camera_picker {
            return;
        }
        let mut open = true;
        egui::Window::new("Camera source")
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Connected devices");
                    if ui
                        .add_enabled(!self.camera_busy, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.open_camera(self.pending_camera.clone());
                    }
                });
                for device in self.devices.clone() {
                    if ui
                        .add_enabled(
                            !self.camera_busy,
                            egui::Button::new(&device.name)
                                .selected(self.device.as_ref() == Some(&device.id)),
                        )
                        .clicked()
                    {
                        self.pending_camera = self
                            .settings
                            .recent_sources
                            .iter()
                            .find(|source| {
                                source
                                    .device(&self.devices)
                                    .is_some_and(|item| item.id == device.id)
                            })
                            .cloned()
                            .map(|source| (source, false));
                        self.select_camera(device.id);
                    }
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for folder in &self.folders {
                            let label = if folder.path.is_empty() {
                                "Device root"
                            } else {
                                &folder.path
                            };
                            if ui
                                .add_enabled(
                                    !self.camera_busy,
                                    egui::Button::new(label)
                                        .selected(self.folder.as_ref() == Some(&folder.id)),
                                )
                                .clicked()
                            {
                                self.folder = Some(folder.id.clone());
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if self.camera_busy {
                        ui.spinner();
                    }
                    if let Some(message) = &self.message {
                        ui.weak(message);
                    }
                });
                if ui
                    .add_enabled(
                        !self.camera_busy && self.device.is_some() && self.folder.is_some(),
                        egui::Button::new("Start review"),
                    )
                    .clicked()
                {
                    self.start_camera_review();
                }
            });
        if !open {
            self.command(Command::Cancel);
        }
    }
    fn import_dialog(&mut self, ctx: &egui::Context) {
        if !self.import_open {
            return;
        }
        let mut open = true;
        let _shown = egui::Window::new("Import selected originals")
            .open(&mut open)
            .default_width(650.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(325.0, 230.0))
            .show(ctx, |ui| {
                #[cfg(feature = "ui-smoke")]
                { self.import_ui_state = (ui.is_visible(), ui.is_sizing_pass()); }
                let mut chosen = None;
                ui.horizontal(|ui| {
                    ui.label("Destination preset");
                    egui::ComboBox::from_id_salt("review-destination-preset")
                        .selected_text("Choose a saved preset...")
                        .show_ui(ui, |ui| {
                            for preset in &self.presets {
                                if ui.button(&preset.name).on_hover_text(preset.detail()).clicked() {
                                    chosen = Some(preset.clone());
                                }
                            }
                            if self.presets.is_empty() { ui.weak("No saved presets yet"); }
                        });
                    if ui.button("Save as preset...").clicked() {
                        self.preset_edit = Some((None, self.review_import_preset()));
                        self.presets_open = true;
                        self.preset_from_review = true;
                        self.import_open = false;
                    }
                });
                if let Some(preset) = chosen { self.apply_review_destinations(&preset); }
                ui.weak("Presets fill destinations and album; your date and review choices stay as set.");
                if let Some(error) = &self.preset_error { ui.colored_label(theme::REJECT, error); }
                ui.separator();
                folder_field(ui, "JPEG destination", &mut self.settings.picture_root);
                folder_field(ui, "RAW destination", &mut self.settings.raw_root);
                folder_field(ui, "Video destination", &mut self.settings.video_root);
                ui.horizontal(|ui| {
                    ui.label("Album");
                    ui.text_edit_singleline(&mut self.settings.album);
                });
                ui.horizontal(|ui| {
                    ui.label("Date");
                    ui.text_edit_singleline(&mut self.date);
                });
                ui.checkbox(&mut self.settings.include_videos, "Include videos");
                ui.weak("Layout: destination / year / date album / filename");
                let selected = self
                    .session
                    .as_ref()
                    .map(|s| s.selected_ids(&self.settings))
                    .unwrap_or_default();
                let mut counts = [0_usize; 4];
                let mut unreviewed = 0;
                if let Some(session) = &self.session {
                    let conflicts=session.shots.iter().filter(|s|s.conflict()).count();
                    let missing=session.shots.iter().filter(|s|s.has(Kind::Jpeg) && !s.has(Kind::Raw)).count();
                    if conflicts>0 {ui.colored_label(egui::Color32::YELLOW,format!("{conflicts} ambiguous JPEG/RAW pairs excluded. Resolve duplicate stems in the source folders before importing."));}
                    if self.settings.link_raw && missing>0 {ui.label(format!("{missing} JPEGs have no RAW companion."));}
                    for shot in &session.shots {
                        for asset in &shot.assets {
                            if selected.contains(&asset.id) {
                                counts[match asset.kind {
                                    Kind::Jpeg => 0,
                                    Kind::Raw => 1,
                                    Kind::Video => 2,
                                    Kind::Other => 3,
                                }] += 1;
                            }
                            if asset.kind == Kind::Jpeg && asset.decision == Decision::Unreviewed {
                                unreviewed += 1;
                            }
                        }
                    }
                }
                ui.label(format!(
                    "{} JPEG · {} RAW · {} video · {} other",
                    counts[0], counts[1], counts[2], counts[3]
                ));
                ui.label(format!(
                    "{unreviewed} unreviewed JPEGs excluded. Originals on the source remain intact."
                ));
                if ui
                    .add_enabled(
                        !selected.is_empty() && self.import.is_none() && !self.camera_busy && self.loader.is_none(),
                        egui::Button::new("Copy selected files"),
                    )
                    .clicked()
                {
                    self.start_import(selected);
                    self.import_open = false;
                }
            });
        #[cfg(feature = "ui-smoke")]
        {
            self.import_area = _shown.map(|shown| (shown.response.rect, shown.response.layer_id));
        }
        if !open {
            self.import_open = false;
        }
    }
    fn apply_review_destinations(&mut self, preset: &ImportPreset) {
        self.settings.picture_root = preset.pictures_path.clone();
        self.settings.raw_root = preset.raw_path.clone();
        self.settings.video_root = preset.videos_path.clone();
        self.settings.album = preset.album_name.clone();
    }
    fn review_import_preset(&self) -> ImportPreset {
        let mut preset = ImportPreset {
            pictures_path: self.settings.picture_root.clone(),
            raw_path: self.settings.raw_root.clone(),
            videos_path: self.settings.video_root.clone(),
            album_name: self.settings.album.clone(),
            // A reusable profile should not lock future sessions to this date.
            ..Default::default()
        };
        if let Some(session) = &self.session
            && let Source::Mtp { device, folder } = &session.source
            && let Some(RecentSource::Camera {
                device_name,
                folder_path,
                ..
            }) = self.settings.recent_sources.iter().find(|source| {
                matches!(source,
                    RecentSource::Camera { device_id, folder_id, .. }
                        if device_id == device && folder_id == folder)
            })
        {
            preset.device = device_name.clone();
            preset.source_path = folder_path.clone();
        }
        preset
    }
    fn start_import(&mut self, selected: Vec<String>) {
        self.save();
        let destinations = match self.destinations() {
            Ok(d) => d,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let Some(session) = &self.session else {
            return;
        };
        if matches!(session.source, Source::Mtp { .. }) {
            self.camera_busy = true;
            self.send(MtpRequest::ImportAssets {
                object_ids: selected,
                destinations,
            });
            return;
        }
        let assets = session
            .shots
            .iter()
            .flat_map(|s| &s.assets)
            .filter(|a| selected.contains(&a.id))
            .cloned()
            .collect::<Vec<_>>();
        self.import = Some(crate::imports::start(assets, destinations));
    }

    fn destinations(&self) -> Result<ImportPaths> {
        let date = NaiveDate::parse_from_str(&self.date, "%Y-%m-%d")?;
        Ok(ImportPaths {
            pictures: optional_path(&self.settings.picture_root),
            raw: optional_path(&self.settings.raw_root),
            videos: optional_path(&self.settings.video_root),
            date,
            album_name: (!self.settings.album.trim().is_empty())
                .then(|| self.settings.album.trim().to_owned()),
        })
    }
}
impl eframe::App for App {
    #[cfg(feature = "ui-smoke")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(smoke) = &mut self.smoke
            && smoke.input_ready()
        {
            raw.events.append(&mut smoke.input);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.canvas.begin_frame();
        let start = Instant::now();
        let ctx = ui.ctx().clone();
        self.poll();
        self.shortcuts(&ctx);
        self.demands.clear();
        self.cache
            .set_budget(self.settings.cpu_cache_mib * 1024 * 1024);
        self.canvas
            .set_budget(self.settings.gpu_cache_mib * 1024 * 1024);
        self.canvas.set_sampling(self.settings.sampling);
        let edge = (ctx.content_rect().height() * ctx.pixels_per_point())
            .ceil()
            .clamp(800.0, 4320.0) as u32;
        // The edge must accommodate the landscape image width at the display's height.
        let edge = (edge * 3 / 2).div_ceil(128) * 128;
        self.plan_demands(edge);
        let pins = self
            .demands
            .iter()
            .filter(|(_, priority)| *priority == 0)
            .map(|(key, _)| key.clone())
            .collect();
        self.cache.poll(&pins);
        egui::Panel::top("toolbar")
            .frame(theme::panel())
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .frame(theme::panel().inner_margin(egui::Margin::symmetric(14, 4)))
            .show(ui, |ui| self.status_bar(ui));
        if self.session.is_some() {
            // Keep this frame's orientation stable if the placement menu changes it.
            let position = self.settings.reel_position;
            let side = position.is_side();
            // Left/right share a width; bottom keeps its own height and panel state.
            let id = egui::Id::new(("filmstrip", side));
            let panel = match position {
                Position::Bottom => egui::Panel::bottom(id)
                    .default_size(self.settings.reel_height)
                    .size_range(155.0..=(ctx.content_rect().height() * 0.6).max(155.0)),
                Position::Left => egui::Panel::left(id)
                    .default_size(self.settings.reel_width)
                    .size_range(260.0..=(ctx.content_rect().width() * 0.5).max(260.0)),
                Position::Right => egui::Panel::right(id)
                    .default_size(self.settings.reel_width)
                    .size_range(260.0..=(ctx.content_rect().width() * 0.5).max(260.0)),
            };
            let reel_panel = panel.resizable(true).frame(theme::panel()).show(ui, |ui| {
                ui.add_space(3.0);
                self.filters(ui, position);
                ui.add_space(3.0);
                self.filmstrip(ui, position);
            });
            let rect = reel_panel.response.rect;
            let (grip, delta) = match position {
                Position::Bottom => (
                    rect.center_top() + egui::vec2(0.0, 1.5),
                    egui::vec2(15.0, 0.0),
                ),
                Position::Left => (
                    rect.right_center() - egui::vec2(1.5, 0.0),
                    egui::vec2(0.0, 15.0),
                ),
                Position::Right => (
                    rect.left_center() + egui::vec2(1.5, 0.0),
                    egui::vec2(0.0, 15.0),
                ),
            };
            ui.painter().line_segment(
                [grip - delta, grip + delta],
                egui::Stroke::new(2.0, theme::MUTED),
            );
            #[cfg(feature = "ui-smoke")]
            {
                self.reel_panel_bounds = rect;
            }
            if side {
                self.settings.reel_width = rect.width();
                self.draft_settings.reel_width = self.settings.reel_width;
            } else {
                self.settings.reel_height = rect.height();
                self.draft_settings.reel_height = self.settings.reel_height;
            }
            if ctx.input(|i| i.pointer.any_released()) {
                self.store.save_settings(&self.settings);
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(8))
            .show(ui, |ui| {
                if self.session.is_none() {
                    egui::ScrollArea::vertical()
                        .id_salt("welcome-content")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let recent_count = self.settings.recent_sources.len().min(5);
                            let preset_height = if self.presets.is_empty() {
                                65.0
                            } else {
                                95.0 + self.presets.len().min(5) as f32 * 34.0
                            };
                            let block_height = 180.0
                                + preset_height
                                + if recent_count > 0 {
                                    40.0 + recent_count as f32 * 34.0
                                } else {
                                    0.0
                                };
                            ui.add_space(((ui.available_height() - block_height) * 0.5).max(20.0));
                            ui.vertical_centered(|ui| {
                                ui.heading("Review your photos");
                                ui.add_space(8.0);
                                ui.label(
                            egui::RichText::new(
                                "Find the keepers. Compare the details. Import your favourites.",
                            )
                            .color(theme::MUTED),
                        );
                                ui.add_space(20.0);
                                let button_width: f32 = ["Connect camera", "Open a folder"]
                                    .iter()
                                    .map(|label| {
                                        ui.painter()
                                            .layout_no_wrap(
                                                (*label).into(),
                                                egui::TextStyle::Button.resolve(ui.style()),
                                                ui.visuals().text_color(),
                                            )
                                            .size()
                                            .x
                                            + 2.0 * ui.spacing().button_padding.x
                                    })
                                    .sum::<f32>()
                                    + ui.spacing().item_spacing.x;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(button_width, 32.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        self.command_button(ui, "Connect camera", Command::Camera);
                                        self.command_button(ui, "Open a folder", Command::Open);
                                    },
                                );
                                ui.add_space(14.0);
                                ui.label(
                                    egui::RichText::new(
                                        "JPEG + RAW linked · Videos included · Originals stay safe",
                                    )
                                    .small()
                                    .color(theme::MUTED),
                                );
                                if recent_count > 0 {
                                    ui.add_space(20.0);
                                    ui.label(
                                        egui::RichText::new("Recent sources")
                                            .small()
                                            .color(theme::MUTED),
                                    );
                                    ui.add_space(8.0);
                                    self.recent_sources_ui(ui, 5, true);
                                }
                                self.home_presets(ui);
                            });
                        });
                } else if self.visible.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label("No images match the current filters.");
                    });
                } else {
                    let a = self.pinned.map(|index| self.view_image(index, edge));
                    let b = self.view_image(self.selected, edge);
                    let blink = !ctx.egui_wants_keyboard_input()
                        && !self.settings_open
                        && !self.presets_open
                        && !self.import_open
                        && !self.camera_picker
                        && !self.palette_open
                        && COMMANDS
                            .iter()
                            .find(|s| s.command == Command::Blink)
                            .and_then(|s| self.shortcut(s))
                            .is_some_and(|s| {
                                ctx.input(|i| {
                                    i.key_down(s.logical_key)
                                        && i.modifiers.matches_exact(s.modifiers)
                                })
                            });
                    #[cfg(feature = "ui-smoke")]
                    let blink = blink || self.smoke.as_ref().is_some_and(|s| s.blink);
                    self.canvas.show(ui, &mut self.cache, a, b, blink);
                    if let Some(command) = self.canvas.context_command.take() {
                        self.command(command);
                    }
                }
            });
        self.dialogs(&ctx);
        if let Some(session) = &self.session {
            let native = self.canvas.native();
            let focus = self.canvas.needs_focus();
            let position = self
                .visible
                .iter()
                .position(|i| *i == self.selected)
                .unwrap_or(0);
            let keys = (1..=2)
                .filter_map(|offset| position.checked_add_signed(self.last_direction * offset))
                .filter_map(|p| self.visible.get(p))
                .filter_map(|i| session.shots[*i].preview())
                .map(|p| {
                    if native {
                        if focus {
                            Key::focus(p.to_owned())
                        } else {
                            Key::native(p.to_owned())
                        }
                    } else {
                        Key::fit(p.to_owned(), edge)
                    }
                });
            self.canvas.prefetch(&ctx, &mut self.cache, keys);
        }
        self.cache.demand(std::mem::take(&mut self.demands));
        if self.cache.pending()
            || self.loader.is_some()
            || self.import.is_some()
            || self.camera_busy
            || self.staging_active
            || !self.mtp_pending.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(10));
        }
        self.frame_ms = start.elapsed().as_secs_f64() * 1000.0;
        #[cfg(feature = "ui-smoke")]
        if let Some(mut smoke) = self.smoke.take() {
            if let Err(error) = smoke.tick(self, &ctx) {
                smoke.fail(&ctx, &format!("{error:#}"));
            }
            self.smoke = Some(smoke);
        }
    }
}

#[path = "preset_ui.rs"]
mod preset_ui;

#[cfg(feature = "ui-smoke")]
#[path = "ui_smoke.rs"]
mod smoke;
impl Drop for App {
    fn drop(&mut self) {
        self.save();
        if let Some(import) = &self.import {
            import.cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn optional_path(value: &str) -> Option<PathBuf> {
    (!value.trim().is_empty()).then(|| PathBuf::from(value.trim()))
}
fn folder_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value);
        if ui.button("Choose").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_folder()
        {
            *value = path.display().to_string();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_rows_follow_filtered_order_and_stop_at_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture_app(root.path());
        app.visible = vec![10, 12, 14, 16, 18, 20, 22];
        app.settings.reel_grid = true;
        app.reel_layout = Some((3, egui::vec2(100.0, 100.0)));
        app.selected = 12;
        app.command(Command::RowUp);
        assert_eq!(app.selected, 12, "top row must stay in its column");
        app.command(Command::RowDown);
        assert_eq!(app.selected, 18);
        assert_eq!(app.reel_selection.indices, [18].into_iter().collect());
        assert!(app.reel_follow);
        app.command(Command::RowDown);
        assert_eq!(app.selected, 22, "incomplete last row uses its last photo");
        app.command(Command::RowDown);
        assert_eq!(app.selected, 22);
        app.command(Command::RowUp);
        assert_eq!(app.selected, 16);
        app.settings.reel_grid = false;
        app.command(Command::RowUp);
        assert_eq!(app.selected, 14, "a single row moves by one photo");
        app.visible.clear();
        app.command(Command::RowDown);
        assert_eq!(app.selected, 14);
    }
    #[test]
    fn review_presets_reuse_roots_without_changing_dates_sources_or_decisions() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture_app(root.path());
        app.command(Command::Keep);
        let before = app.session.as_ref().unwrap().selected_ids(&app.settings);
        app.date = "2026-09-15".into();
        let preset = ImportPreset {
            pictures_path: "P:/Photographer".into(),
            raw_path: "Z:/Camera/RAW".into(),
            videos_path: "V:/Photographer".into(),
            album_name: "Holiday".into(),
            date: "2020-01-01".into(),
            device: "Different camera".into(),
            ..Default::default()
        };
        app.apply_review_destinations(&preset);
        assert_eq!(app.settings.picture_root, preset.pictures_path);
        assert_eq!(app.settings.raw_root, preset.raw_path);
        assert_eq!(app.settings.video_root, preset.videos_path);
        assert_eq!(app.date, "2026-09-15");
        assert_eq!(
            app.session.as_ref().unwrap().selected_ids(&app.settings),
            before
        );
        assert!(matches!(
            app.session.as_ref().unwrap().source,
            Source::Local { .. }
        ));
        let paths = app.destinations().unwrap();
        assert_eq!(paths.album_name.as_deref(), Some("Holiday"));
        app.session.as_mut().unwrap().source = Source::Mtp {
            device: "device-id".into(),
            folder: "folder-id".into(),
        };
        app.settings.recent_sources = vec![RecentSource::Camera {
            device_id: "device-id".into(),
            device_name: "X-T5".into(),
            folder_id: "folder-id".into(),
            folder_path: "SD Card/DCIM".into(),
        }];
        let draft = app.review_import_preset();
        assert_eq!(draft.device, "X-T5");
        assert_eq!(draft.source_path, "SD Card/DCIM");
        assert_eq!(draft.raw_path, preset.raw_path);
        assert!(
            draft.date.is_empty(),
            "reusable presets must not retain a session date"
        );
        assert!(draft.name.is_empty(), "users choose their own preset names");
        // Returning from the preset editor resumes the destination dialog.
        app.preset_from_review = true;
        app.presets_dialog(&egui::Context::default());
        assert!(app.import_open);
        assert!(!app.preset_from_review);
    }
    fn fixture_app(root: &std::path::Path) -> App {
        let photos = root.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        for name in [
            "A.JPG", "A.RAF", "B.JPG", "B.RAF", "C.JPG", "C.RAF", "V.MP4",
        ] {
            std::fs::write(photos.join(name), b"metadata-only fixture").unwrap();
        }
        let mut app = App::with_store(
            Store::at(root.join("test.sqlite3")).unwrap(),
            root.join("presets.json"),
        )
        .unwrap();
        app.install(review::scan_local(photos, None).unwrap());
        app
    }
    #[test]
    fn empty_filters_never_leave_hidden_edit_targets_or_change_their_decisions() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture_app(root.path());
        app.command(Command::SelectAll);
        app.command(Command::FilterKeep);
        assert!(app.visible.is_empty());
        assert!(app.reel_selection.indices.is_empty());
        app.command(Command::Keep);
        app.command(Command::SelectAll);
        app.command(Command::Reject);
        assert!(app.reel_selection.indices.is_empty());
        let session = app.session.as_ref().unwrap();
        assert!(
            session.shots[..3]
                .iter()
                .all(|s| s.decision(Kind::Jpeg, true) == Decision::Unreviewed)
        );
        assert_eq!(session.selected_ids(&app.settings).len(), 1);
    }
    #[test]
    fn ctrl_a_selects_without_deciding_and_batch_decisions_link_raw_and_undo_together() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture_app(root.path());
        app.settings.auto_advance = true;
        let selected = app.selected;
        app.command(Command::SelectAll);
        assert_eq!(app.reel_selection.indices.len(), 3);
        assert_eq!(
            app.session
                .as_ref()
                .unwrap()
                .selected_ids(&app.settings)
                .len(),
            1
        );
        app.command(Command::Keep);
        assert_eq!(
            app.session
                .as_ref()
                .unwrap()
                .selected_ids(&app.settings)
                .len(),
            7
        );
        assert_eq!(app.selected, selected, "batch decisions must not advance");
        app.command(Command::Undo);
        assert_eq!(
            app.session
                .as_ref()
                .unwrap()
                .selected_ids(&app.settings)
                .len(),
            1
        );
        app.command(Command::Redo);
        app.command(Command::Reject);
        assert_eq!(
            app.session
                .as_ref()
                .unwrap()
                .selected_ids(&app.settings)
                .len(),
            1
        );
        app.command(Command::Clear);
        assert!(app.visible.iter().all(|i| {
            app.session.as_ref().unwrap().shots[*i].decision(Kind::Jpeg, true)
                == Decision::Unreviewed
        }));
        app.command(Command::Next);
        assert_eq!(app.reel_selection.indices.len(), 1);
        app.command(Command::Keep);
        assert_eq!(app.selected, 2, "single decisions may advance");
        app.media_filter = Some(Kind::Video);
        app.refresh_visible();
        assert!(
            !app.reel_selection.indices.iter().any(|i| *i < 3),
            "hidden photos cannot remain batch targets"
        );
    }
    #[test]
    fn active_a_decisions_do_not_modify_a_batch_and_ctrl_click_removes_members() {
        let root = tempfile::tempdir().unwrap();
        let mut app = fixture_app(root.path());
        app.reel_selection.click(1, &app.visible, true, false);
        app.selected = 1;
        app.command(Command::Keep);
        assert_eq!(
            app.session
                .as_ref()
                .unwrap()
                .selected_ids(&app.settings)
                .len(),
            5
        );
        app.reel_selection.click(0, &app.visible, true, false);
        assert_eq!(
            app.reel_selection
                .indices
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [1]
        );
        app.pinned = Some(2);
        app.canvas.active_a = true;
        app.command(Command::Reject);
        let session = app.session.as_ref().unwrap();
        assert_eq!(session.shots[1].decision(Kind::Jpeg, true), Decision::Keep);
        assert_eq!(
            session.shots[2].decision(Kind::Jpeg, true),
            Decision::Reject
        );
        app.command(Command::FilterReject);
        assert_eq!(app.visible, [2]);
        assert_eq!(
            app.reel_selection
                .indices
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [2]
        );
    }
}
