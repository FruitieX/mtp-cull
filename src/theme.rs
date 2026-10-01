use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};

pub const BACKGROUND: Color32 = Color32::from_rgb(18, 18, 20);
pub const PANEL: Color32 = Color32::from_rgb(25, 25, 28);
pub const SURFACE: Color32 = Color32::from_rgb(34, 34, 38);
pub const BORDER: Color32 = Color32::from_rgb(49, 49, 55);
pub const TEXT: Color32 = Color32::from_rgb(229, 229, 233);
pub const MUTED: Color32 = Color32::from_rgb(147, 147, 159);
pub const ACCENT: Color32 = Color32::from_rgb(156, 218, 192);
pub const ACCENT_BG: Color32 = Color32::from_rgb(43, 65, 57);
pub const SELECTED: Color32 = Color32::from_rgb(139, 185, 241);
pub const SELECTED_BG: Color32 = Color32::from_rgb(36, 47, 63);
pub const REJECT: Color32 = Color32::from_rgb(223, 151, 155);

#[derive(Clone, Copy)]
pub enum Icon {
    Keep,
    Reject,
    Clear,
    Undo,
    Folder,
    Camera,
    Pin,
    Swap,
    Zoom,
    Retry,
    Pause,
    Single,
    Compare,
    Wipe,
    Grid,
    Row,
    Focus,
}

