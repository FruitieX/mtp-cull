#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use color_eyre::Result;
use eframe::egui;

pub fn init() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        env!("CARGO_PKG_NAME"),
        options,
        Box::new(|cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Box::<MyApp>::default()
        }),
    )
    .unwrap();

    Ok(())
}

#[derive(Default)]
struct MyApp {
    picked_dir: Option<String>,
    picked_index: usize,
    dir_images: Vec<DirImage>,
}

#[derive(Clone)]
struct DirImage {
    path: String,
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let picked_file = self.dir_images.get(self.picked_index).cloned();

        egui::SidePanel::right("panel").show(ctx, |ui| {
            if ui.button("Select directory").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.picked_index = 0;
                    self.picked_dir = Some(path.display().to_string());
                    let paths = std::fs::read_dir(path).unwrap();
                    self.dir_images = paths
                        .into_iter()
                        .map(|p| {
                            let path = p.unwrap().path();
                            let path_str = path.display().to_string();

                            DirImage {
                                path: path_str.clone(),
                            }
                        })
                        .collect();
                }
            }
            if let Some(picked_dir) = &self.picked_dir {
                ui.label("Picked directory:");
                ui.monospace(picked_dir);
            }

            if let Some(picked_file) = &picked_file {
                ui.label("Picked file:");
                ui.monospace(&picked_file.path);
            }

            ui.separator();

            egui::ScrollArea::vertical().show_rows(
                ui,
                50.,
                self.dir_images.len(),
                |ui, row_range| {
                    for image in &self.dir_images.as_slice()[row_range] {
                        let label = ui.button(&image.path);

                        if label.clicked() {
                            let index = self
                                .dir_images
                                .iter()
                                .position(|i| i.path == image.path)
                                .unwrap();

                            self.picked_index = index;
                        }

                        // if self.dir_images[*self.picked_file] == Some(&image.path) {
                        //     label.highlight();
                        // }
                    }
                },
            );
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.input(|i| {
                let next_pressed =
                    i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowDown);

                let prev_pressed =
                    i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowUp);

                if next_pressed && self.picked_index < self.dir_images.len() {
                    let index = self.picked_index + 1;
                    self.picked_index = index;
                }

                if prev_pressed && self.picked_index > 0 {
                    let index = self.picked_index - 1;
                    self.picked_index = index;
                }
            });

            if let Some(picked_file) = &picked_file {
                let path = &picked_file.path;
                let uri = format!("file://{path}");
                let image = egui::Image::new(uri)
                    .shrink_to_fit()
                    .maintain_aspect_ratio(true);
                ui.centered_and_justified(|ui| ui.add(image));
            }
        });
    }
}
