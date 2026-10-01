//! Feature-gated native renderer exercise; deliberately separate from normal UI.
use super::*;
use color_eyre::eyre::{bail, eyre};
use std::collections::BTreeSet;
pub(super) struct Smoke {
    pub blink: bool,
    root: PathBuf,
    step: u8,
    frames: u32,
    started: Instant,
    waiting: Option<String>,
    received: BTreeSet<String>,
    samples: Vec<f64>,
}
impl Smoke {
    pub fn from_environment() -> Result<Option<Self>> {
        let Some(root) = std::env::var_os("MTP_CULL_SMOKE_DIR").map(PathBuf::from) else {
            return Ok(None);
        };
        std::fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        let data = std::env::var_os("MTP_CULL_DATA_DIR")
            .ok_or_else(|| eyre!("UI smoke requires isolated MTP_CULL_DATA_DIR"))?;
        let data = PathBuf::from(data);
        std::fs::create_dir_all(&data)?;
        if !data.canonicalize()?.starts_with(&root) {
            bail!("smoke data must be inside smoke output directory");
        }
        Ok(Some(Self {
            root,
            step: 0,
            frames: 0,
            started: Instant::now(),
            waiting: None,
            received: BTreeSet::new(),
            samples: Vec::new(),
            blink: false,
        }))
    }
    pub fn fail(&mut self, ctx: &egui::Context, message: &str) {
        let _ = std::fs::remove_file(self.root.join("PASS.txt"));
        let _ = std::fs::write(self.root.join("FAILED.txt"), message);
        self.step = 255;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    fn capture(&mut self, ctx: &egui::Context, name: &str) {
        self.waiting = Some(name.into());
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
            name.to_owned(),
        )));
        self.frames = 0;
    }
    pub fn tick(&mut self, app: &mut App, ctx: &egui::Context) -> Result<()> {
        if self.step == 255 {
            return Ok(());
        }
        if self.started.elapsed() > Duration::from_secs(180) {
            bail!("smoke timed out at step {}", self.step);
        }
        if let Some(error) = &app.error {
            bail!("UI error at step {}: {error}", self.step);
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot {
                user_data, image, ..
            } = event
            {
                let name = user_data
                    .data
                    .as_ref()
                    .and_then(|d| d.downcast_ref::<String>())
                    .ok_or_else(|| eyre!("missing screenshot name"))?;
                let rgba = image
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.root.join(format!("{name}.png")),
                    &rgba,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )?;
                self.received.insert(name.clone());
            }
        }
        if self
            .waiting
            .as_ref()
            .is_some_and(|name| !self.received.contains(name))
        {
            ctx.request_repaint();
            return Ok(());
        }
        self.waiting = None;
        self.frames += 1;
        ctx.request_repaint_after(Duration::from_millis(5));
        if app.loader.is_some() || app.cache.pending() || self.frames < 4 {
            return Ok(());
        }
        // Allow popup animation to finish before judging editor layout.
        if matches!(self.step, 25 | 31) && self.frames < 40 {
            return Ok(());
        }
        match self.step {
            0 if std::env::var_os("MTP_CULL_SMOKE_HOME_ONLY").is_some() => {
                if app.session.is_some() {
                    bail!("default launch should open the home page");
                }
                self.capture(ctx, "welcome-default");
                self.step = 30;
            }
            30 => {
                app.command(Command::Presets);
                if let Some(preset) = app.presets.first() {
                    app.preset_edit = Some((Some(0), preset.clone()));
                }
                self.step = 31;
                self.frames = 0;
            }
            31 => {
                self.capture(ctx, "import-presets");
                self.step = 32;
            }
            32 => {
                std::fs::write(
                    self.root.join("PASS.txt"),
                    "PASS: no-argument UI launch, home page, import preset editor.\n",
                )?;
                self.step = 255;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            0 => {
                if app.session.as_ref().is_none_or(|s| s.shots.len() < 3) {
                    bail!("smoke requires at least three paired photos");
                }
                app.command(Command::Clear);
                app.command(Command::Keep);
                app.command(Command::Pin);
                app.command(Command::Keep);
                app.command(Command::Next);
                app.command(Command::Reject);
                app.command(Command::Undo);
                app.command(Command::Redo);
                app.command(Command::Previous);
                let ids = app.session.as_ref().unwrap().selected_ids(&app.settings);
                if ids.len() != 5 {
                    bail!(
                        "linked selection expected two JPEG/RAW pairs and one video, got {}",
                        ids.len()
                    );
                }
                self.step = 1;
                self.frames = 0;
            }
            1 => {
                self.capture(ctx, "side-by-side");
                self.step = 20;
            }
            20 => {
                self.blink = true;
                self.step = 21;
                self.frames = 0;
            }
            21 => {
                self.capture(ctx, "blink");
                self.step = 22;
            }
            22 => {
                self.blink = false;
                self.step = 2;
                self.frames = 0;
            }
            2 => {
                app.canvas.mode = Mode::Wipe;
                self.step = 3;
                self.frames = 0;
            }
            3 => {
                self.capture(ctx, "wipe");
                self.step = 4;
            }
            4 => {
                app.command(Command::Zoom);
                self.step = 5;
                self.frames = 0;
            }
            5 => {
                self.capture(ctx, "native");
                self.step = 6;
            }
            6 => {
                app.command(Command::Peaking);
                app.canvas.region = Some(egui::Rect::from_center_size(
                    egui::pos2(3864.0, 2576.0),
                    egui::vec2(300.0, 300.0),
                ));
                self.step = 7;
                self.frames = 0;
            }
            7 => {
                self.capture(ctx, "focus-region");
                self.step = 8;
            }
            8 => {
                app.canvas.peaking = false;
                app.canvas.region = None;
                app.canvas.viewport.zoom = None;
                app.settings.picture_root = self.root.join("import/jpeg").display().to_string();
                app.settings.raw_root = self.root.join("import/raw").display().to_string();
                app.settings.video_root = self.root.join("import/video").display().to_string();
                app.settings.album = "Smoke".into();
                app.date = "2026-09-30".into();
                app.start_import(app.session.as_ref().unwrap().selected_ids(&app.settings));
                self.step = 9;
            }
            9 => {
                if app.import.is_some() {
                    return Ok(());
                }
                if !app
                    .message
                    .as_deref()
                    .is_some_and(|m| m.contains("Imported 5 files"))
                {
                    bail!("unexpected import result {:?}", app.message);
                }
                app.start_import(app.session.as_ref().unwrap().selected_ids(&app.settings));
                self.step = 10;
            }
            10 => {
                if app.import.is_some() {
                    return Ok(());
                }
                if !app
                    .message
                    .as_deref()
                    .is_some_and(|m| m.contains("5 already present"))
                {
                    bail!("retry did not skip identical files: {:?}", app.message);
                }
                self.step = 11;
                self.frames = 0;
            }
            11 => {
                self.samples.push(app.frame_ms);
                if self.frames.is_multiple_of(4) {
                    app.navigate(if (self.frames / 4).is_multiple_of(2) {
                        1
                    } else {
                        -1
                    });
                }
                if self.frames >= 120 {
                    self.samples.sort_by(f64::total_cmp);
                    let n = self.samples.len();
                    let text = format!(
                        "PASS: linked selections, keep/reject/undo/redo, comparison, native detail, peaking/ROI, import 5 selected assets, identical retry.\nNative renderer: {}x{} points, DPI {}\nWarm UI CPU frames ({} samples): median {:.3} ms, p95 {:.3} ms, max {:.3} ms\nGPU resident {:.1} MiB; CPU resident {:.1} MiB\nPresentation latency is not measured by these CPU timings.\n",
                        ctx.content_rect().width(),
                        ctx.content_rect().height(),
                        ctx.pixels_per_point(),
                        n,
                        self.samples[n / 2],
                        self.samples[n * 95 / 100],
                        self.samples[n - 1],
                        app.canvas.bytes() as f64 / 1048576.0,
                        app.cache.bytes() as f64 / 1048576.0
                    );
                    let session = app.session.as_ref().unwrap().clone();
                    let selected = app.selected;
                    let pinned = app.pinned;
                    app.command(Command::Close);
                    app.install(session);
                    if app.selected != selected
                        || app.pinned != pinned
                        || app
                            .session
                            .as_ref()
                            .unwrap()
                            .selected_ids(&app.settings)
                            .len()
                            != 5
                    {
                        bail!("resume lost view position or linked selections");
                    }
                    std::fs::write(
                        self.root.join("PASS.txt"),
                        format!("{text}Session resume: PASS\n"),
                    )?;
                    app.command(Command::Settings);
                    self.step = 12;
                    self.frames = 0;
                }
            }
            12 => {
                self.capture(ctx, "settings");
                self.step = 13;
            }
            13 => {
                app.settings_tab = 1;
                self.step = 16;
                self.frames = 0;
            }
            16 => {
                self.capture(ctx, "settings-performance");
                self.step = 17;
            }
            17 => {
                app.settings_tab = 2;
                self.step = 18;
                self.frames = 0;
            }
            18 => {
                self.capture(ctx, "settings-shortcuts");
                self.step = 19;
            }
            19 => {
                app.settings_open = false;
                if app
                    .settings
                    .recent_sources
                    .iter()
                    .all(|source| !matches!(source, RecentSource::Local { .. }))
                {
                    bail!("successful folder open was not remembered");
                }
                // Display a camera shortcut without contacting any physical camera.
                recent_sources::remember(
                    &mut app.settings.recent_sources,
                    RecentSource::Camera {
                        device_id: "smoke-device".into(),
                        device_name: "Fujifilm X-T5".into(),
                        folder_id: "smoke-folder".into(),
                        folder_path: "DCIM/100_FUJI".into(),
                    },
                );
                app.command(Command::Close);
                self.step = 14;
                self.frames = 0;
            }
            14 => {
                self.capture(ctx, "welcome");
                self.step = 15;
            }
            15 => {
                let source = app
                    .settings
                    .recent_sources
                    .iter()
                    .find(|source| matches!(source, RecentSource::Local { .. }))
                    .unwrap()
                    .clone();
                app.open_recent(source);
                self.step = 23;
                self.frames = 0;
            }
            23 => {
                if app
                    .session
                    .as_ref()
                    .is_none_or(|session| session.selected_ids(&app.settings).len() != 5)
                {
                    bail!("reopening a recent folder lost linked keep/reject choices");
                }
                self.capture(ctx, "recent-reopened");
                let mut text = std::fs::read_to_string(self.root.join("PASS.txt"))?;
                text.push_str("Recent folder reopen with saved selections: PASS\n");
                std::fs::write(self.root.join("PASS.txt"), text)?;
                self.step = 24;
            }
            24 => {
                app.command(Command::Close);
                app.command(Command::Presets);
                if let Some(preset) = app.presets.first() {
                    app.preset_edit = Some((Some(0), preset.clone()));
                }
                self.step = 25;
                self.frames = 0;
            }
            25 => {
                self.capture(ctx, "import-presets");
                self.step = 26;
            }
            26 => {
                self.step = 255;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => bail!("invalid smoke step"),
        }
        Ok(())
    }
}