/// Small vector icons keep buttons legible without relying on font glyph coverage.
pub fn icon(painter: &egui::Painter, center: egui::Pos2, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    let p = |x, y| center + egui::vec2(x, y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([p(a.0, a.1), p(b.0, b.1)], stroke);
    };
    let rect = |a: (f32, f32), b: (f32, f32)| {
        painter.rect_stroke(
            egui::Rect::from_min_max(p(a.0, a.1), p(b.0, b.1)),
            1.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Keep => {
            line((-5.0, 0.0), (-1.0, 4.0));
            line((-1.0, 4.0), (6.0, -4.0));
        }
        Icon::Reject => {
            line((-4.0, -4.0), (4.0, 4.0));
            line((-4.0, 4.0), (4.0, -4.0));
        }
        Icon::Clear => {
            painter.circle_stroke(center, 5.0, stroke);
        }
        Icon::Undo => {
            line((0.0, -4.0), (-5.0, 0.0));
            line((-5.0, 0.0), (0.0, 3.0));
            line((-5.0, 0.0), (3.0, 0.0));
            line((3.0, 0.0), (5.0, 2.0));
            line((5.0, 2.0), (5.0, 5.0));
        }
        Icon::Folder => {
            rect((-6.0, -3.0), (6.0, 5.0));
            line((-6.0, -3.0), (-6.0, -5.0));
            line((-6.0, -5.0), (-1.0, -5.0));
            line((-1.0, -5.0), (1.0, -3.0));
        }
        Icon::Camera => {
            rect((-6.0, -4.0), (6.0, 5.0));
            painter.circle_stroke(center, 2.8, stroke);
            line((-3.0, -6.0), (2.0, -6.0));
        }
        Icon::Pin => {
            line((-4.0, 6.0), (-4.0, -6.0));
            line((-4.0, -6.0), (5.0, -4.0));
            line((5.0, -4.0), (-4.0, 0.0));
        }
        Icon::Swap => {
            line((-6.0, -2.0), (6.0, -2.0));
            line((3.0, -5.0), (6.0, -2.0));
            line((-6.0, 2.0), (6.0, 2.0));
            line((-6.0, 2.0), (-3.0, 5.0));
        }
        Icon::Zoom => {
            painter.circle_stroke(p(-1.5, -1.5), 4.0, stroke);
            line((1.5, 1.5), (6.0, 6.0));
        }
        Icon::Retry => {
            painter.circle_stroke(center, 5.0, stroke);
            line((2.0, -5.0), (5.0, -5.0));
            line((5.0, -5.0), (5.0, -1.0));
        }
        Icon::Pause => {
            line((-2.0, -5.0), (-2.0, 5.0));
            line((2.0, -5.0), (2.0, 5.0));
        }
        Icon::Single => rect((-6.0, -5.0), (6.0, 5.0)),
        Icon::Compare | Icon::Wipe => {
            rect((-6.0, -5.0), (6.0, 5.0));
            line((0.0, -5.0), (0.0, 5.0));
            if matches!(icon, Icon::Wipe) {
                painter.rect_filled(
                    egui::Rect::from_min_max(p(-4.0, -3.0), p(-1.0, 3.0)),
                    0.0,
                    color,
                );
            }
        }
        Icon::Grid => {
            for x in [-5.0, 1.0] {
                for y in [-5.0, 1.0] {
                    rect((x, y), (x + 4.0, y + 4.0));
                }
            }
        }
        Icon::Row => {
            for x in [-6.0, -1.0, 4.0] {
                rect((x, -4.0), (x + 3.0, 4.0));
            }
        }
        Icon::Focus => {
            painter.circle_stroke(center, 4.0, stroke);
            line((-7.0, 0.0), (-2.0, 0.0));
            line((2.0, 0.0), (7.0, 0.0));
            line((0.0, -7.0), (0.0, -2.0));
            line((0.0, 2.0), (0.0, 7.0));
        }
    }
}
pub fn button_icon(ui: &egui::Ui, response: &egui::Response, glyph: Icon, color: Color32) {
    icon(
        ui.painter(),
        response.rect.left_center() + egui::vec2(ui.spacing().button_padding.x + 6.0, 0.0),
        glyph,
        color,
    );
}
pub fn icon_button(ui: &mut egui::Ui, label: &str, glyph: Icon) -> egui::Response {
    let response = ui.button(format!("     {label}"));
    button_icon(ui, &response, glyph, TEXT);
    response
}
pub fn command_icon(command: crate::review_commands::Command) -> Option<Icon> {
    use crate::review_commands::Command;
    Some(match command {
        Command::Keep | Command::BulkKeep => Icon::Keep,
        Command::Reject | Command::BulkReject => Icon::Reject,
        Command::Clear | Command::DeselectAll => Icon::Clear,
        Command::Undo => Icon::Undo,
        Command::Open => Icon::Folder,
        Command::Camera => Icon::Camera,
        Command::Pin => Icon::Pin,
        Command::Swap => Icon::Swap,
        Command::Zoom => Icon::Zoom,
        Command::Retry => Icon::Retry,
        Command::Pause => Icon::Pause,
        Command::Compare => Icon::Compare,
        Command::ReelMode | Command::SelectAll => Icon::Grid,
        Command::Peaking => Icon::Focus,
        _ => return None,
    })
}

pub fn install(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    let visuals = &mut style.visuals;
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BACKGROUND;
    visuals.faint_bg_color = SURFACE;
    visuals.weak_text_color = Some(MUTED);
    visuals.selection.bg_fill = ACCENT_BG;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.error_fg_color = REJECT;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = 12.into();
    visuals.menu_corner_radius = 8.into();
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = 6.into();
        widget.expansion = 0.0;
        widget.fg_stroke = Stroke::new(1.0, TEXT);
        widget.bg_stroke = Stroke::new(1.0, BORDER);
    }
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.inactive.bg_fill = SURFACE;
    visuals.widgets.inactive.weak_bg_fill = SURFACE;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(47, 47, 53);
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.active.bg_fill = ACCENT_BG;
    visuals.widgets.active.weak_bg_fill = ACCENT_BG;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.open.bg_fill = SURFACE;
    visuals.widgets.open.weak_bg_fill = SURFACE;
    style.spacing.item_spacing = egui::vec2(6.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.slider_width = 150.0;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(22.0));
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}

pub fn panel() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .inner_margin(egui::Margin::symmetric(14, 9))
}

pub fn group(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(BACKGROUND)
        .corner_radius(8)
        .inner_margin(3)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(content);
        });
}

pub fn tab(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    ui.add(
        egui::Button::selectable(selected, label)
            .frame_when_inactive(false)
            .min_size(egui::vec2(0.0, 28.0)),
    )
}
