use crate::culling::{
    Action, BurstSettings, Database, Decision, Session, Shot, apply_burst_groups, apply_rejects,
    decode_jpeg_preview, load_session, sort_by_capture_time,
};
use crate::mtp_worker::{
    ImportPaths, MtpDevice, MtpEvent, MtpProgress, MtpRequest, MtpWorker, RemoteSession,
    SourceFolder,
};
use chrono::{Local, NaiveDate};
use color_eyre::eyre::Result;
use eframe::egui;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

const FIT_PREVIEW_EDGE: u32 = 1_600;
const PREVIEW_CACHE_CAPACITY: usize = 6;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PreviewKey {
    path: PathBuf,
    orientation: Option<u16>,
    max_edge: Option<u32>,
}

struct PreviewResult {
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
    asset_summary: String,
    decision: Decision,
}

struct DirectSession {
    shots: Vec<DirectShot>,
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
    preview_cache: HashMap<PreviewKey, CachedPreview>,
    preview_pending: HashSet<PreviewKey>,
    preview_tick: u64,
    selected: usize,
    zoom: Option<f32>,
    compare_next: bool,
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
    mtp_progress: Option<MtpProgress>,
    mtp_operation: Option<&'static str>,
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
            preview_cache: HashMap::new(),
            preview_pending: HashSet::new(),
            preview_tick: 0,
            selected: 0,
            zoom: None,
            compare_next: false,
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
            mtp_progress: None,
            mtp_operation: None,
        })
    }

    fn begin_loading(&mut self, primary_directory: PathBuf, raw_directory: Option<PathBuf>) {
        if let Some(operation) = self.mtp_operation {
            self.error = Some(format!(
                "Wait for MTP {operation} to finish or cancel it before opening a local album"
            ));
            return;
        }
        if self.direct_session.is_some() {
            let _ = self.send_mtp_request(MtpRequest::CloseSession, "close session");
        }
        let database = self.database.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(load_session(&database, primary_directory, raw_directory));
        });
        self.loading = Some(receiver);
        self.session = None;
        self.direct_session = None;
        self.selected = 0;
        self.zoom = None;
        self.compare_next = false;
        self.preview_cache.clear();
        self.preview_pending.clear();
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
        if self.send_mtp_request(MtpRequest::ListDevices, "list devices") {
            self.show_mtp_picker = true;
        }
    }

    fn send_mtp_request(&mut self, request: MtpRequest, operation: &'static str) -> bool {
        if let Some(active) = self.mtp_operation {
            self.error = Some(format!(
                "Wait for MTP {active} to finish or cancel it before starting {operation}"
            ));
            return false;
        }
        let result = self.mtp_worker.as_ref().map_or_else(
            || Err(color_eyre::eyre::eyre!("direct MTP worker is not running")),
            |worker| worker.send(request),
        );
        match result {
            Ok(()) => {
                self.mtp_operation = Some(operation);
                self.mtp_progress = None;
                true
            }
            Err(error) => {
                self.error = Some(format!("Could not start MTP {operation}: {error:#}"));
                false
            }
        }
    }

    fn cancel_mtp_operation(&mut self) {
        if let Some(worker) = &self.mtp_worker
            && let Err(error) = worker.send(MtpRequest::Cancel)
        {
            self.error = Some(format!("Could not cancel MTP operation: {error:#}"));
        } else {
            self.notice = Some(
                "Cancellation requested; waiting for the device operation to stop...".to_owned(),
            );
        }
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
                MtpEvent::Progress(progress) => {
                    self.mtp_operation = Some(progress.operation);
                    self.mtp_progress = Some(progress);
                }
                MtpEvent::Devices(devices) => {
                    self.mtp_devices = devices;
                    self.mtp_progress = None;
                    self.mtp_operation = None;
                }
                MtpEvent::SourceFolders { device_id, folders } => {
                    self.mtp_progress = None;
                    self.mtp_operation = None;
                    if self.selected_mtp_device.as_deref() == Some(&device_id) {
                        self.selected_mtp_folder = folders.first().map(|folder| folder.id.clone());
                        self.mtp_folders = folders;
                    }
                }
                MtpEvent::SessionScanned(session) => {
                    self.mtp_progress = None;
                    self.mtp_operation = None;
                    self.start_direct_session(session);
                }
                MtpEvent::ImportFinished {
                    copied,
                    skipped_existing,
                } => {
                    self.notice = Some(format!(
                        "Imported {copied} files; {skipped_existing} identical existing files skipped"
                    ));
                    self.direct_session = None;
                    self.preview_cache.clear();
                    self.preview_pending.clear();
                    self.mtp_progress = None;
                    self.mtp_operation = None;
                }
                MtpEvent::SessionClosed => {
                    self.direct_session = None;
                    self.preview_cache.clear();
                    self.preview_pending.clear();
                    self.mtp_progress = None;
                    self.mtp_operation = None;
                    self.notice =
                        Some("Closed the MTP session and removed its preview cache".to_owned());
                }
                MtpEvent::Cancelled { operation } => {
                    self.mtp_progress = None;
                    self.mtp_operation = None;
                    self.notice = Some(format!("Cancelled MTP {operation}"));
                }
                MtpEvent::Error { operation, message } => {
                    self.mtp_progress = None;
                    self.mtp_operation = None;
                    self.error = Some(format!("MTP {operation} failed: {message}"));
                }
                MtpEvent::FilesListed(_) | MtpEvent::CopyFinished(_) => {}
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
            "Cached JPEG previews for {} MTP shots",
            shots.len()
        ));
        self.session = None;
        self.direct_session = Some(DirectSession { shots });
        self.selected = 0;
        self.zoom = None;
        self.compare_next = false;
        self.preview_cache.clear();
        self.preview_pending.clear();
        self.show_mtp_picker = false;
    }

    fn poll_previews(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.preview_receiver.try_recv() {
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
        std::thread::spawn(move || {
            let image = decode_jpeg_preview(&key.path, key.orientation, key.max_edge)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(PreviewResult { key, image });
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
        if let Some(session) = &self.direct_session {
            let keys = [-1_isize, 0, 1]
                .into_iter()
                .filter_map(|offset| {
                    let index = self.selected.saturating_add_signed(offset);
                    let shot = session.shots.get(index)?;
                    let path = shot.preview_path.clone()?;
                    Some(PreviewKey {
                        path,
                        orientation: None,
                        max_edge: if index == self.selected && self.zoom.is_some() {
                            None
                        } else {
                            Some(FIT_PREVIEW_EDGE)
                        },
                    })
                })
                .collect::<Vec<_>>();
            for key in keys {
                self.request_preview(ctx, key);
            }
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let mut keys = Vec::new();
        for offset in [-1_isize, 0, 1] {
            let index = self.selected.saturating_add_signed(offset);
            if index >= session.shots.len() {
                continue;
            }
            if let Some(asset) = session.shots[index].jpeg() {
                keys.push(PreviewKey {
                    path: asset.path.clone(),
                    orientation: asset.orientation,
                    max_edge: if index == self.selected && self.zoom.is_some() {
                        None
                    } else {
                        Some(FIT_PREVIEW_EDGE)
                    },
                });
            }
        }
        for key in keys {
            self.request_preview(ctx, key);
        }
    }

    fn selected_shot(&self) -> Option<&Shot> {
        self.session.as_ref()?.shots.get(self.selected)
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

    fn move_selection(&mut self, amount: isize) {
        let shot_count = self.shot_count();
        if shot_count == 0 {
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(shot_count.saturating_sub(1));
        self.zoom = None;
        self.compare_next = false;
    }

    fn set_decision(&mut self, decision: Decision) {
        if let Some(session) = &mut self.direct_session {
            let Some(shot) = session.shots.get_mut(self.selected) else {
                return;
            };
            shot.decision = decision;
            if self.selected + 1 < session.shots.len() {
                self.selected += 1;
                self.zoom = None;
                self.compare_next = false;
            }
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
            self.zoom = None;
            self.compare_next = false;
        }
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
            self.zoom = if self.zoom.is_some() { None } else { Some(1.0) };
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
        self.zoom = None;
        self.compare_next = false;
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let mut decision = None;
        let mut close_mtp = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.mtp_operation.is_none(),
                    egui::Button::new("Open album"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.begin_loading(path, None);
            }
            if ui
                .add_enabled(
                    self.mtp_operation.is_none(),
                    egui::Button::new("Cull MTP device"),
                )
                .clicked()
            {
                self.open_mtp_picker();
            }
            if ui
                .add_enabled(
                    self.session.is_some(),
                    egui::Button::new("Choose RAW folder"),
                )
                .clicked()
                && let Some(raw_directory) = rfd::FileDialog::new().pick_folder()
                && let Some(session) = &self.session
            {
                self.begin_loading(session.primary_directory.clone(), Some(raw_directory));
            }
            if ui
                .add_enabled(self.has_session(), egui::Button::new("All Keep"))
                .clicked()
            {
                self.show_keep_all_confirmation = true;
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
                    egui::Button::new("Import Keep shots"),
                )
                .clicked()
            {
                self.show_import_review = true;
            }
            if ui
                .add_enabled(
                    self.direct_session.is_some() && self.mtp_operation.is_none(),
                    egui::Button::new("Close MTP session"),
                )
                .clicked()
            {
                close_mtp = true;
            }
            if ui
                .add_enabled(
                    self.has_session(),
                    egui::Button::new(if self.sort_by_sharpness {
                        "Sort by capture time"
                    } else {
                        "Sort by sharpness"
                    }),
                )
                .clicked()
            {
                self.toggle_sharpness_sort();
            }
            if ui.button("Shortcuts").clicked() {
                self.show_shortcuts = true;
            }
            if ui.button("Burst settings").clicked() {
                self.show_burst_settings = true;
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.has_session(),
                    egui::Button::new(format!(
                        "Reject [{}]",
                        self.keybindings
                            .get(&Action::Reject)
                            .map_or(Action::Reject.default_key(), String::as_str)
                    )),
                )
                .clicked()
            {
                decision = Some(Decision::Reject);
            }
            if ui
                .add_enabled(
                    self.has_session(),
                    egui::Button::new(format!(
                        "Keep [{}]",
                        self.keybindings
                            .get(&Action::Keep)
                            .map_or(Action::Keep.default_key(), String::as_str)
                    )),
                )
                .clicked()
            {
                decision = Some(Decision::Keep);
            }
            if ui
                .add_enabled(
                    self.has_session(),
                    egui::Button::new(format!(
                        "Unrated [{}]",
                        self.keybindings
                            .get(&Action::Unrated)
                            .map_or(Action::Unrated.default_key(), String::as_str)
                    )),
                )
                .clicked()
            {
                decision = Some(Decision::Unrated);
            }
            if ui
                .add_enabled(
                    self.has_session(),
                    egui::Button::new(if self.zoom.is_some() { "Fit" } else { "100%" }),
                )
                .clicked()
            {
                self.zoom = if self.zoom.is_some() { None } else { Some(1.0) };
            }
            let has_compare_target = self.selected + 1 < self.shot_count();
            if ui
                .add_enabled(
                    has_compare_target,
                    egui::Button::new(if self.compare_next {
                        "A/B: next"
                    } else {
                        "A/B"
                    }),
                )
                .clicked()
            {
                self.compare_next = !self.compare_next;
            }
            if let Some(session) = &self.session {
                ui.separator();
                ui.label(format!("{} shots", session.shots.len()));
                ui.label(format!("Album: {}", session.primary_directory.display()));
                if let Some(raw_directory) = &session.raw_directory {
                    ui.label(format!("RAW: {}", raw_directory.display()));
                }
            }
            if let Some(session) = &self.direct_session {
                ui.separator();
                ui.label(format!("{} MTP shots", session.shots.len()));
            }
        });
        if let Some(decision) = decision {
            self.set_decision(decision);
        }
        if close_mtp {
            self.send_mtp_request(MtpRequest::CloseSession, "close session");
            self.direct_session = None;
            self.preview_cache.clear();
            self.preview_pending.clear();
        }
    }

    fn shot_list(&mut self, ui: &mut egui::Ui) {
        if let Some(session) = &self.direct_session {
            egui::ScrollArea::vertical().show_rows(ui, 24.0, session.shots.len(), |ui, range| {
                for index in range {
                    let shot = &session.shots[index];
                    let decision = match shot.decision {
                        Decision::Keep => "K",
                        Decision::Reject => "R",
                        Decision::Unrated => "-",
                    };
                    if ui
                        .selectable_label(
                            index == self.selected,
                            format!("[{decision}] {}", shot.stem),
                        )
                        .clicked()
                    {
                        self.selected = index;
                        self.zoom = None;
                        self.compare_next = false;
                    }
                }
            });
            return;
        }
        let Some(session) = &self.session else {
            ui.centered_and_justified(|ui| ui.label("Open an album to start culling"));
            return;
        };
        egui::ScrollArea::vertical().show_rows(ui, 24.0, session.shots.len(), |ui, range| {
            for index in range {
                let shot = &session.shots[index];
                let decision = match shot.decision {
                    Decision::Keep => "K",
                    Decision::Reject => "R",
                    Decision::Unrated => "-",
                };
                let mut label = format!("[{decision}] {}", shot.stem);
                if shot.has_conflict() {
                    label.push_str(" !");
                }
                if let Some(burst) = shot.burst {
                    label.push_str(&format!(" B{}:{}", burst.id, burst.distance_to_previous));
                }
                if ui.selectable_label(index == self.selected, label).clicked() {
                    self.selected = index;
                    self.zoom = None;
                    self.compare_next = false;
                }
            }
        });
    }

    fn image_viewer(&mut self, ui: &mut egui::Ui) {
        if self.direct_session.is_some() {
            self.direct_image_viewer(ui);
            return;
        }
        let displayed = if self.compare_next {
            self.session
                .as_ref()
                .and_then(|session| session.shots.get(self.selected + 1))
        } else {
            self.selected_shot()
        };
        let Some(shot) = displayed else {
            ui.centered_and_justified(|ui| ui.label("Open an album to start culling"));
            return;
        };
        ui.horizontal(|ui| {
            ui.strong(&shot.stem);
            ui.label(shot.decision.label());
            if self.compare_next {
                ui.colored_label(egui::Color32::LIGHT_BLUE, "A/B: next shot");
            }
            if shot.has_conflict() {
                ui.colored_label(egui::Color32::YELLOW, "Duplicate companion conflict");
            }
            if let Some(burst) = shot.burst {
                ui.colored_label(
                    egui::Color32::LIGHT_BLUE,
                    format!(
                        "Burst {} (distance {})",
                        burst.id, burst.distance_to_previous
                    ),
                );
            }
            if let Some(sharpness) = shot.sharpness() {
                ui.label(format!("Sharpness: {sharpness:.0}"));
            }
            if let Some(capture_time) = shot.capture_time() {
                ui.label(format!(
                    "Captured: {}",
                    capture_time.format("%Y-%m-%d %H:%M:%S")
                ));
            }
            if let Some(orientation) = shot.jpeg().and_then(|asset| asset.orientation) {
                ui.label(format!("Orientation: {orientation}"));
            }
        });
        ui.label(
            shot.assets
                .iter()
                .map(|asset| asset.kind.label())
                .collect::<Vec<_>>()
                .join(" + "),
        );

        let Some(asset) = shot.jpeg() else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("No displayable JPEG companion");
                    ui.label("The files remain paired for decisions and rejection.");
                    ui.label(
                        "Embedded RAW preview and HEIF decoding are the next viewer backend step.",
                    );
                });
            });
            return;
        };
        let key = PreviewKey {
            path: asset.path.clone(),
            orientation: asset.orientation,
            max_edge: if self.zoom.is_some() {
                None
            } else {
                Some(FIT_PREVIEW_EDGE)
            },
        };
        self.request_preview(ui.ctx(), key.clone());
        let available = ui.available_size();
        if let Some(texture) = self.cached_preview(&key) {
            let natural_size = texture.size_vec2();
            let fit_scale = (available.x / natural_size.x)
                .min(available.y / natural_size.y)
                .min(1.0);
            let scale = self.zoom.unwrap_or(fit_scale);
            let desired_size = natural_size * scale;
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let response = ui.add(
                        egui::Image::from_texture(&texture)
                            .fit_to_exact_size(desired_size)
                            .sense(egui::Sense::hover()),
                    );
                    if response.hovered() {
                        let zoom_delta = ui.input(|input| input.zoom_delta());
                        if (zoom_delta - 1.0).abs() > f32::EPSILON {
                            self.zoom = Some((scale * zoom_delta).clamp(0.05, 8.0));
                        }
                    }
                });
        } else {
            ui.centered_and_justified(|ui| {
                ui.spinner();
                ui.label(if self.zoom.is_some() {
                    "Loading full-resolution pixel preview..."
                } else {
                    "Loading oriented preview..."
                });
            });
        }
    }

    fn direct_image_viewer(&mut self, ui: &mut egui::Ui) {
        let index = self.selected + usize::from(self.compare_next);
        let Some(session) = &self.direct_session else {
            return;
        };
        let Some(shot) = session.shots.get(index) else {
            ui.centered_and_justified(|ui| ui.label("No MTP shot selected"));
            return;
        };
        let stem = shot.stem.clone();
        let decision = shot.decision;
        let summary = shot.asset_summary.clone();
        let preview_path = shot.preview_path.clone();
        ui.horizontal(|ui| {
            ui.strong(stem);
            ui.label(decision.label());
            if self.compare_next {
                ui.colored_label(egui::Color32::LIGHT_BLUE, "A/B: next shot");
            }
        });
        ui.label(summary);
        let Some(path) = preview_path else {
            ui.centered_and_justified(|ui| {
                ui.label("No JPEG companion is available for this MTP shot");
            });
            return;
        };
        let key = PreviewKey {
            path,
            orientation: None,
            max_edge: if self.zoom.is_some() {
                None
            } else {
                Some(FIT_PREVIEW_EDGE)
            },
        };
        self.request_preview(ui.ctx(), key.clone());
        if let Some(texture) = self.cached_preview(&key) {
            let natural_size = texture.size_vec2();
            let available = ui.available_size();
            let fit_scale = (available.x / natural_size.x)
                .min(available.y / natural_size.y)
                .min(1.0);
            let scale = self.zoom.unwrap_or(fit_scale);
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let response = ui.add(
                        egui::Image::from_texture(&texture)
                            .fit_to_exact_size(natural_size * scale)
                            .sense(egui::Sense::hover()),
                    );
                    if response.hovered() {
                        let zoom_delta = ui.input(|input| input.zoom_delta());
                        if (zoom_delta - 1.0).abs() > f32::EPSILON {
                            self.zoom = Some((scale * zoom_delta).clamp(0.05, 8.0));
                        }
                    }
                });
        } else {
            ui.centered_and_justified(|ui| {
                ui.spinner();
                ui.label("Loading cached MTP JPEG preview...");
            });
        }
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
                        egui::Button::new("Cache JPEG previews and start culling"),
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
            self.send_mtp_request(
                MtpRequest::ListSourceFolders { device_id },
                "list source folders",
            );
        }
        if let Some((device_id, source_folder_id)) = start
            && self.send_mtp_request(
                MtpRequest::StartSession {
                    device_id,
                    source_folder_id,
                },
                "scan session",
            )
        {
            self.notice = Some("Caching JPEG previews from the MTP device...".to_owned());
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
                    if self.send_mtp_request(
                        MtpRequest::ImportKept {
                            shot_ids,
                            destinations,
                        },
                        "import kept shots",
                    ) {
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
            .default_size(280.0)
            .show(ui, |ui| self.shot_list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.image_viewer(ui));
        let mut cancel_mtp = false;
        egui::Panel::bottom("status").show(ui, |ui| {
            if self.loading.is_some() {
                ui.spinner();
                ui.label("Scanning, pairing, and fingerprinting files in the background...");
            }
            if let Some(progress) = &self.mtp_progress {
                ui.horizontal(|ui| {
                    if let Some(total) = progress.total.filter(|total| *total > 0) {
                        let fraction = (progress.completed as f32 / total as f32).clamp(0.0, 1.0);
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_width(220.0)
                                .text(format!("{:.0}%", fraction * 100.0)),
                        );
                    } else {
                        ui.spinner();
                    }
                    ui.label(&progress.detail);
                    if ui.button("Cancel").clicked() {
                        cancel_mtp = true;
                    }
                });
            } else if let Some(operation) = self.mtp_operation {
                ui.spinner();
                ui.label(format!("MTP {operation}..."));
                if ui.button("Cancel").clicked() {
                    cancel_mtp = true;
                }
            }
            if let Some(notice) = &self.notice {
                ui.colored_label(egui::Color32::LIGHT_GREEN, notice);
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });
        if cancel_mtp {
            self.cancel_mtp_operation();
        }

        self.shortcut_editor(&ctx);
        self.burst_settings_editor(&ctx);
        self.mtp_picker(&ctx);
        self.import_review(&ctx);
        self.reject_review(&ctx);
        self.keep_all_confirmation(&ctx);
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
