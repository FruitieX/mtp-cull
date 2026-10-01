use crate::image_cache::{ImageCache, Key, Picture};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Mode {
    #[default]
    Single,
    SideBySide,
    Wipe,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Single => "Single",
            Self::SideBySide => "Side by side",
            Self::Wipe => "Vertical wipe",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Single => Self::SideBySide,
            Self::SideBySide => Self::Wipe,
            Self::Wipe => Self::Single,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Viewport {
    pub center: Vec2,
    pub zoom: Option<f32>,
    pub alignment: Vec2,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            center: egui::vec2(0.5, 0.5),
            zoom: None,
            alignment: Vec2::ZERO,
        }
    }
}
impl Viewport {
    pub fn swap(&mut self, source: Vec2) {
        self.center += self.alignment / source;
        self.alignment = -self.alignment;
    }
    pub fn scale(&self, rect: Rect, source: Vec2, dpi: f32) -> f32 {
        self.zoom.map_or_else(
            || (rect.width() / source.x).min(rect.height() / source.y),
            |zoom| zoom / dpi,
        )
    }
    pub fn image_rect(&self, rect: Rect, source: Vec2, dpi: f32, b: bool) -> Rect {
        let scale = self.scale(rect, source, dpi);
        let center = self.center * source + if b { self.alignment } else { Vec2::ZERO };
        Rect::from_min_size(rect.center() - center * scale, source * scale)
    }
    pub fn zoom_at(
        &mut self,
        rect: Rect,
        source: Vec2,
        dpi: f32,
        pointer: Pos2,
        factor: f32,
        b: bool,
    ) {
        let before = self.image_rect(rect, source, dpi, b);
        let point = (pointer - before.min) / self.scale(rect, source, dpi);
        let scale = (self.scale(rect, source, dpi) * dpi * factor).clamp(0.02, 16.0);
        self.zoom = Some(scale);
        self.center = (point
            - (pointer - rect.center()) / (scale / dpi)
            - if b { self.alignment } else { Vec2::ZERO })
            / source;
    }
}
struct Texture {
    handle: egui::TextureHandle,
    bytes: usize,
    tick: u64,
}
struct Overlay {
    handle: egui::TextureHandle,
    threshold: u8,
    opacity: u8,
}
pub struct Canvas {
    pub mode: Mode,
    pub viewport: Viewport,
    pub divider: f32,
    pub active_a: bool,
    pub peaking: bool,
    pub threshold: u8,
    pub opacity: u8,
    pub region: Option<Rect>,
    pub region_tool: bool,
    pub center_region_on_load: bool,
    textures: HashMap<Key, Texture>,
    overlays: HashMap<Key, Overlay>,
    tick: u64,
    budget: usize,
    drag_region_start: Option<Pos2>,
    uploads_left: usize,
    upload_bytes_left: usize,
    uploaded: bool,
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            mode: Mode::Single,
            viewport: Viewport::default(),
            divider: 0.5,
            active_a: false,
            peaking: false,
            threshold: 18,
            opacity: 160,
            region: None,
            region_tool: false,
            center_region_on_load: false,
            textures: HashMap::new(),
            overlays: HashMap::new(),
            tick: 0,
            budget: 512 * 1024 * 1024,
            drag_region_start: None,
            uploads_left: 0,
            upload_bytes_left: 0,
            uploaded: false,
        }
    }
}
pub struct ViewImage {
    pub name: String,
    pub status_color: Color32,
    pub status_label: &'static str,
    pub key: Option<Key>,
    pub fallback: Option<Key>,
}
impl Canvas {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes.max(64 * 1024 * 1024);
    }
    pub fn bytes(&self) -> usize {
        self.textures.values().map(|t| t.bytes).sum::<usize>()
            + self
                .overlays
                .values()
                .map(|t| t.handle.size()[0] * t.handle.size()[1] * 4)
                .sum::<usize>()
    }
    pub fn native(&self) -> bool {
        self.viewport.zoom.is_some()
            || self.peaking
            || self.region_tool
            || self.region.is_some()
            || self.center_region_on_load
    }
    pub fn center_region(&mut self, source_size: [usize; 2]) {
        self.region = Some(Rect::from_center_size(
            Pos2::ZERO
                + self.viewport.center * egui::vec2(source_size[0] as f32, source_size[1] as f32),
            egui::vec2(300.0, 300.0),
        ));
        self.center_region_on_load = false;
    }
    pub fn needs_focus(&self) -> bool {
        self.peaking || self.region_tool || self.region.is_some()
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut ImageCache,
        a: Option<ViewImage>,
        b: ViewImage,
        blink: bool,
    ) {
        self.uploads_left = 2;
        self.upload_bytes_left = 32 * 1024 * 1024;
        self.uploaded = false;
        let size = ui.available_size().max(egui::vec2(1.0, 1.0));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(15, 17, 20));
        let mut pins = HashSet::new();
        for image in [&b].into_iter().chain(a.iter()) {
            if let Some(key) = &image.key {
                pins.insert(key.clone());
            }
            if let Some(key) = &image.fallback {
                pins.insert(key.clone());
            }
        }
        let active_a = self.active_a;
        match (self.mode, a.as_ref()) {
            (Mode::SideBySide, Some(a)) => {
                let mid = rect.center().x;
                let left = Rect::from_min_max(rect.min, egui::pos2(mid - 3.0, rect.max.y));
                let right = Rect::from_min_max(egui::pos2(mid + 3.0, rect.min.y), rect.max);
                self.pane(ui, left, left, a, cache, false);
                if blink {
                    self.pane(ui, right, right, a, cache, false);
                } else {
                    self.pane(ui, right, right, &b, cache, true);
                }
            }
            (Mode::Wipe, Some(a)) if blink => self.pane(ui, rect, rect, a, cache, false),
            (Mode::Wipe, Some(a)) => {
                let split = rect.left() + rect.width() * self.divider;
                self.pane(
                    ui,
                    rect,
                    Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom())),
                    a,
                    cache,
                    false,
                );
                self.pane(
                    ui,
                    rect,
                    Rect::from_min_max(egui::pos2(split, rect.top()), rect.max),
                    &b,
                    cache,
                    true,
                );
                let handle = Rect::from_min_max(
                    egui::pos2(split - 8.0, rect.top()),
                    egui::pos2(split + 8.0, rect.bottom()),
                );
                let response = ui.interact(handle, ui.id().with("wipe-handle"), Sense::drag());
                if response.dragged()
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    self.divider = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                }
                response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                ui.painter().line_segment(
                    [
                        egui::pos2(split, rect.top()),
                        egui::pos2(split, rect.bottom()),
                    ],
                    egui::Stroke::new(2.0, Color32::WHITE),
                );
                ui.painter()
                    .circle_filled(egui::pos2(split, rect.center().y), 7.0, Color32::WHITE);
            }
            _ => {
                if blink && let Some(a) = a.as_ref() {
                    self.pane(ui, rect, rect, a, cache, false);
                } else {
                    self.pane(ui, rect, rect, &b, cache, true);
                }
            }
        }
        if blink {
            self.active_a = active_a;
        }
        while self.bytes() > self.budget {
            let victim = self
                .textures
                .iter()
                .filter(|(key, _)| !pins.contains(*key))
                .min_by_key(|(_, t)| t.tick)
                .map(|(key, _)| key.clone());
            if let Some(key) = victim {
                self.textures.remove(&key);
                self.overlays.remove(&key);
            } else {
                break;
            }
        }
    }
    fn pane(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        clip: Rect,
        image: &ViewImage,
        cache: &mut ImageCache,
        b: bool,
    ) {
        let label = if b { "B" } else { "A" };
        let painter = ui.painter().with_clip_rect(clip);
        let status_border = |bounds: Rect| {
            if bounds.is_positive() {
                painter.rect_stroke(
                    bounds,
                    0.0,
                    egui::Stroke::new(3.0, image.status_color),
                    egui::StrokeKind::Inside,
                );
            }
        };
        let Some(key) = &image.key else {
            status_border(clip);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for JPEG companion",
                egui::FontId::proportional(18.0),
                Color32::GRAY,
            );
            return;
        };
        let picture = cache
            .get(key)
            .map(|p| (key.clone(), p, true))
            .or_else(|| {
                key.analyze
                    .then(|| Key::native(key.path.clone()))
                    .and_then(|k| cache.get(&k).map(|p| (k, p, false)))
            })
            .or_else(|| {
                image
                    .fallback
                    .as_ref()
                    .and_then(|k| cache.get(k).map(|p| (k.clone(), p, false)))
            });
        let Some((loaded, picture, native_ready)) = picture else {
            status_border(clip);
            let text = cache.failure(key).unwrap_or("Loading preview…");
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(16.0),
                Color32::GRAY,
            );
            return;
        };
        let source = egui::vec2(picture.source_size[0] as f32, picture.source_size[1] as f32);
        if b && self.center_region_on_load {
            self.center_region(picture.source_size);
        }
        let dpi = ui.ctx().pixels_per_point();
        let response = ui.interact(
            clip,
            ui.id()
                .with(("canvas-pane", b, rect.min.x.to_bits(), rect.min.y.to_bits())),
            Sense::click_and_drag(),
        );
        if response.clicked() {
            self.active_a = !b;
        }
        if response.double_clicked() {
            self.viewport.zoom = if self.viewport.zoom.is_some() {
                None
            } else {
                Some(1.0)
            };
        }
        if response.hovered()
            && let Some(pointer) = response.hover_pos()
        {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let zoom = ui.input(|i| i.zoom_delta());
            let factor = zoom * (scroll * 0.002).exp();
            if (factor - 1.0).abs() > 0.0001 {
                self.viewport.zoom_at(rect, source, dpi, pointer, factor, b);
            }
        }
        let image_rect = self.viewport.image_rect(rect, source, dpi, b);
        let scale = self.viewport.scale(rect, source, dpi);
        let alignment = if b {
            self.viewport.alignment
        } else {
            Vec2::ZERO
        };
        let to_source =
            |point: Pos2| egui::Pos2::ZERO + (point - image_rect.min) / scale - alignment;
        if response.drag_started()
            && self.region_tool
            && let Some(point) = response.interact_pointer_pos()
        {
            self.drag_region_start = Some(to_source(point));
        }
        if response.dragged() {
            if self.region_tool {
                if let (Some(start), Some(point)) =
                    (self.drag_region_start, response.interact_pointer_pos())
                {
                    let candidate = Rect::from_two_pos(start, to_source(point));
                    self.region =
                        Some(candidate.intersect(Rect::from_min_size(Pos2::ZERO, source)));
                }
            } else if b && ui.input(|i| i.modifiers.alt) {
                self.viewport.alignment -= response.drag_delta() / scale;
            } else {
                self.viewport.center -= response.drag_delta() / scale / source;
            }
        }
        if response.drag_stopped() {
            self.drag_region_start = None;
        }
        let image_rect = self.viewport.image_rect(rect, source, dpi, b);
        if let Some(texture) = self.texture(ui.ctx(), &loaded, &picture) {
            painter.image(
                texture,
                image_rect,
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if self.peaking
            && native_ready
            && let Some(focus) = &picture.focus
        {
            let refresh = self
                .overlays
                .get(&loaded)
                .is_none_or(|o| o.threshold != self.threshold || o.opacity != self.opacity);
            if refresh && self.uploads_left > 0 {
                self.uploads_left -= 1;
                let handle = ui.ctx().load_texture(
                    format!("focus:{}", loaded.path.display()),
                    focus.overlay(self.threshold, self.opacity),
                    egui::TextureOptions::LINEAR,
                );
                self.overlays.insert(
                    loaded.clone(),
                    Overlay {
                        handle,
                        threshold: self.threshold,
                        opacity: self.opacity,
                    },
                );
            }
            if let Some(overlay) = self.overlays.get(&loaded) {
                painter.image(
                    overlay.handle.id(),
                    image_rect,
                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        let mut text = format!("{label} · {} · {}", image.name, image.status_label);
        if self.native() {
            text.push_str(if native_ready {
                " · native detail"
            } else {
                " · native detail loading"
            });
        }
        if let Some(region) = self.region {
            let region = region.translate(alignment);
            let displayed = Rect::from_min_max(
                image_rect.min + region.min.to_vec2() * scale,
                image_rect.min + region.max.to_vec2() * scale,
            );
            painter.rect_stroke(
                displayed,
                0.0,
                egui::Stroke::new(1.0, Color32::YELLOW),
                egui::StrokeKind::Inside,
            );
            if !Rect::from_min_size(Pos2::ZERO, source).contains_rect(region) {
                text.push_str(" · region outside image");
            } else if native_ready && let Some(focus) = &picture.focus {
                text.push_str(&format!(" · region {:.1}", focus.score(region)));
            }
        }
        let text_rect = Rect::from_min_size(clip.min, egui::vec2(clip.width(), 27.0));
        painter.rect_filled(text_rect, 0.0, Color32::from_black_alpha(190));
        painter.text(
            text_rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(14.0),
            if self.active_a != b {
                Color32::LIGHT_BLUE
            } else {
                Color32::WHITE
            },
        );
        // Outline the visible photo, so its state stays visible at any zoom.
        // Wipe panes each outline their own clipped portion in the same color
        // as their filmstrip card, including while holding A/B blink.
        status_border(image_rect.intersect(clip));
    }
    fn texture(
        &mut self,
        ctx: &egui::Context,
        key: &Key,
        picture: &Arc<Picture>,
    ) -> Option<egui::TextureId> {
        self.tick += 1;
        if let Some(texture) = self.textures.get_mut(key) {
            texture.tick = self.tick;
            return Some(texture.handle.id());
        }
        let bytes = picture.image.pixels.len() * 4;
        // A native image may exceed the per-frame byte allowance. Allow one
        // oversized upload to make progress, then defer the other pane.
        if self.uploads_left == 0 || (self.uploaded && bytes > self.upload_bytes_left) {
            ctx.request_repaint();
            return None;
        }
        self.uploads_left -= 1;
        self.upload_bytes_left = self.upload_bytes_left.saturating_sub(bytes);
        self.uploaded = true;
        let handle = ctx.load_texture(
            format!("image:{}:{:?}", key.path.display(), key.edge),
            picture.image.clone(),
            egui::TextureOptions::LINEAR,
        );
        let id = handle.id();
        self.textures.insert(
            key.clone(),
            Texture {
                handle,
                bytes: picture.image.pixels.len() * 4,
                tick: self.tick,
            },
        );
        Some(id)
    }
    pub fn prefetch(
        &mut self,
        ctx: &egui::Context,
        cache: &mut ImageCache,
        keys: impl Iterator<Item = Key>,
    ) {
        for key in keys.take(2) {
            if self.bytes() >= self.budget || self.uploads_left == 0 {
                break;
            }
            if let Some(picture) = cache.get(&key) {
                let _ = self.texture(ctx, &key, &picture);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_view_maps_wipe_blink_pan_and_alignment_at_same_pixel_scale() {
        let pane = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0));
        let source = egui::vec2(7728.0, 5152.0);
        let mut view = Viewport {
            zoom: Some(1.0),
            alignment: egui::vec2(8.0, -4.0),
            ..Viewport::default()
        };
        let a = view.image_rect(pane, source, 1.25, false);
        let b = view.image_rect(pane, source, 1.25, true);
        assert_eq!(a.size(), b.size());
        assert!((a.min - b.min - view.alignment / 1.25).length() < 0.01);
        let center = view.center;
        view.center += egui::vec2(0.05, -0.02);
        let moved_a = view.image_rect(pane, source, 1.25, false);
        let moved_b = view.image_rect(pane, source, 1.25, true);
        assert!((moved_a.min - a.min - (moved_b.min - b.min)).length() < 0.01);
        assert!((moved_a.min - a.min + (view.center - center) * source / 1.25).length() < 0.01);
        view.swap(source);
        assert!((view.image_rect(pane, source, 1.25, false).min - moved_b.min).length() < 0.01);
        assert!((view.image_rect(pane, source, 1.25, true).min - moved_a.min).length() < 0.01);
    }
    #[test]
    fn native_scale_accounts_for_dpi_and_zoom_keeps_cursor_point() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let source = egui::vec2(4000.0, 3000.0);
        let mut view = Viewport {
            zoom: Some(1.0),
            ..Viewport::default()
        };
        assert_eq!(view.scale(rect, source, 2.0), 0.5);
        let pointer = egui::pos2(210.0, 123.0);
        let before = (pointer - view.image_rect(rect, source, 2.0, false).min)
            / view.scale(rect, source, 2.0);
        view.zoom_at(rect, source, 2.0, pointer, 1.7, false);
        let after = (pointer - view.image_rect(rect, source, 2.0, false).min)
            / view.scale(rect, source, 2.0);
        assert!((before - after).length() < 0.01);
    }
}
