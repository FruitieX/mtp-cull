use crate::culling::{
    Action, BurstMembership, BurstSettings, Database, Decision, Session, apply_burst_groups,
    apply_rejects, decode_jpeg_preview, load_session, sort_by_capture_time,
};
use crate::mtp_worker::{
    ImportPaths, MtpDevice, MtpEvent, MtpRequest, MtpWorker, RemoteSession, SourceFolder,
};
use chrono::{Local, NaiveDate};
use color_eyre::eyre::Result;
use eframe::egui;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

const FIT_PREVIEW_EDGE: u32 = 1_600;
const THUMBNAIL_EDGE: u32 = 96;
const PREVIEW_CACHE_CAPACITY: usize = 18;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PreviewKey {
    path: PathBuf,
    orientation: Option<u16>,
    max_edge: Option<u32>,
}

struct PreviewResult {
    generation: u64,
    key: PreviewKey,
    image: Result<egui::ColorImage, String>,
}

struct CachedPreview {
    texture: egui::TextureHandle,
    last_used: u64,
}

struct DirectShot {
    id: String,
    stem: String,
    preview_path: Option<PathBuf>,
    preview_known: bool,
    preview_error: Option<String>,
    asset_summary: String,
    decision: Decision,
}

struct DirectSession {
    shots: Vec<DirectShot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecisionFilter {
    All,
    Unrated,
    Keep,
    Reject,
}

impl DecisionFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Unrated => "Unrated",
            Self::Keep => "Keep",
            Self::Reject => "Reject",
        }
    }

    fn matches(self, decision: Decision) -> bool {
        match self {
            Self::All => true,
            Self::Unrated => decision == Decision::Unrated,
            Self::Keep => decision == Decision::Keep,
            Self::Reject => decision == Decision::Reject,
        }
    }
}

#[derive(Clone)]
struct ShotDisplay {
    identity: String,
    stem: String,
    decision: Decision,
    preview_path: Option<PathBuf>,
    preview_known: bool,
    preview_error: Option<String>,
    orientation: Option<u16>,
    asset_summary: String,
    has_conflict: bool,
    burst: Option<BurstMembership>,
    sharpness: Option<f64>,
    capture_time: Option<String>,
}

struct ImportForm {
    pictures: String,
    raw: String,
    videos: String,
    date: String,
    album_name: String,
}

impl Default for ImportForm {
    fn default() -> Self {
        Self {
            pictures: String::new(),
            raw: String::new(),
            videos: String::new(),
            date: Local::now().date_naive().to_string(),
            album_name: String::new(),
        }
    }
}

pub fn init() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        env!("CARGO_PKG_NAME"),
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(MyApp::new()?))
        }),
    )?;
    Ok(())
}

struct MyApp {
    database: Database,
    keybindings: BTreeMap<Action, String>,
    burst_settings: BurstSettings,
    session: Option<Session>,
    direct_session: Option<DirectSession>,
    mtp_worker: Option<MtpWorker>,
    mtp_devices: Vec<MtpDevice>,
    mtp_folders: Vec<SourceFolder>,
    selected_mtp_device: Option<String>,
    selected_mtp_folder: Option<String>,
    loading: Option<Receiver<Result<Session>>>,
    preview_sender: Sender<PreviewResult>,
    preview_receiver: Receiver<PreviewResult>,
    preview_generation: u64,
    preview_cache: HashMap<PreviewKey, CachedPreview>,
    preview_pending: HashSet<PreviewKey>,
    mtp_preview_pending: HashSet<String>,
    preview_tick: u64,
    selected: usize,
    zoom_levels: HashMap<String, f32>,
    compare_next: bool,
    filter: DecisionFilter,
    decision_counts: [usize; 3],
    filtered_indices: Vec<usize>,
    sort_by_sharpness: bool,
    show_shortcuts: bool,
    show_burst_settings: bool,
    show_mtp_picker: bool,
    show_import_review: bool,
    import_form: ImportForm,
    show_reject_review: bool,
    show_keep_all_confirmation: bool,
    error: Option<String>,
    notice: Option<String>,
}

impl MyApp {
    fn new() -> Result<Self> {
        let database = Database::open()?;
        let keybindings = database.keybindings()?;
        let burst_settings = database.burst_settings()?;
        let (preview_sender, preview_receiver) = mpsc::channel();
        Ok(Self {
            database,
            keybindings,
            burst_settings,
            session: None,
            direct_session: None,
            mtp_worker: None,
            mtp_devices: Vec::new(),
            mtp_folders: Vec::new(),
            selected_mtp_device: None,
            selected_mtp_folder: None,
            loading: None,
            preview_sender,
            preview_receiver,
            preview_generation: 0,
            preview_cache: HashMap::new(),
            preview_pending: HashSet::new(),
            mtp_preview_pending: HashSet::new(),
            preview_tick: 0,
            selected: 0,
            zoom_levels: HashMap::new(),
            compare_next: false,
            filter: DecisionFilter::All,
            decision_counts: [0; 3],
            filtered_indices: Vec::new(),
            sort_by_sharpness: false,
            show_shortcuts: false,
            show_burst_settings: false,
            show_mtp_picker: false,
            show_import_review: false,
            import_form: ImportForm::default(),
            show_reject_review: false,
            show_keep_all_confirmation: false,
            error: None,
            notice: None,
        })
    }

