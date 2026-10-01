//! Feature-gated native renderer exercise; deliberately separate from normal UI.
use super::*;
use color_eyre::eyre::{bail, eyre};
use std::collections::BTreeSet;
const REEL_SHOTS: usize = 500;
pub(super) struct Smoke {
    pub blink: bool,
    pub input: Vec<egui::Event>,
    root: PathBuf,
    step: u8,
    frames: u32,
    started: Instant,
    waiting: Option<String>,
    received: BTreeSet<String>,
    samples: Vec<f64>,
}
impl Smoke {
    pub fn input_ready(&self) -> bool {
        self.waiting.is_none()
    }
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
            input: Vec::new(),
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
    fn click(&mut self, pos: egui::Pos2, button: egui::PointerButton, modifiers: egui::Modifiers) {
        self.input.extend([
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers,
            },
            egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers,
            },
        ]);
    }
    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.input.push(egui::Event::ModifiersChanged(modifiers));
        self.input.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        self.input.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        });
    }
    fn tick_reel(&mut self, app: &mut App, ctx: &egui::Context) -> Result<()> {
        let ctrl = egui::Modifiers::CTRL;
        match self.step {
            0 => {
                app.canvas.verify_mipmaps()?;
                let source = app
                    .session
                    .as_ref()
                    .ok_or_else(|| eyre!("reel smoke needs JPEG fixtures"))?;
                let originals = source
                    .shots
                    .iter()
                    .filter(|s| s.has(Kind::Jpeg))
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>();
                if originals.len() != 3 {
                    bail!("reel smoke needs three paired photos");
                }
                let mut session = source.clone();
                session.id.push_str("-synthetic-reel-smoke");
                session.shots = (0..REEL_SHOTS)
                    .map(|i| {
                        let mut shot = originals[i % 3].clone();
                        shot.id = format!("reel-{i}");
                        shot.name = format!("PHOTO_{i:04}");
                        for asset in &mut shot.assets {
                            asset.id = format!("reel-{i}-{:?}", asset.kind);
                            asset.key = asset.id.clone();
                            asset.decision = Decision::Unreviewed;
                        }
                        shot
                    })
                    .collect();
                app.install(session);
                app.settings.reel_grid = false;
                self.step = 1;
            }
            1 => {
                for _ in 0..79 {
                    app.command(Command::Next);
                }
                self.key(egui::Key::ArrowRight, egui::Modifiers::NONE);
                self.step = 2;
            }
            2 => {
                if app.selected != 80
                    || !app
                        .reel_cells
                        .iter()
                        .any(|(i, r)| *i == 80 && app.reel_clip.contains(r.center()))
                {
                    bail!(
                        "row did not follow keyboard navigation: {} / {:?}",
                        app.selected,
                        app.reel_viewport
                    );
                }
                self.capture(ctx, "reel-row-follow");
                self.step = 3;
            }
            3 if app.selected == 80 => {
                app.selected = 0;
                app.reel_selection.single(0);
                app.reel_follow = true;
                self.frames = 0;
                return Ok(());
            }
            3 => {
                let rect = app
                    .reel_cells
                    .iter()
                    .find(|(i, _)| *i == 2)
                    .ok_or_else(|| eyre!("third thumbnail missing"))?
                    .1;
                self.click(rect.center(), egui::PointerButton::Primary, ctrl);
                self.step = 4;
            }
            4 => {
                if app
                    .reel_selection
                    .indices
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    != [0, 2]
                {
                    bail!("Ctrl-click did not add to selection");
                }
                self.capture(ctx, "reel-multiselect");
                self.key(egui::Key::A, ctrl);
                self.step = 5;
            }
            5 => {
                if app.reel_selection.indices.len() != REEL_SHOTS
                    || !app
                        .session
                        .as_ref()
                        .unwrap()
                        .selected_ids(&app.settings)
                        .is_empty()
                {
                    bail!("Ctrl+A changed decisions or failed to select all");
                }
                self.key(egui::Key::Num1, egui::Modifiers::NONE);
                self.step = 6;
            }
            6 => {
                if app
                    .session
                    .as_ref()
                    .unwrap()
                    .selected_ids(&app.settings)
                    .len()
                    != REEL_SHOTS * 2
                {
                    bail!("1 did not keep all selected JPEG/RAW pairs");
                }
                self.key(egui::Key::Num0, egui::Modifiers::NONE);
                self.step = 7;
            }
            7 => {
                if !app
                    .session
                    .as_ref()
                    .unwrap()
                    .selected_ids(&app.settings)
                    .is_empty()
                {
                    bail!("0 did not clear batch decisions");
                }
                let rect = app.reel_cells.iter().find(|(i, _)| *i == 2).unwrap().1;
                self.click(
                    rect.center(),
                    egui::PointerButton::Secondary,
                    egui::Modifiers::NONE,
                );
                self.step = 8;
            }
            8 => {
                let item = app
                    .reel_menu_items
                    .iter()
                    .find(|(c, _)| *c == Command::Reject)
                    .ok_or_else(|| eyre!("thumbnail context menu did not open"))?
                    .1;
                self.capture(ctx, "reel-context-menu");
                self.click(
                    item.center(),
                    egui::PointerButton::Primary,
                    egui::Modifiers::NONE,
                );
                self.step = 9;
            }
            9 => {
                if !app
                    .session
                    .as_ref()
                    .unwrap()
                    .shots
                    .iter()
                    .all(|s| s.decision(Kind::Jpeg, true) == Decision::Reject)
                {
                    bail!("context menu failed to reject the batch");
                }
                app.command(Command::ReelMode);
                for _ in 0..78 {
                    app.command(Command::Next);
                }
                self.step = 10;
            }
            10 => {
                if !app.settings.reel_grid
                    || app.reel_viewport.min.y <= 0.0
                    || !app
                        .reel_cells
                        .iter()
                        .any(|(i, r)| *i == 80 && app.reel_clip.contains(r.center()))
                {
                    bail!("grid did not follow navigation");
                }
                self.capture(ctx, "reel-grid-follow");
                let rect = app
                    .canvas
                    .pane_rects
                    .iter()
                    .find(|(b, _)| *b)
                    .ok_or_else(|| eyre!("canvas not loaded"))?
                    .1;
                self.click(
                    rect.center(),
                    egui::PointerButton::Secondary,
                    egui::Modifiers::NONE,
                );
                self.step = 11;
            }
            11 => {
                let item = app
                    .canvas
                    .menu_items
                    .iter()
                    .find(|(c, _)| *c == Command::Keep)
                    .ok_or_else(|| eyre!("canvas context menu did not open"))?
                    .1;
                self.capture(ctx, "canvas-context-menu");
                self.click(
                    item.center(),
                    egui::PointerButton::Primary,
                    egui::Modifiers::NONE,
                );
                self.step = 12;
            }
            12 => {
                if app.session.as_ref().unwrap().shots[80].decision(Kind::Jpeg, true)
                    != Decision::Keep
                {
                    bail!("canvas keep action failed");
                }
                let before = app.reel_viewport.min.y;
                std::fs::write(self.root.join("scroll-before.txt"), before.to_string())?;
                self.input.extend([
                    egui::Event::PointerMoved(app.reel_clip.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0.0, -120.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                self.step = 13;
            }
            13 => {
                let before =
                    std::fs::read_to_string(self.root.join("scroll-before.txt"))?.parse::<f32>()?;
                if app.reel_viewport.min.y <= before {
                    bail!("ordinary wheel did not scroll grid");
                }
                app.command(Command::ReelMode);
                self.step = 14;
            }
            14 => {
                std::fs::write(
                    self.root.join("scroll-before.txt"),
                    app.reel_viewport.min.x.to_string(),
                )?;
                self.input.extend([
                    egui::Event::PointerMoved(app.reel_clip.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0.0, -120.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                self.step = 15;
            }
            15 => {
                let before =
                    std::fs::read_to_string(self.root.join("scroll-before.txt"))?.parse::<f32>()?;
                if app.reel_viewport.min.x <= before {
                    bail!("ordinary wheel did not scroll row horizontally");
                }
                // Exercise the actual panel edge with press/move/release events.
                let edge = app.reel_panel_bounds.center_top();
                std::fs::write(
                    self.root.join("height-before.txt"),
                    app.settings.reel_height.to_string(),
                )?;
                self.input.extend([
                    egui::Event::PointerMoved(edge),
                    egui::Event::PointerButton {
                        pos: edge,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                self.step = 16;
            }
            16 => {
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap() - egui::vec2(0.0, 80.0);
                self.input.push(egui::Event::PointerMoved(pos));
                self.step = 17;
            }
            17 => {
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap();
                self.input.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                self.step = 18;
            }
            18 => {
                let before =
                    std::fs::read_to_string(self.root.join("height-before.txt"))?.parse::<f32>()?;
                if app.settings.reel_height <= before + 20.0 {
                    bail!(
                        "panel edge did not resize reel: {before} -> {}",
                        app.settings.reel_height
                    );
                }
                self.capture(ctx, "reel-resized");
                self.step = 19;
            }
            19 if !app.settings_open => {
                app.command(Command::Settings);
                app.settings_tab = 1;
                self.frames = 0;
                return Ok(());
            }
            19 => {
                self.capture(ctx, "reel-performance-settings");
                self.step = 20;
            }
            20 => {
                std::fs::write(
                    self.root.join("PASS.txt"),
                    "PASS: synthetic 500-photo reel; keyboard-follow in row/grid; Ctrl-click batch; Ctrl+A selection; 1/0 batch decisions; thumbnail and canvas context actions; normal-wheel scrolling; native panel resize; GPU checkerboard low-pass readback.\n",
                )?;
                self.step = 255;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => bail!("invalid reel smoke step"),
        }
        self.frames = 0;
        Ok(())
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
        if (matches!(self.step, 25 | 31)
            || (std::env::var_os("MTP_CULL_SMOKE_REEL").is_some()
                && matches!(self.step, 8 | 11 | 19)))
            && self.frames < 40
        {
            return Ok(());
        }
        if std::env::var_os("MTP_CULL_SMOKE_REEL").is_some() {
            return self.tick_reel(app, ctx);
        }
        match self.step {
            0 if std::env::var_os("MTP_CULL_SMOKE_HOME_ONLY").is_some() => {
                #[cfg(windows)]
                if std::env::var_os("MTP_CULL_SMOKE_EXPECT_DETACHED").is_some()
                    && crate::windows_app::console_process_count() != 0
                {
                    bail!("Explorer-style launch retained its owned console");
                }
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
