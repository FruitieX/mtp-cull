use crate::culling::{Action, Database, Decision, Session, Shot, apply_rejects, load_session};
use color_eyre::eyre::Result;
use eframe::egui;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

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
    session: Option<Session>,
    loading: Option<Receiver<Result<Session>>>,
    selected: usize,
    zoom: Option<f32>,
    compare_next: bool,
    sort_by_sharpness: bool,
    show_shortcuts: bool,
    show_reject_review: bool,
    show_keep_all_confirmation: bool,
    error: Option<String>,
    notice: Option<String>,
}

impl MyApp {
    fn new() -> Result<Self> {
        let database = Database::open()?;
        let keybindings = database.keybindings()?;
        Ok(Self {
            database,
            keybindings,
            session: None,
            loading: None,
            selected: 0,
            zoom: None,
            compare_next: false,
            sort_by_sharpness: false,
            show_shortcuts: false,
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
        self.selected = 0;
        self.zoom = None;
        self.compare_next = false;
        self.error = None;
        self.notice = None;
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.loading else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(session)) => {
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

    fn selected_shot(&self) -> Option<&Shot> {
        self.session.as_ref()?.shots.get(self.selected)
    }

    fn move_selection(&mut self, amount: isize) {
        let Some(session) = &self.session else {
            return;
        };
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(session.shots.len().saturating_sub(1));
        self.zoom = None;
        self.compare_next = false;
    }

    fn set_decision(&mut self, decision: Decision) {
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
            && self
                .session
                .as_ref()
                .is_some_and(|session| self.selected + 1 < session.shots.len())
        {
            self.compare_next = !self.compare_next;
        }
    }

    fn set_all_keep(&mut self) {
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
            session
                .shots
                .sort_by(|left, right| left.stem.cmp(&right.stem));
            self.notice = Some("Sorted by filename".to_owned());
        }
        self.selected = 0;
        self.zoom = None;
        self.compare_next = false;
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let mut decision = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open album").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.begin_loading(path, None);
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
                .add_enabled(self.session.is_some(), egui::Button::new("All Keep"))
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
                    self.session.is_some(),
                    egui::Button::new(if self.sort_by_sharpness {
                        "Sort by filename"
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
            ui.separator();
            if ui
                .add_enabled(
                    self.session.is_some(),
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
                    self.session.is_some(),
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
                    self.session.is_some(),
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
                    self.selected_shot().is_some(),
                    egui::Button::new(if self.zoom.is_some() { "Fit" } else { "100%" }),
                )
                .clicked()
            {
                self.zoom = if self.zoom.is_some() { None } else { Some(1.0) };
            }
            let has_compare_target = self
                .session
                .as_ref()
                .is_some_and(|session| self.selected + 1 < session.shots.len());
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
        });
        if let Some(decision) = decision {
            self.set_decision(decision);
        }
    }

    fn shot_list(&mut self, ui: &mut egui::Ui) {
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
                if ui.selectable_label(index == self.selected, label).clicked() {
                    self.selected = index;
                    self.zoom = None;
                    self.compare_next = false;
                }
            }
        });
    }

    fn image_viewer(&mut self, ui: &mut egui::Ui) {
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
            if let Some(sharpness) = shot.sharpness() {
                ui.label(format!("Sharpness: {sharpness:.0}"));
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
        let Ok(uri) = url::Url::from_file_path(&asset.path) else {
            ui.colored_label(egui::Color32::LIGHT_RED, "Invalid image path");
            return;
        };

        let available = ui.available_size();
        let image = egui::Image::from_uri(uri.as_str());
        match image.load_for_size(ui.ctx(), available) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                let natural_size = texture.size;
                let fit_scale = (available.x / natural_size.x)
                    .min(available.y / natural_size.y)
                    .min(1.0);
                let scale = self.zoom.unwrap_or(fit_scale);
                let desired_size = natural_size * scale;
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let response = ui.add(
                            egui::Image::from_texture(texture)
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
            }
            Ok(egui::load::TexturePoll::Pending { .. }) => {
                ui.centered_and_justified(|ui| ui.spinner());
            }
            Err(error) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("Could not load image: {error}"),
                );
            }
        }
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
        self.handle_shortcuts(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| self.top_bar(ui));
        egui::Panel::right("shots")
            .default_size(280.0)
            .show(ui, |ui| self.shot_list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.image_viewer(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            if self.loading.is_some() {
                ui.spinner();
                ui.label("Scanning, pairing, and fingerprinting files in the background...");
            }
            if let Some(notice) = &self.notice {
                ui.colored_label(egui::Color32::LIGHT_GREEN, notice);
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });

        self.shortcut_editor(&ctx);
        self.reject_review(&ctx);
        self.keep_all_confirmation(&ctx);
    }
}