    fn begin_loading(&mut self, primary_directory: PathBuf, raw_directory: Option<PathBuf>) {
        let database = self.database.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(load_session(&database, primary_directory, raw_directory));
        });
        self.loading = Some(receiver);
        self.session = None;
        self.direct_session = None;
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.selected = 0;
        self.zoom_levels.clear();
        self.compare_next = false;
        self.filter = DecisionFilter::All;
        self.refresh_navigation_cache();
        self.preview_cache.clear();
        self.preview_pending.clear();
        self.mtp_preview_pending.clear();
        self.error = None;
        self.notice = None;
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.loading else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(session)) => {
                let mut session = session;
                apply_burst_groups(&mut session.shots, self.burst_settings);
                self.notice = Some(format!("Loaded {} shots", session.shots.len()));
                self.session = Some(session);
                self.refresh_navigation_cache();
                self.loading = None;
            }
            Ok(Err(error)) => {
                self.error = Some(format!("Could not load session: {error:#}"));
                self.loading = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50))
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some("The session loader stopped unexpectedly".to_owned());
                self.loading = None;
            }
        }
    }

    fn open_mtp_picker(&mut self) {
        if self.mtp_worker.is_none() {
            match MtpWorker::spawn() {
                Ok(worker) => self.mtp_worker = Some(worker),
                Err(error) => {
                    self.error = Some(format!("Could not start direct MTP mode: {error:#}"));
                    return;
                }
            }
        }
        if let Some(worker) = &self.mtp_worker
            && let Err(error) = worker.send(MtpRequest::ListDevices)
        {
            self.error = Some(format!("Could not list MTP devices: {error:#}"));
        }
        self.show_mtp_picker = true;
    }

    fn poll_mtp_worker(&mut self) {
        let mut events = Vec::new();
        if let Some(worker) = &self.mtp_worker {
            while let Some(event) = worker.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event {
                MtpEvent::Devices(devices) => self.mtp_devices = devices,
                MtpEvent::SourceFolders { device_id, folders } => {
                    if self.selected_mtp_device.as_deref() == Some(&device_id) {
                        self.selected_mtp_folder = folders.first().map(|folder| folder.id.clone());
                        self.mtp_folders = folders;
                    }
                }
                MtpEvent::SessionScanned(session) => self.start_direct_session(session),
                MtpEvent::PreviewCached {
                    shot_id,
                    preview_path,
                } => {
                    if let Some(session) = &mut self.direct_session
                        && let Some(shot) = session.shots.iter_mut().find(|shot| shot.id == shot_id)
                    {
                        shot.preview_path = preview_path;
                        shot.preview_known = true;
                        shot.preview_error = None;
                    }
                    self.mtp_preview_pending.remove(&shot_id);
                    self.error = None;
                }
                MtpEvent::PreviewFailed { shot_id, message } => {
                    if let Some(session) = &mut self.direct_session
                        && let Some(shot) = session.shots.iter_mut().find(|shot| shot.id == shot_id)
                    {
                        shot.preview_known = true;
                        shot.preview_error = Some(message.clone());
                    }
                    self.mtp_preview_pending.remove(&shot_id);
                    self.error = Some(format!("Could not fetch MTP preview: {message}"));
                }
                MtpEvent::ImportFinished {
                    copied,
                    skipped_existing,
                } => {
                    self.notice = Some(format!(
                        "Imported {copied} files; {skipped_existing} identical existing files skipped"
                    ));
                    self.direct_session = None;
                    self.preview_generation = self.preview_generation.wrapping_add(1);
                    self.refresh_navigation_cache();
                    self.preview_cache.clear();
                    self.preview_pending.clear();
                    self.mtp_preview_pending.clear();
                }
                MtpEvent::Error { operation, message } => {
                    self.error = Some(format!("MTP {operation} failed: {message}"));
                }
            }
        }
    }

    fn start_direct_session(&mut self, session: RemoteSession) {
        let shots = session
            .shots
            .into_iter()
            .map(|shot| DirectShot {
                id: shot.id,
                stem: shot.stem,
                preview_path: shot
                    .assets
                    .iter()
                    .find_map(|asset| asset.preview_path.clone()),
                preview_known: false,
                preview_error: None,
                asset_summary: shot
                    .assets
                    .iter()
                    .map(|asset| format!("{:?}", asset.kind))
                    .collect::<Vec<_>>()
                    .join(" + "),
                decision: Decision::Unrated,
            })
            .collect::<Vec<_>>();
        self.notice = Some(format!(
            "MTP session ready: {} shots; previews load as you browse",
            shots.len()
        ));
        self.session = None;
        self.direct_session = Some(DirectSession { shots });
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.selected = 0;
        self.zoom_levels.clear();
        self.compare_next = false;
        self.filter = DecisionFilter::All;
        self.refresh_navigation_cache();
        self.preview_cache.clear();
        self.preview_pending.clear();
        self.mtp_preview_pending.clear();
        self.show_mtp_picker = false;
    }

    fn poll_previews(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.preview_receiver.try_recv() {
            if result.generation != self.preview_generation {
                continue;
            }
            self.preview_pending.remove(&result.key);
            match result.image {
                Ok(image) => {
                    let texture = ctx.load_texture(
                        format!(
                            "preview:{}:{:?}",
                            result.key.path.display(),
                            result.key.max_edge
                        ),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.preview_tick += 1;
                    self.preview_cache.insert(
                        result.key,
                        CachedPreview {
                            texture,
                            last_used: self.preview_tick,
                        },
                    );
                }
                Err(error) => self.error = Some(format!("Could not decode JPEG preview: {error}")),
            }
        }
        while self.preview_cache.len() > PREVIEW_CACHE_CAPACITY {
            if let Some(key) = self
                .preview_cache
                .iter()
                .min_by_key(|(_, preview)| preview.last_used)
                .map(|(key, _)| key.clone())
            {
                self.preview_cache.remove(&key);
            }
        }
    }

    fn request_preview(&mut self, ctx: &egui::Context, key: PreviewKey) {
        if self.preview_cache.contains_key(&key) || !self.preview_pending.insert(key.clone()) {
            return;
        }
        let sender = self.preview_sender.clone();
        let context = ctx.clone();
        let generation = self.preview_generation;
        std::thread::spawn(move || {
            let image = decode_jpeg_preview(&key.path, key.orientation, key.max_edge)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(PreviewResult {
                generation,
                key,
                image,
            });
            context.request_repaint();
        });
    }

    fn cached_preview(&mut self, key: &PreviewKey) -> Option<egui::TextureHandle> {
        let preview = self.preview_cache.get_mut(key)?;
        self.preview_tick += 1;
        preview.last_used = self.preview_tick;
        Some(preview.texture.clone())
    }

    fn preload_nearby_previews(&mut self, ctx: &egui::Context) {
        self.request_mtp_previews();
        let mut keys = Vec::new();
        for offset in [-2_isize, -1, 0, 1, 2] {
            let index = self.selected.saturating_add_signed(offset);
            if index < self.shot_count()
                && let Some(display) = self.display_shot(index)
                && let Some(key) = self.preview_key(
                    &display,
                    if self.zoom_for(index).is_some() {
                        None
                    } else {
                        Some(FIT_PREVIEW_EDGE)
                    },
                )
            {
                keys.push(key);
            }
        }
        for key in keys {
            self.request_preview(ctx, key);
        }
    }

    fn request_mtp_previews(&mut self) {
        let Some(session) = &self.direct_session else {
            return;
        };
        let candidates = [0_isize, 1, -1, 2, -2]
            .into_iter()
            .filter_map(|offset| {
                let index = self.selected.saturating_add_signed(offset);
                session
                    .shots
                    .get(index)
                    .map(|shot| (shot.id.clone(), shot.preview_known))
            })
            .collect::<Vec<_>>();
        let mut shot_ids = Vec::new();
        for (shot_id, preview_known) in candidates {
            if !preview_known && self.mtp_preview_pending.insert(shot_id.clone()) {
                shot_ids.push(shot_id);
            }
        }
        if shot_ids.is_empty() {
            return;
        }
        if let Some(worker) = &self.mtp_worker {
            for shot_id in shot_ids {
                if let Err(error) = worker.send(MtpRequest::CachePreview {
                    shot_id: shot_id.clone(),
                }) {
                    self.mtp_preview_pending.remove(&shot_id);
                    self.error = Some(format!("Could not request MTP preview: {error:#}"));
                    break;
                }
            }
        }
    }

    fn retry_mtp_preview(&mut self, index: usize) {
        let Some(session) = &mut self.direct_session else {
            return;
        };
        let Some(shot) = session.shots.get_mut(index) else {
            return;
        };
        let shot_id = shot.id.clone();
        shot.preview_known = false;
        shot.preview_error = None;
        self.mtp_preview_pending.remove(&shot_id);
        self.error = None;
    }

    fn shot_count(&self) -> usize {
        self.session.as_ref().map_or_else(
            || {
                self.direct_session
                    .as_ref()
                    .map_or(0, |session| session.shots.len())
            },
            |session| session.shots.len(),
        )
    }

    fn has_session(&self) -> bool {
        self.shot_count() > 0
    }

    fn decision_at(&self, index: usize) -> Option<Decision> {
        if let Some(session) = &self.direct_session {
            return session.shots.get(index).map(|shot| shot.decision);
        }
        self.session
            .as_ref()
            .and_then(|session| session.shots.get(index))
            .map(|shot| shot.decision)
    }

    fn refresh_navigation_cache(&mut self) {
        let mut counts = [0; 3];
        let mut filtered = Vec::new();
        for index in 0..self.shot_count() {
            if let Some(decision) = self.decision_at(index) {
                match decision {
                    Decision::Unrated => counts[0] += 1,
                    Decision::Keep => counts[1] += 1,
                    Decision::Reject => counts[2] += 1,
                }
                if self.filter.matches(decision) {
                    filtered.push(index);
                }
            }
        }
        self.decision_counts = counts;
        self.filtered_indices = filtered;
    }

    fn decision_counts(&self) -> [usize; 3] {
        self.decision_counts
    }

    fn visible_indices(&self) -> &[usize] {
        &self.filtered_indices
    }

    fn ensure_selected_visible(&mut self) {
        if !self.visible_indices().contains(&self.selected)
            && let Some(index) = self.visible_indices().first().copied()
        {
            self.selected = index;
            self.compare_next = false;
        }
    }

    fn set_filter(&mut self, filter: DecisionFilter) {
        self.filter = filter;
        self.refresh_navigation_cache();
        self.ensure_selected_visible();
    }

    fn zoom_key(&self, index: usize) -> Option<String> {
        self.display_shot(index).map(|display| display.identity)
    }

    fn zoom_for(&self, index: usize) -> Option<f32> {
        self.zoom_key(index)
            .and_then(|key| self.zoom_levels.get(&key).copied())
    }

    fn set_zoom_for(&mut self, index: usize, zoom: Option<f32>) {
        let Some(key) = self.zoom_key(index) else {
            return;
        };
        if let Some(zoom) = zoom {
            self.zoom_levels.insert(key, zoom);
        } else {
            self.zoom_levels.remove(&key);
        }
    }

    fn display_shot(&self, index: usize) -> Option<ShotDisplay> {
        if let Some(session) = &self.direct_session {
            let shot = session.shots.get(index)?;
            return Some(ShotDisplay {
                identity: format!("mtp:{}", shot.id),
                stem: shot.stem.clone(),
                decision: shot.decision,
                preview_path: shot.preview_path.clone(),
                preview_known: shot.preview_known,
                preview_error: shot.preview_error.clone(),
                orientation: None,
                asset_summary: shot.asset_summary.clone(),
                has_conflict: false,
                burst: None,
                sharpness: None,
                capture_time: None,
            });
        }
        let shot = self.session.as_ref()?.shots.get(index)?;
        let jpeg = shot.jpeg();
        let preview_path = jpeg.map(|asset| asset.path.clone());
        let identity = preview_path
            .clone()
            .or_else(|| shot.assets.first().map(|asset| asset.path.clone()))
            .map(|path| format!("local:{}", path.display()))
            .unwrap_or_else(|| format!("local:{}", shot.stem));
        Some(ShotDisplay {
            identity,
            stem: shot.stem.clone(),
            decision: shot.decision,
            preview_path,
            preview_known: true,
            preview_error: None,
            orientation: jpeg.and_then(|asset| asset.orientation),
            asset_summary: shot
                .assets
                .iter()
                .map(|asset| asset.kind.label())
                .collect::<Vec<_>>()
                .join(" + "),
            has_conflict: shot.has_conflict(),
            burst: shot.burst,
            sharpness: shot.sharpness(),
            capture_time: shot
                .capture_time()
                .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string()),
        })
    }

    fn preview_key(&self, display: &ShotDisplay, max_edge: Option<u32>) -> Option<PreviewKey> {
        Some(PreviewKey {
            path: display.preview_path.clone()?,
            orientation: display.orientation,
            max_edge,
        })
    }

    fn move_to_next_unrated(&mut self) {
        let count = self.shot_count();
        if count == 0 {
            return;
        }
        for offset in 1..=count {
            let index = (self.selected + offset) % count;
            if self.decision_at(index) == Some(Decision::Unrated) {
                self.selected = index;
                self.compare_next = false;
                return;
            }
        }
        self.notice = Some("All shots have a decision".to_owned());
    }

    fn move_selection(&mut self, amount: isize) {
        let shot_count = self.shot_count();
        if shot_count == 0 {
            return;
        }
        if self.filter != DecisionFilter::All {
            if let Some(position) = self
                .filtered_indices
                .iter()
                .position(|index| *index == self.selected)
            {
                let target = position
                    .saturating_add_signed(amount)
                    .min(self.filtered_indices.len().saturating_sub(1));
                self.selected = self.filtered_indices[target];
            } else {
                self.ensure_selected_visible();
            }
            self.compare_next = self.selected + 1 >= shot_count;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(shot_count.saturating_sub(1));
        if self.selected + 1 >= shot_count {
            self.compare_next = false;
        }
    }

    fn set_decision(&mut self, decision: Decision) {
        if let Some(session) = &mut self.direct_session {
            let Some(shot) = session.shots.get_mut(self.selected) else {
                return;
            };
            shot.decision = decision;
            if self.selected + 1 < session.shots.len() {
                self.selected += 1;
            }
            self.refresh_navigation_cache();
            self.ensure_selected_visible();
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        let Some(shot) = session.shots.get_mut(self.selected) else {
            return;
        };
        if let Err(error) = self.database.set_decision(shot, decision) {
            self.error = Some(format!("Could not save decision: {error:#}"));
            return;
        }
        shot.decision = decision;
        if self.selected + 1 < session.shots.len() {
            self.selected += 1;
        }
        self.refresh_navigation_cache();
        self.ensure_selected_visible();
    }

    fn shortcut_pressed(&self, ctx: &egui::Context, action: Action) -> bool {
        let Some(name) = self.keybindings.get(&action) else {
            return false;
        };
        let Some(key) = egui::Key::from_name(name) else {
            return false;
        };
        ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, key))
        })
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.shortcut_pressed(ctx, Action::Previous) {
            self.move_selection(-1);
        }
        if self.shortcut_pressed(ctx, Action::Next) {
            self.move_selection(1);
        }
        if self.shortcut_pressed(ctx, Action::NextUnrated) {
            self.move_to_next_unrated();
        }
        if self.shortcut_pressed(ctx, Action::Reject) {
            self.set_decision(Decision::Reject);
        }
        if self.shortcut_pressed(ctx, Action::Keep) {
            self.set_decision(Decision::Keep);
        }
        if self.shortcut_pressed(ctx, Action::Unrated) {
            self.set_decision(Decision::Unrated);
        }
        if self.shortcut_pressed(ctx, Action::ToggleZoom) {
            let zoom = if self.zoom_for(self.selected).is_some() {
                None
            } else {
                Some(1.0)
            };
            self.set_zoom_for(self.selected, zoom);
        }
        if self.shortcut_pressed(ctx, Action::ToggleCompare)
            && self.selected + 1 < self.shot_count()
        {
            self.compare_next = !self.compare_next;
        }
    }

    fn set_all_keep(&mut self) {
        if let Some(session) = &mut self.direct_session {
            for shot in &mut session.shots {
                shot.decision = Decision::Keep;
            }
            self.refresh_navigation_cache();
            self.ensure_selected_visible();
            self.notice = Some("Marked every MTP shot as Keep".to_owned());
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        for shot in &mut session.shots {
            if let Err(error) = self.database.set_decision(shot, Decision::Keep) {
                self.error = Some(format!("Could not save decisions: {error:#}"));
                return;
            }
            shot.decision = Decision::Keep;
        }
        self.refresh_navigation_cache();
        self.ensure_selected_visible();
        self.notice = Some("Marked every shot as Keep".to_owned());
    }

    fn toggle_sharpness_sort(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        self.sort_by_sharpness = !self.sort_by_sharpness;
        if self.sort_by_sharpness {
            session.shots.sort_by(|left, right| {
                right
                    .sharpness()
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&left.sharpness().unwrap_or(f64::NEG_INFINITY))
                    .then(left.stem.cmp(&right.stem))
            });
            self.notice = Some("Sorted by JPEG sharpness score".to_owned());
        } else {
            sort_by_capture_time(&mut session.shots);
            self.notice = Some("Sorted by capture time".to_owned());
        }
        self.selected = 0;
        self.refresh_navigation_cache();
        self.ensure_selected_visible();
        self.compare_next = false;
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let mut decision = None;
        let has_session = self.has_session();
        ui.horizontal(|ui| {
            ui.heading("MTP CULL");
            if ui.button("Open album").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.begin_loading(path, None);
            }
            if ui.button("Cull MTP").clicked() {
                self.open_mtp_picker();
            }
            if ui
                .add_enabled(self.session.is_some(), egui::Button::new("RAW folder"))
                .clicked()
                && let Some(raw_directory) = rfd::FileDialog::new().pick_folder()
                && let Some(session) = &self.session
            {
                self.begin_loading(session.primary_directory.clone(), Some(raw_directory));
            }
            ui.separator();
            if let Some(session) = &self.session {
                ui.label(format!("{}", session.primary_directory.display()));
                if session.raw_directory.is_some() {
                    ui.weak("+ RAW");
                }
            } else if self.direct_session.is_some() {
                ui.label("MTP device session");
            } else {
                ui.weak("No session");
            }
            let counts = self.decision_counts();
            if has_session {
                ui.separator();
                ui.colored_label(
                    egui::Color32::from_rgb(220, 180, 80),
                    format!("{} unrated", counts[0]),
                );
                ui.colored_label(
                    egui::Color32::from_rgb(100, 205, 130),
                    format!("{} keep", counts[1]),
                );
                ui.colored_label(
                    egui::Color32::from_rgb(220, 105, 105),
                    format!("{} reject", counts[2]),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Shortcuts").clicked() {
                    self.show_shortcuts = true;
                }
                if ui.button("Burst").clicked() {
                    self.show_burst_settings = true;
                }
                if ui
                    .add_enabled(
                        has_session,
                        egui::Button::new(if self.sort_by_sharpness {
                            "Capture order"
                        } else {
                            "Sort sharpness"
                        }),
                    )
                    .on_hover_text("Sort by the JPEG sharpness hint")
                    .clicked()
                {
                    self.toggle_sharpness_sort();
                }
            });
        });
        ui.horizontal(|ui| {
            let key = |action: Action| {
                self.keybindings
                    .get(&action)
                    .map_or(action.default_key(), String::as_str)
            };
            if ui
                .add_enabled(
                    has_session,
                    egui::Button::new(format!("Reject  [{}]", key(Action::Reject)))
                        .fill(egui::Color32::from_rgb(105, 45, 48)),
                )
                .clicked()
            {
                decision = Some(Decision::Reject);
            }
            if ui
                .add_enabled(
                    has_session,
                    egui::Button::new(format!("Keep  [{}]", key(Action::Keep)))
                        .fill(egui::Color32::from_rgb(42, 103, 70)),
                )
                .clicked()
            {
                decision = Some(Decision::Keep);
            }
            if ui
                .add_enabled(
                    has_session,
                    egui::Button::new(format!("Clear  [{}]", key(Action::Unrated)))
                        .fill(egui::Color32::from_rgb(85, 85, 85)),
                )
                .clicked()
            {
                decision = Some(Decision::Unrated);
            }
            ui.separator();
            if ui
                .add_enabled(
                    has_session,
                    egui::Button::new(if self.zoom_for(self.selected).is_some() {
                        "Fit"
                    } else {
                        "100%"
                    }),
                )
                .on_hover_text("The zoom level is remembered for each shot")
                .clicked()
            {
                let zoom = if self.zoom_for(self.selected).is_some() {
                    None
                } else {
                    Some(1.0)
                };
                self.set_zoom_for(self.selected, zoom);
            }
            let has_compare_target = self.selected + 1 < self.shot_count();
            if ui
                .add_enabled(
                    has_compare_target,
                    egui::Button::new(if self.compare_next {
                        "A/B on"
                    } else {
                        "Compare A/B"
                    }),
                )
                .on_hover_text("Compare the selected shot with the next one")
                .clicked()
            {
                self.compare_next = !self.compare_next;
            }
            if ui
                .add_enabled(self.session.is_some(), egui::Button::new("Review rejects"))
                .clicked()
            {
                self.show_reject_review = true;
            }
            if ui
                .add_enabled(
                    self.direct_session.is_some(),
                    egui::Button::new("Import Keep"),
                )
                .clicked()
            {
                self.show_import_review = true;
            }
            if ui
                .add_enabled(has_session, egui::Button::new("All Keep"))
                .clicked()
            {
                self.show_keep_all_confirmation = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if has_session {
                    ui.label(format!(
                        "Shot {} / {}",
                        self.selected + 1,
                        self.shot_count()
                    ));
                }
            });
        });
        if let Some(decision) = decision {
            self.set_decision(decision);
        }
    }

    fn shot_list(&mut self, ui: &mut egui::Ui) {
        if !self.has_session() {
            ui.centered_and_justified(|ui| ui.label("Open an album to start culling"));
            return;
        }
        let counts = self.decision_counts();
        ui.horizontal_wrapped(|ui| {
            for (filter, count) in [
                (DecisionFilter::All, self.shot_count()),
                (DecisionFilter::Unrated, counts[0]),
                (DecisionFilter::Keep, counts[1]),
                (DecisionFilter::Reject, counts[2]),
            ] {
                if ui
                    .selectable_label(
                        self.filter == filter,
                        format!("{}  {}", filter.label(), count),
                    )
                    .clicked()
                {
                    self.set_filter(filter);
                }
            }
        });
        ui.separator();
        let filtered = (self.filter != DecisionFilter::All).then(|| self.filtered_indices.clone());
        let visible_count = filtered.as_ref().map_or(self.shot_count(), Vec::len);
        if visible_count == 0 {
            ui.centered_and_justified(|ui| ui.label("Nothing in this view"));
            return;
        }
        egui::ScrollArea::vertical().show_rows(ui, 64.0, visible_count, |ui, range| {
            for row in range {
                let index = filtered.as_ref().map_or(row, |indices| indices[row]);
                let Some(display) = self.display_shot(index) else {
                    continue;
                };
                let selected = index == self.selected;
                let mut clicked = false;
                ui.horizontal(|ui| {
                    let (rect, thumb_response) =
                        ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::click());
                    if let Some(key) = self.preview_key(&display, Some(THUMBNAIL_EDGE)) {
                        self.request_preview(ui.ctx(), key.clone());
                        if let Some(texture) = self.cached_preview(&key) {
                            ui.put(
                                rect,
                                egui::Image::from_texture(&texture).fit_to_exact_size(rect.size()),
                            );
                        } else {
                            ui.painter()
                                .rect_filled(rect, 2.0, egui::Color32::from_gray(38));
                        }
                    } else {
                        ui.painter()
                            .rect_filled(rect, 2.0, egui::Color32::from_gray(38));
                    }
                    clicked |= thumb_response.clicked();
                    ui.vertical(|ui| {
                        ui.set_min_height(56.0);
                        let status = match display.decision {
                            Decision::Keep => ("K", egui::Color32::from_rgb(100, 205, 130)),
                            Decision::Reject => ("R", egui::Color32::from_rgb(220, 105, 105)),
                            Decision::Unrated => ("·", egui::Color32::from_rgb(220, 180, 80)),
                        };
                        ui.horizontal(|ui| {
                            ui.colored_label(status.1, status.0);
                            let response = ui.selectable_label(selected, &display.stem);
                            clicked |= response.clicked();
                        });
                        ui.weak(display.asset_summary);
                        let mut detail = String::new();
                        if display.has_conflict {
                            detail.push_str("conflict  ");
                        }
                        if let Some(burst) = display.burst {
                            detail.push_str(&format!("burst {}  ", burst.id));
                        }
                        if let Some(sharpness) = display.sharpness {
                            detail.push_str(&format!("sharpness {:.0}", sharpness));
                        }
                        if !detail.is_empty() {
                            ui.small(detail);
                        }
                    });
                });
                if clicked {
                    self.selected = index;
                    self.compare_next = false;
                }
                ui.add_space(2.0);
            }
        });
    }

    fn image_viewer(&mut self, ui: &mut egui::Ui) {
        if !self.has_session() {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Ready to cull");
                    ui.label("Open an album or connect an MTP device.");
                    ui.small("1 Reject   2 Keep   0 Clear   ←/→ Browse   Tab Next unrated");
                });
            });
            return;
        }
        if self.compare_next && self.selected + 1 < self.shot_count() {
            ui.horizontal(|ui| {
                ui.strong("A/B comparison");
                ui.weak("Decision applies to A  •  Keep or reject, then continue with →");
            });
            ui.columns(2, |columns| {
                columns[0].push_id(("compare", self.selected), |ui| {
                    self.image_pane(ui, self.selected, "A");
                });
                columns[1].push_id(("compare", self.selected + 1), |ui| {
                    self.image_pane(ui, self.selected + 1, "B");
                });
            });
        } else {
            self.image_pane(ui, self.selected, "");
        }
    }

    fn image_pane(&mut self, ui: &mut egui::Ui, index: usize, pane_label: &str) {
        let Some(display) = self.display_shot(index) else {
            return;
        };
        ui.horizontal(|ui| {
            if !pane_label.is_empty() {
                ui.strong(pane_label);
            }
            ui.strong(&display.stem);
            ui.colored_label(decision_color(display.decision), display.decision.label());
            if display.has_conflict {
                ui.colored_label(egui::Color32::YELLOW, "conflict");
            }
            if let Some(sharpness) = display.sharpness {
                ui.weak(format!("sharpness {sharpness:.0}"));
            }
            if ui
                .small_button(if self.zoom_for(index).is_some() {
                    "Fit"
                } else {
                    "100%"
                })
                .clicked()
            {
                let zoom = if self.zoom_for(index).is_some() {
                    None
                } else {
                    Some(1.0)
                };
                self.set_zoom_for(index, zoom);
            }
        });
        ui.horizontal(|ui| {
            ui.weak(&display.asset_summary);
            if let Some(capture_time) = &display.capture_time {
                ui.weak(capture_time);
            }
            if let Some(burst) = display.burst {
                ui.colored_label(egui::Color32::LIGHT_BLUE, format!("burst {}", burst.id));
            }
        });

        if let Some(error) = &display.preview_error {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, "Preview unavailable");
                ui.small(error);
                if ui.small_button("Retry").clicked() {
                    self.retry_mtp_preview(index);
                }
            });
            return;
        }
        if !display.preview_known {
            ui.centered_and_justified(|ui| {
                ui.spinner();
                ui.label("Fetching JPEG preview from the MTP device...");
            });
            return;
        }
        let zoomed = self.zoom_for(index).is_some();
        let Some(key) =
            self.preview_key(&display, if zoomed { None } else { Some(FIT_PREVIEW_EDGE) })
        else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("No displayable JPEG companion");
                    ui.label("The paired files remain available for decisions and rejection.");
                });
            });
            return;
        };
        self.request_preview(ui.ctx(), key.clone());
        let Some(texture) = self.cached_preview(&key) else {
            ui.centered_and_justified(|ui| {
                ui.spinner();
                ui.label(if zoomed {
                    "Loading full-resolution preview..."
                } else {
                    "Loading preview..."
                });
            });
            return;
        };
        let available = ui.available_size();
        let natural_size = texture.size_vec2();
        let fit_scale = (available.x / natural_size.x)
            .min(available.y / natural_size.y)
            .min(1.0);
        let scale = self.zoom_for(index).unwrap_or(fit_scale);
        let desired_size = natural_size * scale;
        ui.push_id(("preview-scroll", display.identity), |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let response = ui.add(
                        egui::Image::from_texture(&texture)
                            .fit_to_exact_size(desired_size)
                            .sense(egui::Sense::click()),
                    );
                    if response.double_clicked() {
                        self.set_zoom_for(index, if zoomed { None } else { Some(1.0) });
                    }
                    if response.hovered() {
                        let zoom_delta = ui.input(|input| input.zoom_delta());
                        if (zoom_delta - 1.0).abs() > f32::EPSILON {
                            self.set_zoom_for(index, Some((scale * zoom_delta).clamp(0.05, 8.0)));
                        }
                    }
                });
        });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let has_session = self.has_session();
            if ui
                .add_enabled(has_session, egui::Button::new("← Previous"))
                .clicked()
            {
                self.move_selection(-1);
            }
            if ui
                .add_enabled(has_session, egui::Button::new("Next →"))
                .clicked()
            {
                self.move_selection(1);
            }
            if ui
                .add_enabled(has_session, egui::Button::new("Next unrated"))
                .on_hover_text("Tab")
                .clicked()
            {
                self.move_to_next_unrated();
            }
            ui.separator();
            if has_session {
                let counts = self.decision_counts();
                ui.weak(format!(
                    "{} unrated  ·  {} keep  ·  {} reject",
                    counts[0], counts[1], counts[2]
                ));
            } else if self.loading.is_some() {
                ui.spinner();
                ui.weak("Scanning album in background...");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                } else if let Some(notice) = &self.notice {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, notice);
                }
            });
        });
    }

    fn burst_settings_editor(&mut self, ctx: &egui::Context) {
        let mut open = self.show_burst_settings;
        egui::Window::new("Burst grouping")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Only adjacent shots within this time window are compared.");
                let mut changed = false;
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.burst_settings.window_seconds)
                            .range(0.1..=60.0)
                            .speed(0.1)
                            .suffix(" seconds"),
                    )
                    .changed();
                ui.label("Maximum dHash distance: lower is more conservative.");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.burst_settings.similarity_threshold)
                            .range(0..=64),
                    )
                    .changed();
                if changed {
                    if let Err(error) = self.database.set_burst_settings(self.burst_settings) {
                        self.error = Some(format!("Could not save burst settings: {error:#}"));
                    } else if let Some(session) = &mut self.session {
                        apply_burst_groups(&mut session.shots, self.burst_settings);
                    }
                }
            });
        self.show_burst_settings = open;
    }

    fn mtp_picker(&mut self, ctx: &egui::Context) {
        let mut open = self.show_mtp_picker;
        let mut device_to_load = None;
        let mut start = None;
        egui::Window::new("Cull MTP device")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    "Choose a device and source folder. JPEG companions are cached temporarily.",
                );
                let selected_device_name = self
                    .selected_mtp_device
                    .as_deref()
                    .and_then(|id| self.mtp_devices.iter().find(|device| device.id == id))
                    .map_or("Choose device", |device| device.name.as_str());
                egui::ComboBox::from_label("Device")
                    .selected_text(selected_device_name)
                    .show_ui(ui, |ui| {
                        for device in &self.mtp_devices {
                            if ui
                                .selectable_label(
                                    self.selected_mtp_device.as_deref() == Some(&device.id),
                                    &device.name,
                                )
                                .clicked()
                            {
                                device_to_load = Some(device.id.clone());
                            }
                        }
                    });
                let selected_folder_name = self
                    .selected_mtp_folder
                    .as_deref()
                    .and_then(|id| self.mtp_folders.iter().find(|folder| folder.id == id))
                    .map_or("Choose source folder", |folder| folder.path.as_str());
                egui::ComboBox::from_label("Source folder")
                    .selected_text(selected_folder_name)
                    .show_ui(ui, |ui| {
                        for folder in &self.mtp_folders {
                            if ui
                                .selectable_label(
                                    self.selected_mtp_folder.as_deref() == Some(&folder.id),
                                    &folder.path,
                                )
                                .clicked()
                            {
                                self.selected_mtp_folder = Some(folder.id.clone());
                            }
                        }
                    });
                if ui
                    .add_enabled(
                        self.selected_mtp_device.is_some() && self.selected_mtp_folder.is_some(),
                        egui::Button::new("Start culling (previews load on demand)"),
                    )
                    .clicked()
                    && let (Some(device_id), Some(source_folder_id)) = (
                        self.selected_mtp_device.clone(),
                        self.selected_mtp_folder.clone(),
                    )
                {
                    start = Some((device_id, source_folder_id));
                }
            });
        if let Some(device_id) = device_to_load {
            self.selected_mtp_device = Some(device_id.clone());
            self.selected_mtp_folder = None;
            self.mtp_folders.clear();
            if let Some(worker) = &self.mtp_worker
                && let Err(error) = worker.send(MtpRequest::ListSourceFolders { device_id })
            {
                self.error = Some(format!("Could not list MTP folders: {error:#}"));
            }
        }
        if let Some((device_id, source_folder_id)) = start {
            if let Some(worker) = &self.mtp_worker
                && let Err(error) = worker.send(MtpRequest::StartSession {
                    device_id,
                    source_folder_id,
                })
            {
                self.error = Some(format!("Could not start MTP culling: {error:#}"));
            } else {
                self.notice = Some(
                    "Scanning MTP files; the first previews will load on demand...".to_owned(),
                );
            }
        }
        self.show_mtp_picker = open;
    }

    fn import_review(&mut self, ctx: &egui::Context) {
        let mut open = self.show_import_review;
        let mut import = None;
        egui::Window::new("Import Keep shots")
            .open(&mut open)
            .show(ctx, |ui| {
                let keep_count = self.direct_session.as_ref().map_or(0, |session| {
                    session
                        .shots
                        .iter()
                        .filter(|shot| shot.decision == Decision::Keep)
                        .count()
                });
                ui.label(format!(
                    "Only {keep_count} explicit Keep shot(s) will be imported."
                ));
                folder_field(ui, "JPEG/HEIF root", &mut self.import_form.pictures);
                folder_field(ui, "RAW root", &mut self.import_form.raw);
                folder_field(ui, "Video root", &mut self.import_form.videos);
                ui.horizontal(|ui| {
                    ui.label("Date");
                    ui.text_edit_singleline(&mut self.import_form.date);
                });
                ui.horizontal(|ui| {
                    ui.label("Album name (optional)");
                    ui.text_edit_singleline(&mut self.import_form.album_name);
                });
                if ui
                    .add_enabled(keep_count > 0, egui::Button::new("Import Keep shots"))
                    .clicked()
                {
                    import = Some(());
                }
            });
        if import.is_some() {
            match self.import_paths() {
                Ok(destinations) => {
                    let shot_ids = self
                        .direct_session
                        .as_ref()
                        .into_iter()
                        .flat_map(|session| &session.shots)
                        .filter(|shot| shot.decision == Decision::Keep)
                        .map(|shot| shot.id.clone())
                        .collect();
                    if let Some(worker) = &self.mtp_worker
                        && let Err(error) = worker.send(MtpRequest::ImportKept {
                            shot_ids,
                            destinations,
                        })
                    {
                        self.error = Some(format!("Could not import MTP files: {error:#}"));
                    } else {
                        self.notice = Some("Importing explicit Keep shots...".to_owned());
                        open = false;
                    }
                }
                Err(error) => self.error = Some(error),
            }
        }
        self.show_import_review = open;
    }

    fn import_paths(&self) -> std::result::Result<ImportPaths, String> {
        let date = NaiveDate::parse_from_str(&self.import_form.date, "%Y-%m-%d")
            .map_err(|_| "Date must use YYYY-MM-DD".to_owned())?;
        Ok(ImportPaths {
            pictures: optional_path(&self.import_form.pictures),
            raw: optional_path(&self.import_form.raw),
            videos: optional_path(&self.import_form.videos),
            date,
            album_name: (!self.import_form.album_name.trim().is_empty())
                .then(|| self.import_form.album_name.trim().to_owned()),
        })
    }

    fn shortcut_editor(&mut self, ctx: &egui::Context) {
        let mut open = self.show_shortcuts;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Enter egui key names such as 1, ArrowLeft, Z, or Space.");
                for action in Action::ALL {
                    ui.horizontal(|ui| {
                        ui.label(action.label());
                        let key = self.keybindings.entry(action).or_default();
                        let response = ui.text_edit_singleline(key);
                        if response.changed() {
                            if egui::Key::from_name(key).is_some() {
                                if let Err(error) = self.database.set_keybinding(action, key) {
                                    self.error =
                                        Some(format!("Could not save shortcut: {error:#}"));
                                }
                            } else {
                                self.error = Some(format!("{key:?} is not a valid key name"));
                            }
                        }
                    });
                }
            });
        self.show_shortcuts = open;
    }

    fn reject_review(&mut self, ctx: &egui::Context) {
        let mut open = self.show_reject_review;
        let mut close_after_action = false;
        egui::Window::new("Review rejected shots")
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                let Some(session) = &self.session else {
                    return;
                };
                let rejected = session
                    .shots
                    .iter()
                    .filter(|shot| shot.decision == Decision::Reject)
                    .collect::<Vec<_>>();
                let files = rejected.iter().map(|shot| shot.assets.len()).sum::<usize>();
                ui.label(format!(
                    "{}/{} rejected shots, {files} files",
                    rejected.len(),
                    session.shots.len()
                ));
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Files are re-fingerprinted before they are sent to the OS recycle bin.",
                );
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for shot in &rejected {
                            ui.label(format!("{} ({})", shot.stem, shot.assets.len()));
                        }
                    });
                if ui
                    .add_enabled(
                        !rejected.is_empty(),
                        egui::Button::new("Send reviewed files to recycle bin"),
                    )
                    .clicked()
                {
                    match apply_rejects(session) {
                        Ok(count) => {
                            self.notice = Some(format!("Sent {count} files to the recycle bin"));
                            close_after_action = true;
                            self.session = None;
                        }
                        Err(error) => {
                            self.error = Some(format!("Could not recycle files: {error:#}"))
                        }
                    }
                }
            });
        if close_after_action {
            open = false;
        }
        self.show_reject_review = open;
    }

    fn keep_all_confirmation(&mut self, ctx: &egui::Context) {
        let mut open = self.show_keep_all_confirmation;
        let mut close_after_action = false;
        egui::Window::new("Mark every shot as Keep")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let manual = self.session.as_ref().map_or(0, |session| {
                    session
                        .shots
                        .iter()
                        .filter(|shot| shot.decision != Decision::Unrated)
                        .count()
                });
                ui.label(format!(
                    "This will overwrite {manual} existing manual decision(s)."
                ));
                ui.horizontal(|ui| {
                    if ui.button("Mark all Keep").clicked() {
                        self.set_all_keep();
                        close_after_action = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close_after_action = true;
                    }
                });
            });
        if close_after_action {
            open = false;
        }
        self.show_keep_all_confirmation = open;
    }
}

impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loading(&ctx);
        self.poll_mtp_worker();
        if self.mtp_worker.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.poll_previews(&ctx);
        self.handle_shortcuts(&ctx);
        self.preload_nearby_previews(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| self.top_bar(ui));
        egui::Panel::right("shots")
            .default_size(328.0)
            .resizable(true)
            .show(ui, |ui| self.shot_list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.image_viewer(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.bottom_bar(ui));

        self.shortcut_editor(&ctx);
        self.burst_settings_editor(&ctx);
        self.mtp_picker(&ctx);
        self.import_review(&ctx);
        self.reject_review(&ctx);
        self.keep_all_confirmation(&ctx);
    }
}

fn decision_color(decision: Decision) -> egui::Color32 {
    match decision {
        Decision::Keep => egui::Color32::from_rgb(100, 205, 130),
        Decision::Reject => egui::Color32::from_rgb(220, 105, 105),
        Decision::Unrated => egui::Color32::from_rgb(220, 180, 80),
    }
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

fn optional_path(value: &str) -> Option<PathBuf> {
    (!value.trim().is_empty()).then(|| PathBuf::from(value.trim()))
}
