use super::*;

impl App {
    pub(super) fn home_presets(&mut self, ui: &mut egui::Ui) {
        ui.add_space(20.0);
        ui.label(
            egui::RichText::new("Import without reviewing")
                .small()
                .color(theme::MUTED),
        );
        ui.add_space(8.0);
        if self.presets.is_empty() {
            if ui.button("Add import preset").clicked() {
                self.command(Command::Presets);
            }
        } else {
            let mut chosen = None;
            ui.allocate_ui_with_layout(
                egui::vec2(
                    ui.available_width().min(440.0),
                    (self.presets.len() as f32 * 42.0).min(170.0),
                ),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("home-presets")
                        .max_height(170.0)
                        .show(ui, |ui| {
                            for preset in &self.presets {
                                if ui
                                    .add_enabled(
                                        self.can_run_preset(),
                                        egui::Button::new(format!("Import · {}", preset.name))
                                            .min_size(egui::vec2(ui.available_width(), 30.0))
                                            .truncate(),
                                    )
                                    .on_hover_text(preset.detail())
                                    .clicked()
                                {
                                    chosen = Some(preset.clone());
                                }
                                ui.add_space(4.0);
                            }
                        });
                },
            );
            if let Some(preset) = chosen {
                self.start_preset(&preset);
            }
            ui.add_space(6.0);
            if ui.small_button("Manage import presets").clicked() {
                self.command(Command::Presets);
            }
        }
        if let Some(error) = &self.preset_error {
            ui.colored_label(theme::REJECT, "Could not load import presets")
                .on_hover_text(error);
        }
    }

    fn write_presets(&mut self, presets: Vec<ImportPreset>) -> bool {
        match import_presets::save(&self.presets_path, &presets) {
            Ok(()) => {
                self.presets = presets;
                self.preset_error = None;
                true
            }
            Err(error) => {
                self.preset_error = Some(format!("{error:#}"));
                false
            }
        }
    }

    pub(super) fn presets_dialog(&mut self, ctx: &egui::Context) {
        if !self.presets_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Import presets")
            .open(&mut open)
            .default_width(680.0)
            .default_height(640.0)
            .min_height(360.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(350.0, 320.0))
            .max_height((ctx.content_rect().height() - 90.0).max(300.0))
            .show(ctx, |ui| {
                ui.label("Copy all supported photos, RAW files and videos without culling.");
                ui.label(
                    egui::RichText::new("Files go to destination / year / date album / filename.")
                        .small()
                        .color(theme::MUTED),
                );
                ui.horizontal(|ui| {
                    if ui.button("New preset").clicked() {
                        self.preset_edit = Some((None, ImportPreset::default()));
                    }
                    if ui.button("Reload file").clicked() {
                        match import_presets::load(&self.presets_path) {
                            Ok(presets) => {
                                self.presets = presets;
                                self.preset_error = None;
                                self.preset_edit = None;
                            }
                            Err(error) => self.preset_error = Some(format!("{error:#}")),
                        }
                    }
                });
                if let Some(error) = &self.preset_error {
                    ui.colored_label(theme::REJECT, error);
                }
                ui.separator();
                let mut edit = self.preset_edit.take();
                egui::ScrollArea::vertical()
                    .id_salt("preset-editor")
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - 120.0).clamp(180.0, 540.0))
                    .show(ui, |ui| {
                        let mut run = None;
                        let mut remove = None;
                        egui::ScrollArea::vertical()
                            .id_salt("saved-presets-list")
                            .max_height(if edit.is_some() { 100.0 } else { 280.0 })
                            .show(ui, |ui| {
                                for (index, preset) in self.presets.clone().iter().enumerate() {
                                    ui.push_id(index, |ui| {
                                        ui.horizontal(|ui| {
                                            if ui
                                                .add_enabled(
                                                    self.can_run_preset(),
                                                    egui::Button::new("Import"),
                                                )
                                                .on_hover_text(preset.detail())
                                                .clicked()
                                            {
                                                run = Some(preset.clone());
                                            }
                                            ui.label(egui::RichText::new(&preset.name).strong())
                                                .on_hover_text(preset.detail());
                                            if ui.small_button("Edit").clicked() {
                                                edit = Some((Some(index), preset.clone()));
                                            }
                                            if ui.small_button("Duplicate").clicked() {
                                                let mut draft = preset.clone();
                                                draft.name = format!("{} copy", draft.name);
                                                edit = Some((None, draft));
                                            }
                                            if ui.small_button("Delete").clicked() {
                                                remove = Some(index);
                                            }
                                        });
                                    });
                                }
                            });
                        if let Some(index) = remove {
                            let mut presets = self.presets.clone();
                            presets.remove(index);
                            if self.write_presets(presets) {
                                edit = None;
                            }
                        }
                        if let Some(preset) = run {
                            self.start_preset(&preset);
                            self.presets_open = false;
                        }
                        if self.session.is_some() {
                            ui.weak("Close the review session to run a preset from the home page.");
                        }
                        if let Some((index, mut draft)) = edit.take() {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.strong(if index.is_some() {
                                "Edit preset"
                            } else {
                                "New preset"
                            });
                            if !self.settings.recent_sources.is_empty() {
                                ui.menu_button("Use a recent camera or phone", |ui| {
                                    for source in &self.settings.recent_sources {
                                        if let RecentSource::Camera {
                                            device_name,
                                            folder_path,
                                            ..
                                        } = source
                                            && ui
                                                .button(source.label())
                                                .on_hover_text(source.detail())
                                                .clicked()
                                        {
                                            draft.device = device_name.clone();
                                            draft.source_path = folder_path.clone();
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            egui::Grid::new("preset-fields")
                                .num_columns(2)
                                .spacing(egui::vec2(14.0, 8.0))
                                .show(ui, |ui| {
                                    for (label, value, hint) in [
                                        ("Preset name", &mut draft.name, "Phone / photographer"),
                                        (
                                            "Device",
                                            &mut draft.device,
                                            "Blank: first connected device",
                                        ),
                                        (
                                            "Source folder",
                                            &mut draft.source_path,
                                            "Blank: device root",
                                        ),
                                    ] {
                                        ui.label(label);
                                        ui.add(
                                            egui::TextEdit::singleline(value)
                                                .hint_text(hint)
                                                .desired_width(410.0),
                                        );
                                        ui.end_row();
                                    }
                                    for (label, value) in [
                                        ("Photo destination", &mut draft.pictures_path),
                                        ("Video destination", &mut draft.videos_path),
                                        ("RAW destination", &mut draft.raw_path),
                                    ] {
                                        ui.label(label);
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(value)
                                                    .desired_width(335.0),
                                            );
                                            if ui.button("Browse").clicked()
                                                && let Some(path) =
                                                    rfd::FileDialog::new().pick_folder()
                                            {
                                                *value = path.display().to_string();
                                            }
                                        });
                                        ui.end_row();
                                    }
                                    for (label, value, hint) in [
                                        ("Date", &mut draft.date, "Blank: today. Or YYYY-MM-DD"),
                                        ("Album", &mut draft.album_name, "Optional album name"),
                                    ] {
                                        ui.label(label);
                                        ui.add(
                                            egui::TextEdit::singleline(value)
                                                .hint_text(hint)
                                                .desired_width(410.0),
                                        );
                                        ui.end_row();
                                    }
                                });
                            ui.checkbox(
                                &mut draft.keep_going,
                                "Continue if an individual file fails",
                            );
                            edit = Some((index, draft));
                        }
                    });
                ui.separator();
                if let Some((index, mut draft)) = edit {
                    let validation = draft.args(Local::now().date_naive()).and_then(|_| {
                        if self.presets.iter().enumerate().any(|(i, preset)| {
                            Some(i) != index
                                && preset.name.trim().eq_ignore_ascii_case(draft.name.trim())
                        }) {
                            color_eyre::eyre::bail!("A preset with that name already exists");
                        }
                        Ok(())
                    });
                    if let Err(error) = &validation {
                        ui.weak(error.to_string());
                    }
                    let mut save = false;
                    let mut cancel = false;
                    ui.horizontal(|ui| {
                        save = ui
                            .add_enabled(validation.is_ok(), egui::Button::new("Save preset"))
                            .clicked();
                        cancel = ui.button("Cancel edit").clicked();
                    });
                    if save {
                        draft.name = draft.name.trim().into();
                        let mut presets = self.presets.clone();
                        if let Some(index) = index {
                            presets[index] = draft.clone();
                        } else {
                            presets.push(draft.clone());
                        }
                        cancel = self.write_presets(presets);
                    }
                    if !cancel {
                        self.preset_edit = Some((index, draft));
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(self.presets_path.display().to_string())
                                .small()
                                .color(theme::MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(self.presets_path.display().to_string());
                    if ui.small_button("Copy path").clicked() {
                        ctx.copy_text(self.presets_path.display().to_string());
                    }
                });
            });
        if !open {
            self.presets_open = false;
            self.preset_edit = None;
        }
    }
}
