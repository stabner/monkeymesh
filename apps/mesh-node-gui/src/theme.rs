//! Readable studio UI — navy, cyan, Outfit. Compact so every screen fits.

use egui::{
    epaint::CornerRadius, style::WidgetVisuals, Color32, Context, FontData, FontDefinitions,
    FontFamily, FontId, Pos2, Rect, RichText, Shadow, Stroke, TextStyle, TextureHandle, Visuals,
};
use std::sync::Arc;

pub const BG0: Color32 = Color32::from_rgb(9, 14, 20);
pub const BG1: Color32 = Color32::from_rgb(14, 20, 28);
pub const SURFACE: Color32 = Color32::from_rgb(18, 26, 36);
pub const FIELD_BG: Color32 = Color32::from_rgb(16, 24, 34);
pub const FIELD_BG_HOVER: Color32 = Color32::from_rgb(24, 34, 46);
pub const ACCENT: Color32 = Color32::from_rgb(46, 220, 240);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(20, 140, 160);
pub const CYAN: Color32 = ACCENT;
pub const CYAN_DIM: Color32 = ACCENT_DIM;
pub const TEXT_BLUE: Color32 = Color32::from_rgb(220, 234, 244);
pub const INK: Color32 = Color32::from_rgb(236, 244, 250);
pub const MUTED: Color32 = Color32::from_rgb(132, 150, 166);
pub const RULE: Color32 = Color32::from_rgb(36, 50, 64);
pub const OK: Color32 = Color32::from_rgb(72, 210, 160);
pub const DANGER: Color32 = Color32::from_rgb(255, 96, 112);
pub const WARN: Color32 = Color32::from_rgb(240, 190, 80);
pub const GOLD: Color32 = ACCENT;

const FONT_BODY: &[u8] = include_bytes!("../../../assets/fonts/Outfit-Regular.ttf");
const FONT_UI: &[u8] = include_bytes!("../../../assets/fonts/Outfit-SemiBold.ttf");
const FONT_MONO: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");

pub fn display_family() -> FontFamily {
    FontFamily::Name("mm_ui".into())
}
pub fn ui_family() -> FontFamily {
    FontFamily::Name("mm_ui".into())
}
pub fn body_family() -> FontFamily {
    FontFamily::Proportional
}
pub fn display_text(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).size(size).family(display_family())
}
pub fn body_text(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).size(size).family(body_family())
}
pub fn leaf_radius() -> CornerRadius {
    CornerRadius::same(8)
}

pub fn install(ctx: &Context) {
    install_fonts(ctx);
    apply_visuals(ctx);
}

pub fn apply_visuals(ctx: &Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = Visuals::dark();
    visuals.dark_mode = true;
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BG0;
    visuals.window_fill = BG1;
    visuals.extreme_bg_color = FIELD_BG;
    visuals.faint_bg_color = SURFACE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.inactive = WidgetVisuals {
        bg_fill: FIELD_BG,
        weak_bg_fill: FIELD_BG,
        bg_stroke: Stroke::new(1.0, RULE),
        corner_radius: leaf_radius(),
        fg_stroke: Stroke::new(1.0, INK),
        expansion: 0.0,
    };
    visuals.widgets.hovered = WidgetVisuals {
        bg_fill: FIELD_BG_HOVER,
        weak_bg_fill: FIELD_BG_HOVER,
        bg_stroke: Stroke::new(1.0, ACCENT_DIM),
        corner_radius: leaf_radius(),
        fg_stroke: Stroke::new(1.0, ACCENT),
        expansion: 0.0,
    };
    visuals.widgets.active = WidgetVisuals {
        bg_fill: ACCENT,
        weak_bg_fill: ACCENT_DIM,
        bg_stroke: Stroke::new(1.0, ACCENT),
        corner_radius: leaf_radius(),
        fg_stroke: Stroke::new(1.0, BG0),
        expansion: 0.0,
    };
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(46, 220, 240, 40);
    visuals.window_shadow = Shadow::NONE;
    visuals.window_stroke = Stroke::new(1.0, RULE);
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.text_styles.insert(TextStyle::Heading, FontId::new(20.0, display_family()));
    style.text_styles.insert(TextStyle::Body, FontId::new(13.5, body_family()));
    style.text_styles.insert(TextStyle::Button, FontId::new(13.0, ui_family()));
    style.text_styles.insert(TextStyle::Small, FontId::new(11.5, body_family()));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
    ctx.set_style(style);
}

fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("mm".into(), Arc::new(FontData::from_owned(FONT_BODY.to_vec())));
    fonts.font_data.insert("mm_ui".into(), Arc::new(FontData::from_owned(FONT_UI.to_vec())));
    fonts.font_data.insert("mm_mono".into(), Arc::new(FontData::from_owned(FONT_MONO.to_vec())));
    fonts.families.insert(ui_family(), vec!["mm_ui".into(), "mm".into()]);
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "mm".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "mm_mono".into());
    ctx.set_fonts(fonts);
}

pub fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.5)
            .family(ui_family())
            .color(MUTED)
            .extra_letter_spacing(1.1),
    );
}

#[derive(Clone, Copy)]
enum Glass {
    Solid(Color32),
    Frost,
    Danger,
}

pub fn primary_btn(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    glass_label_btn(
        ui,
        label,
        if enabled { BG0 } else { Color32::from_rgb(70, 90, 100) },
        Glass::Solid(if enabled { ACCENT } else { Color32::from_rgb(32, 48, 58) }),
        enabled,
    )
}
pub fn ghost_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ghost_btn_enabled(ui, label, true)
}
pub fn ghost_btn_enabled(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    glass_label_btn(ui, label, INK, Glass::Frost, enabled)
}
pub fn danger_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    glass_label_btn(ui, label, INK, Glass::Danger, true)
}

fn glass_label_btn(
    ui: &mut egui::Ui,
    label: &str,
    color: Color32,
    kind: Glass,
    enabled: bool,
) -> egui::Response {
    let font = FontId::new(13.0, ui_family());
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font, color));
    let size = egui::vec2((galley.size().x + 24.0).max(72.0), 32.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    paint_glass(ui.painter(), rect, kind, resp.hovered(), enabled);
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    pointer(resp)
}

pub fn pointer(resp: egui::Response) -> egui::Response {
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_glass(painter: &egui::Painter, rect: Rect, kind: Glass, hovered: bool, enabled: bool) {
    use egui::StrokeKind;
    let rounding = leaf_radius();
    let (base, sheen, rim) = match kind {
        Glass::Solid(c) => (
            c,
            Color32::from_rgba_unmultiplied(255, 255, 255, if hovered { 50 } else { 32 }),
            Color32::from_rgba_unmultiplied(255, 255, 255, 40),
        ),
        Glass::Frost => (
            Color32::from_rgba_unmultiplied(255, 255, 255, if !enabled { 8 } else if hovered { 22 } else { 14 }),
            Color32::from_rgba_unmultiplied(255, 255, 255, 20),
            Color32::from_rgba_unmultiplied(180, 220, 235, if hovered { 70 } else { 40 }),
        ),
        Glass::Danger => (
            Color32::from_rgba_unmultiplied(255, 96, 112, if hovered { 70 } else { 48 }),
            Color32::from_rgba_unmultiplied(255, 220, 220, 28),
            DANGER,
        ),
    };
    painter.rect_filled(rect, rounding, base);
    painter.rect_filled(
        Rect::from_min_size(rect.left_top(), egui::vec2(rect.width(), rect.height() * 0.42)),
        CornerRadius { nw: 8, ne: 8, sw: 2, se: 2 },
        sheen,
    );
    painter.rect_stroke(rect, rounding, Stroke::new(1.0, rim), StrokeKind::Inside);
}

pub fn tile() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(14, 22, 32, 236))
        .stroke(Stroke::new(1.0, RULE))
        .corner_radius(leaf_radius())
        .inner_margin(egui::Margin::symmetric(10, 8))
}

pub fn dual_rail(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (resp, painter) = ui.allocate_painter(egui::vec2(w, 1.0), egui::Sense::hover());
    painter.rect_filled(resp.rect, 0.0, RULE);
    painter.rect_filled(
        Rect::from_min_size(resp.rect.left_top(), egui::vec2(56.0, 1.0)),
        0.0,
        ACCENT,
    );
}

pub fn paint_resize_grip(painter: &egui::Painter, r: Rect) {
    painter.hline(r.x_range(), r.center().y, Stroke::new(1.0, RULE));
    painter.hline(
        egui::Rangef::new(r.center().x - 16.0, r.center().x + 16.0),
        r.center().y,
        Stroke::new(2.0, ACCENT),
    );
}

pub fn paint_brand_backdrop(ui: &egui::Ui, scene: Option<&TextureHandle>, _time: f64) {
    let rect = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(rect, 0.0, BG0);
    if let Some(tex) = scene {
        let size = tex.size_vec2();
        let scale = (rect.width() / size.x).max(rect.height() / size.y) * 1.02;
        let draw = size * scale;
        let origin = egui::pos2(rect.right() - draw.x * 0.70, rect.center().y - draw.y * 0.5);
        p.image(
            tex.id(),
            Rect::from_min_size(origin, draw),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::from_rgba_unmultiplied(160, 190, 210, 255),
        );
    }
    p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(9, 14, 20, 212));
}
