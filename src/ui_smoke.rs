//! Feature-gated native renderer exercise; deliberately separate from normal UI.
use super::*;
use color_eyre::eyre::{bail, eyre};
use std::collections::BTreeSet;
const REEL_SHOTS: usize = 500;
#[derive(Default)]
struct ResizeProbe {
    case: usize,
    phase: usize,
    edge: egui::Pos2,
    anchor: f32,
    samples: Vec<String>,
}
impl ResizeProbe {
    fn tick(
        &mut self,
        app: &mut App,
        input: &mut Vec<egui::Event>,
        root: &std::path::Path,
    ) -> Result<bool> {
        let position = Position::ALL[self.case / 2];
        let grid = self.case % 2 == 1;
        let vertical = position.is_side() || grid;
        let axis = usize::from(vertical);
        match self.phase {
            0 => {
                app.set_reel_position(position);
                app.settings.reel_grid = grid;
                app.settings.reel_thumbnail_size = 280.0;
                app.selected = 200;
                app.reel_selection.single(200);
                app.reel_follow = true;
                input.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            }
            1..=5 => {}
            6 => {
                let rect = app
                    .reel_cells
                    .iter()
                    .find(|(i, _)| *i == 200)
                    .ok_or_else(|| eyre!("resize probe could not find active photo"))?
                    .1;
                self.anchor =
                    (rect.center()[axis] - app.reel_clip.min[axis]) / app.reel_clip.size()[axis];
                self.edge = match position {
                    Position::Bottom => app.reel_panel_bounds.center_top(),
                    Position::Left => app.reel_panel_bounds.right_center(),
                    Position::Right => app.reel_panel_bounds.left_center(),
                };
                input.extend([
                    egui::Event::PointerMoved(self.edge),
                    egui::Event::PointerButton {
                        pos: self.edge,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            7..=49 => {
                let selected = app
                    .reel_cells
                    .iter()
                    .find(|(i, _)| *i == 200)
                    .map(|(_, r)| r.center()[axis]);
                let expected = app.reel_clip.min[axis] + self.anchor * app.reel_clip.size()[axis];
                let drift = selected.map(|actual| actual - expected);
                self.samples.push(format!(
                    "{position:?},{grid},{},{:?},{},{},{:?}",
                    self.phase,
                    app.reel_panel_bounds.size(),
                    app.reel_viewport.min[axis],
                    expected,
                    drift
                ));
                std::fs::write(root.join("resize-trace.csv"), self.samples.join("\n"))?;
                if drift.is_none_or(|d| d.abs() > 2.5) {
                    bail!(
                        "live resize jumped in {position:?} / grid={grid} / frame {}: drift {drift:?}",
                        self.phase
                    );
                }
                if self.phase < 47 {
                    let step = self.phase - 6;
                    let distance = if step <= 20 { step } else { 40 - step } as f32 * 4.0;
                    let delta = match position {
                        Position::Bottom => egui::vec2(0.0, -distance),
                        Position::Left => egui::vec2(-distance, 0.0),
                        Position::Right => egui::vec2(distance, 0.0),
                    };
                    input.push(egui::Event::PointerMoved(self.edge + delta));
                } else if self.phase == 47 {
                    input.push(egui::Event::PointerButton {
                        pos: self.edge,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                } else if self.phase == 49 {
                    self.case += 1;
                    self.phase = 0;
                    if self.case == 6 {
                        for _ in 0..6 {
                            app.command(Command::ReelLarger);
                        }
                        if app.settings.reel_thumbnail_size != crate::reel::MAX_THUMBNAIL_WIDTH {
                            bail!("size commands did not reach the expanded thumbnail limit");
                        }
                    }
                    return Ok(self.case == 6);
                }
            }
            _ => bail!("invalid resize probe phase"),
        }
        self.phase += 1;
        Ok(false)
    }
}
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
    warm_uploads: u64,
    resize_probe: Option<ResizeProbe>,
}
impl Smoke {
    pub fn quick_previews_only(&self) -> bool {
        std::env::var_os("MTP_CULL_SMOKE_PREVIEWS").is_some() && (self.step <= 2 || self.step == 4)
    }
    fn tick_previews(&mut self, app: &mut App, ctx: &egui::Context) -> Result<()> {
        match self.step {
            0 => {
                if app.session.as_ref().is_none_or(|s| s.shots.len() < 3) {
                    bail!("preview smoke needs three JPEG fixtures");
                }
                app.cache.clear();
                app.canvas.reset();
                app.thumbnails.clear();
                app.thumbnail_ticks.clear();
                app.settings.group_bursts = false;
                app.settings.reel_grid = true;
                app.settings.reel_thumbnail_size = 100.0;
                app.settings.reel_width = 260.0;
                app.set_reel_position(Position::Left);
                app.selected = 1;
                app.reel_selection.single(1);
                app.pinned = Some(0);
                app.canvas.mode = Mode::SideBySide;
                app.canvas.viewport.zoom = Some(1.0);
                self.step = 1;
            }
            1 | 2 => {
                if app.canvas.previews.len() != 2
                    || app
                        .canvas
                        .previews
                        .iter()
                        .any(|(_, key)| key.edge != Some(crate::image_cache::QUICK_PREVIEW_EDGE))
                {
                    bail!(
                        "both comparison panes must draw quick pixels while sharp decodes are held"
                    );
                }
                for (b, key) in &app.canvas.previews {
                    let index = if *b { 1 } else { 0 };
                    if app.session.as_ref().unwrap().shots[index].preview()
                        != Some(key.path.as_path())
                    {
                        bail!("comparison preview belongs to the wrong photo");
                    }
                }
                if self.step == 1 {
                    self.capture(ctx, "comparison-quick-preview");
                    app.canvas.mode = Mode::Wipe;
                    self.step = 2;
                } else {
                    self.capture(ctx, "wipe-quick-preview");
                    self.step = 3;
                }
            }
            3 => {
                if app.canvas.previews.len() != 2
                    || app
                        .canvas
                        .previews
                        .iter()
                        .any(|(_, key)| key.edge.is_some())
                {
                    bail!("comparison did not upgrade to native pixels");
                }
                self.capture(ctx, "wipe-sharp-preview");
                app.settings.reel_thumbnail_size = 390.0;
                app.cache.clear();
                self.step = 4;
            }
            4 => {
                if app.reel_previews.is_empty()
                    || app
                        .reel_previews
                        .iter()
                        .any(|(_, key)| key.edge != Some(crate::image_cache::QUICK_PREVIEW_EDGE))
                {
                    bail!("resizing must keep drawing the old small reel textures");
                }
                if app.canvas.previews.len() != 2
                    || app
                        .canvas
                        .previews
                        .iter()
                        .any(|(_, key)| key.edge.is_some())
                {
                    bail!("CPU eviction must retain resident native pixels");
                }
                self.capture(ctx, "reel-cached-size-preview");
                self.step = 5;
            }
            5 => {
                if app.reel_previews.is_empty()
                    || app.reel_previews.iter().any(|(_, key)| {
                        key.edge
                            .is_none_or(|edge| edge <= crate::image_cache::QUICK_PREVIEW_EDGE)
                    })
                {
                    bail!("reel textures did not upgrade after resize");
                }
                self.capture(ctx, "reel-sharp-size-preview");
                self.step = 6;
            }
            6 => {
                std::fs::write(
                    self.root.join("PASS.txt"),
                    "PASS: quick previews in both comparison and wipe panes; upgrade to native pixels; correct photo identity; resident native pixels after CPU eviction; previous reel size during resize, followed by sharper thumbnails.\n",
                )?;
                self.step = 255;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => bail!("invalid preview smoke step"),
        }
        self.frames = 0;
        Ok(())
    }
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
            warm_uploads: 0,
            resize_probe: None,
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
        fn check_fitted_grid(app: &App) -> Result<()> {
            let columns = app.reel_layout.unwrap().0;
            let last = app
                .reel_cells
                .iter()
                .find(|(i, _)| (i + 1) % columns == 0)
                .ok_or_else(|| eyre!("grid has no complete visible row"))?
                .1;
            if (last.right() - app.reel_clip.right()).abs() > 1.0 {
                bail!(
                    "grid did not fill the viewport: {} vs {}",
                    last.right(),
                    app.reel_clip.right()
                );
            }
            if !app
                .reel_cells
                .iter()
                .any(|(i, r)| *i == app.selected && app.reel_clip.contains(r.center()))
            {
                bail!("grid did not follow the active photo after reflow");
            }
            Ok(())
        }
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
                check_fitted_grid(app)?;
                self.capture(ctx, "reel-grid-follow");
                self.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
                self.step = 40;
            }
            40 => {
                let columns = app.reel_layout.unwrap().0;
                if app.selected != 80 + columns {
                    bail!(
                        "Down did not move one grid row: {} with {columns} columns",
                        app.selected
                    );
                }
                self.key(egui::Key::ArrowUp, egui::Modifiers::NONE);
                self.step = 41;
            }
            41 => {
                if app.selected != 80
                    || !app
                        .reel_cells
                        .iter()
                        .any(|(i, r)| *i == 80 && app.reel_clip.contains(r.center()))
                {
                    bail!("Up did not return to the same column or reel did not follow");
                }
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
                app.settings_open = false;
                std::fs::write(
                    self.root.join("bottom-height.txt"),
                    app.settings.reel_height.to_string(),
                )?;
                app.set_reel_position(Position::Left);
                app.selected = 80;
                app.reel_selection.single(80);
                self.step = 42;
            }
            42 => {
                if app.reel_layout.unwrap().0 != 1 || app.reel_panel_bounds.left().abs() > 2.0 {
                    bail!("left strip did not use one column at the left edge");
                }
                self.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
                self.step = 43;
            }
            43 => {
                if app.selected != 81
                    || app.reel_viewport.min.y <= 0.0
                    || !app
                        .reel_cells
                        .iter()
                        .any(|(i, r)| *i == 81 && app.reel_clip.contains(r.center()))
                {
                    bail!(
                        "left strip did not follow Down: {} / {:?}",
                        app.selected,
                        app.reel_viewport
                    );
                }
                self.capture(ctx, "reel-left-strip");
                self.key(egui::Key::G, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
                self.step = 44;
            }
            44 => {
                if app.settings.reel_position != Position::Right
                    || (app.reel_panel_bounds.right() - ctx.content_rect().right()).abs() > 2.0
                    || !app
                        .reel_cells
                        .iter()
                        .any(|(i, r)| *i == 81 && app.reel_clip.contains(r.center()))
                {
                    bail!("placement shortcut did not move/follow into right strip");
                }
                self.capture(ctx, "reel-right-strip");
                std::fs::write(
                    self.root.join("scroll-before.txt"),
                    app.reel_viewport.min.y.to_string(),
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
                self.step = 45;
            }
            45 => {
                let before =
                    std::fs::read_to_string(self.root.join("scroll-before.txt"))?.parse::<f32>()?;
                if app.reel_viewport.min.y <= before {
                    bail!("normal wheel did not scroll side strip vertically");
                }
                std::fs::write(
                    self.root.join("width-before.txt"),
                    app.settings.reel_width.to_string(),
                )?;
                let edge = app.reel_panel_bounds.left_center();
                self.input.extend([
                    egui::Event::PointerMoved(edge),
                    egui::Event::PointerButton {
                        pos: edge,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                self.step = 46;
            }
            46 => {
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap() - egui::vec2(80.0, 0.0);
                self.input.push(egui::Event::PointerMoved(pos));
                self.step = 47;
            }
            47 => {
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap();
                self.input.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                self.step = 48;
            }
            48 => {
                let before =
                    std::fs::read_to_string(self.root.join("width-before.txt"))?.parse::<f32>()?;
                if app.settings.reel_width <= before + 20.0 {
                    bail!("side panel edge did not resize reel");
                }
                self.key(egui::Key::G, ctrl);
                self.step = 49;
            }
            49 => {
                let columns = app.reel_layout.unwrap().0;
                if !app.settings.reel_grid || columns < 2 {
                    bail!("side grid did not form multiple columns");
                }
                self.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
                self.step = 50;
            }
            50 => {
                let columns = app.reel_layout.unwrap().0;
                if app.selected != 81 + columns
                    || !app
                        .reel_cells
                        .iter()
                        .any(|(i, r)| *i == app.selected && app.reel_clip.contains(r.center()))
                {
                    bail!("side grid did not navigate/follow by column count");
                }
                check_fitted_grid(app)?;
                self.capture(ctx, "reel-right-grid");
                self.key(egui::Key::G, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
                self.step = 51;
            }
            51 => {
                let height =
                    std::fs::read_to_string(self.root.join("bottom-height.txt"))?.parse::<f32>()?;
                if app.settings.reel_position != Position::Bottom
                    || (app.settings.reel_height - height).abs() > 2.0
                {
                    bail!("returning to bottom lost its independent height");
                }
                std::fs::write(
                    self.root.join("side-width.txt"),
                    app.settings.reel_width.to_string(),
                )?;
                self.key(egui::Key::G, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
                self.step = 52;
            }
            52 => {
                let width =
                    std::fs::read_to_string(self.root.join("side-width.txt"))?.parse::<f32>()?;
                if app.settings.reel_position != Position::Left
                    || (app.settings.reel_width - width).abs() > 2.0
                {
                    bail!("left/right did not share the resized sidebar width");
                }
                check_fitted_grid(app)?;
                let rect = app.reel_size_slider;
                if !rect.is_positive() {
                    bail!("grid size slider is missing");
                }
                let pos = egui::pos2(rect.left() + 5.0, rect.center().y);
                self.input.extend([
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                self.step = 53;
            }
            53 => {
                if app.settings.reel_thumbnail_size >= 140.0 || app.reel_layout.unwrap().0 < 3 {
                    bail!(
                        "Size slider did not increase side-grid density: size {}, columns {}",
                        app.settings.reel_thumbnail_size,
                        app.reel_layout.unwrap().0
                    );
                }
                check_fitted_grid(app)?;
                self.capture(ctx, "reel-small-grid");
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap();
                self.input.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                self.step = 60;
            }
            60 => {
                for _ in 0..9 {
                    app.command(Command::ReelLarger);
                }
                self.key(
                    egui::Key::Equals,
                    egui::Modifiers::CTRL | egui::Modifiers::ALT,
                );
                self.step = 54;
            }
            54 => {
                if app.settings.reel_thumbnail_size != 300.0 || app.reel_layout.unwrap().0 != 1 {
                    bail!("thumbnail size shortcut did not reduce side grid to one column");
                }
                check_fitted_grid(app)?;
                self.capture(ctx, "reel-large-grid");
                self.key(
                    egui::Key::Minus,
                    egui::Modifiers::CTRL | egui::Modifiers::ALT,
                );
                self.step = 55;
            }
            55 => {
                if app.settings.reel_thumbnail_size != 280.0 {
                    bail!("smaller-thumbnail shortcut did not adjust the preference");
                }
                let edge = app.reel_panel_bounds.right_center();
                self.input.extend([
                    egui::Event::PointerMoved(edge),
                    egui::Event::PointerButton {
                        pos: edge,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                self.step = 56;
            }
            56 => {
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap() + egui::vec2(80.0, 0.0);
                self.input.push(egui::Event::PointerMoved(pos));
                self.step = 57;
            }
            57 => {
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap();
                self.input.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                self.step = 58;
            }
            58 => {
                if app.reel_layout.unwrap().0 < 2 {
                    bail!("resizing side grid did not reflow its columns");
                }
                check_fitted_grid(app)?;
                self.capture(ctx, "reel-grid-resized");
                self.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
                std::fs::write(
                    self.root.join("selected-before.txt"),
                    app.selected.to_string(),
                )?;
                self.step = 59;
            }
            59 => {
                let before = std::fs::read_to_string(self.root.join("selected-before.txt"))?
                    .parse::<usize>()?;
                if app.selected != before + app.reel_layout.unwrap().0 {
                    bail!("grid navigation did not use resized column count");
                }
                check_fitted_grid(app)?;
                self.resize_probe = Some(ResizeProbe::default());
                self.step = 61;
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
        if std::env::var_os("MTP_CULL_SMOKE_PREVIEWS").is_some() && self.step == 4 {
            return self.tick_previews(app, ctx);
        }
        if let Some(mut probe) = self.resize_probe.take() {
            ctx.request_repaint_after(Duration::from_millis(5));
            if probe.tick(app, &mut self.input, &self.root)? {
                std::fs::write(
                    self.root.join("PASS.txt"),
                    "PASS: 500-photo reel; native input, batch decisions, menus, wheel, full-width grids, size controls; every-frame resize anchoring in Bottom/Left/Right strip and grid; GPU low-pass readback.\n",
                )?;
                self.step = 255;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.resize_probe = Some(probe);
            }
            return Ok(());
        }
        self.frames += 1;
        ctx.request_repaint_after(Duration::from_millis(5));
        if app.loader.is_some() || app.cache.pending() || self.frames < 4 {
            return Ok(());
        }
        if std::env::var_os("MTP_CULL_SMOKE_PREVIEWS").is_some() {
            return self.tick_previews(app, ctx);
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
                if std::env::var_os("MTP_CULL_SMOKE_COLOURBLIND").is_some() {
                    app.settings.colourblind = true;
                    app.draft_settings.colourblind = true;
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
                if std::env::var_os("MTP_CULL_SMOKE_COLOURBLIND").is_some() {
                    app.selected = 2;
                    app.reel_selection.single(2);
                    app.reel_follow = true;
                }
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
                self.step = 35;
            }
            35 => {
                app.canvas.peaking = false;
                app.canvas.region = None;
                self.step = 37;
                self.frames = 0;
            }
            37 => {
                // Switching away from focus can replace both visible textures
                // and admit a deferred neighbor. Let that transition settle.
                if self.frames < 40 {
                    return Ok(());
                }
                self.warm_uploads = app.canvas.texture_uploads;
                self.step = 36;
                self.frames = 0;
            }
            36 => {
                if self.frames < 64 {
                    return Ok(());
                }
                if app.canvas.texture_uploads != self.warm_uploads {
                    bail!(
                        "idle native comparison repeatedly uploaded textures: {} -> {}",
                        self.warm_uploads,
                        app.canvas.texture_uploads
                    );
                }
                self.step = 8;
                self.frames = 0;
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
                let mut preset = app.review_import_preset();
                preset.name = "Fixture destinations".into();
                app.presets.push(preset);
                app.import_open = true;
                self.step = 38;
                self.frames = 0;
            }
            38 => {
                if !app.import_open {
                    bail!("review import dialog unexpectedly closed");
                }
                if self.frames < 40 {
                    return Ok(());
                }
                let (rect, layer) = app
                    .import_area
                    .ok_or_else(|| eyre!("review import dialog missing"))?;
                if app.import_ui_state != (true, false)
                    || ctx.layer_id_at(rect.center()) != Some(layer)
                {
                    bail!("review import dialog was invisible or obscured");
                }
                self.capture(ctx, "review-import-destinations");
                self.step = 39;
            }
            39 => {
                app.import_open = false;
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
                        "PASS: linked selections, keep/reject/undo/redo, comparison, native detail, peaking/ROI, idle native comparison without repeated uploads, review import destinations/presets, import 5 selected assets, identical retry.\nNative renderer: {}x{} points, DPI {}\nWarm UI CPU frames ({} samples): median {:.3} ms, p95 {:.3} ms, max {:.3} ms\nGPU resident {:.1} MiB; CPU resident {:.1} MiB\nPresentation latency is not measured by these CPU timings.\n",
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
