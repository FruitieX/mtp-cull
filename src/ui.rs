use color_eyre::Result;
use eframe::egui;
use std::path::PathBuf;

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
            Ok(Box::<MyApp>::default())
        }),
    )?;
    Ok(())
}

#[derive(Default)]
struct MyApp {
    picked_dir: Option<PathBuf>,
    picked_index: usize,
    dir_images: Vec<PathBuf>,
    error: Option<String>,
}

impl MyApp {
    fn select_directory(&mut self, path: PathBuf) {
        self.picked_index = 0;
        self.picked_dir = Some(path.clone());
        self.error = None;

        match std::fs::read_dir(&path) {
            Ok(entries) => {
                self.dir_images = entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| {
                        path.extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| {
                                extension.eq_ignore_ascii_case("jpg")
                                    || extension.eq_ignore_ascii_case("jpeg")
                            })
                    })
                    .collect();
                self.dir_images.sort();
            }
            Err(error) => {
                self.dir_images.clear();
                self.error = Some(format!("Could not read {}: {error}", path.display()));
            }
        }
    }

    fn handle_navigation(&mut self, ui: &egui::Ui) {
        let (next, previous) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowUp),
            )
        });
        if next && self.picked_index + 1 < self.dir_images.len() {
            self.picked_index += 1;
        }
        if previous && self.picked_index > 0 {
            self.picked_index -= 1;
        }
    }
}

impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_navigation(ui);

        egui::Panel::right("browser").show(ui, |ui| {
            if ui.button("Select directory").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.select_directory(path);
            }

            if let Some(directory) = &self.picked_dir {
                ui.label("Directory");
                ui.monospace(directory.display().to_string());
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.separator();

            egui::ScrollArea::vertical().show_rows(
                ui,
                24.0,
                self.dir_images.len(),
                |ui, row_range| {
                    for index in row_range {
                        let path = &self.dir_images[index];
                        let label = path
                            .file_name()
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_else(|| path.as_os_str().to_string_lossy());
                        if ui
                            .selectable_label(index == self.picked_index, label)
                            .clicked()
                        {
                            self.picked_index = index;
                        }
                    }
                },
            );
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(path) = self.dir_images.get(self.picked_index) {
                ui.label(path.display().to_string());
                if let Ok(uri) = url::Url::from_file_path(path) {
                    let image = egui::Image::from_uri(uri.as_str())
                        .shrink_to_fit()
                        .maintain_aspect_ratio(true);
                    ui.centered_and_justified(|ui| ui.add(image));
                } else {
                    ui.colored_label(egui::Color32::LIGHT_RED, "Invalid image path");
                }
            } else {
                ui.centered_and_justified(|ui| ui.label("Select a directory of JPEG images"));
            }
        });
    }
}
