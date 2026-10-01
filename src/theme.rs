use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};

pub const BACKGROUND: Color32 = Color32::from_rgb(18, 18, 20);
pub const PANEL: Color32 = Color32::from_rgb(25, 25, 28);
pub const SURFACE: Color32 = Color32::from_rgb(34, 34, 38);
pub const BORDER: Color32 = Color32::from_rgb(49, 49, 55);
pub const TEXT: Color32 = Color32::from_rgb(229, 229, 233);
pub const MUTED: Color32 = Color32::from_rgb(147, 147, 159);
pub const ACCENT: Color32 = Color32::from_rgb(156, 218, 192);
pub const ACCENT_BG: Color32 = Color32::from_rgb(43, 65, 57);
pub const REJECT: Color32 = Color32::from_rgb(223, 151, 155);

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
