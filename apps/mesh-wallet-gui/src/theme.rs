//! Readable studio UI — navy, cyan, Outfit. Compact so every screen fits.

use egui::{
    epaint::CornerRadius, style::WidgetVisuals, Color32, Context, FontData, FontDefinitions,
    FontFamily, FontId, Margin, RichText, Shadow, Stroke, TextStyle, Visuals,
};
use std::sync::Arc;

pub const BG0: Color32 = Color32::from_rgb(9, 14, 20);
pub const BG1: Color32 = Color32::from_rgb(14, 20, 28);
pub const SIDE: Color32 = Color32::from_rgb(10, 16, 22);
pub const PANEL: Color32 = Color32::from_rgb(18, 26, 36);
pub const ACCENT: Color32 = Color32::from_rgb(46, 220, 240);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(20, 140, 160);
pub const CYAN: Color32 = ACCENT;
pub const CYAN_DIM: Color32 = ACCENT_DIM;
pub const GOLD: Color32 = ACCENT;
pub const MINT: Color32 = Color32::from_rgb(72, 210, 160);
pub const INK: Color32 = Color32::from_rgb(236, 244, 250);
pub const MUTED: Color32 = Color32::from_rgb(132, 150, 166);
pub const OK: Color32 = Color32::from_rgb(72, 210, 160);
pub const WARN: Color32 = Color32::from_rgb(240, 190, 80);
pub const DANGER: Color32 = Color32::from_rgb(255, 96, 112);
pub const RULE: Color32 = Color32::from_rgb(36, 50, 64);

pub fn display_family() -> FontFamily {
    FontFamily::Name("mm_ui".into())
}

pub fn ui_family() -> FontFamily {
    FontFamily::Name("mm_ui".into())
}

pub fn leaf_radius() -> CornerRadius {
    CornerRadius::same(8)
}

pub fn install(ctx: &Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    let mut visuals = Visuals::dark();
    visuals.dark_mode = true;
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BG0;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BG1;
    visuals.faint_bg_color = BG1;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.inactive = WidgetVisuals {
        bg_fill: Color32::from_rgb(22, 32, 42),
        weak_bg_fill: Color32::from_rgb(18, 26, 36),
        bg_stroke: Stroke::new(1.0, RULE),
        corner_radius: leaf_radius(),
        fg_stroke: Stroke::new(1.0, INK),
        expansion: 0.0,
    };
    visuals.widgets.hovered = WidgetVisuals {
        bg_fill: Color32::from_rgb(28, 40, 52),
        weak_bg_fill: Color32::from_rgb(24, 34, 46),
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
    style.text_styles.insert(TextStyle::Heading, FontId::new(20.0, ui_family()));
    style.text_styles.insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(13.0, ui_family()));
    style.text_styles.insert(TextStyle::Small, FontId::new(11.5, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
    ctx.set_style(style);
}

fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "mm".into(),
        Arc::new(FontData::from_owned(
            include_bytes!("../../../assets/fonts/Outfit-Regular.ttf").to_vec(),
        )),
    );
    fonts.font_data.insert(
        "mm_ui".into(),
        Arc::new(FontData::from_owned(
            include_bytes!("../../../assets/fonts/Outfit-SemiBold.ttf").to_vec(),
        )),
    );
    fonts.font_data.insert(
        "mm_mono".into(),
        Arc::new(FontData::from_owned(
            include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf").to_vec(),
        )),
    );
    fonts.families.insert(ui_family(), vec!["mm_ui".into(), "mm".into()]);
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "mm".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "mm_mono".into());
    ctx.set_fonts(fonts);
}

pub fn panel() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(14, 22, 32, 236))
        .stroke(Stroke::new(1.0, RULE))
        .corner_radius(leaf_radius())
        .inner_margin(Margin::symmetric(14, 12))
}

pub fn label_upper(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .color(MUTED)
            .size(10.5)
            .family(ui_family())
            .extra_letter_spacing(1.1),
    );
}

#[derive(Clone, Copy)]
enum Glass {
    Solid(Color32),
    Frost,
}

pub fn primary_btn(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    glass_label_btn(
        ui,
        label,
        if enabled { Color32::from_rgb(6, 16, 20) } else { Color32::from_rgb(70, 90, 100) },
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

fn paint_glass(painter: &egui::Painter, rect: egui::Rect, kind: Glass, hovered: bool, enabled: bool) {
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
    };
    painter.rect_filled(rect, rounding, base);
    painter.rect_filled(
        egui::Rect::from_min_size(rect.left_top(), egui::vec2(rect.width(), rect.height() * 0.42)),
        CornerRadius { nw: 8, ne: 8, sw: 2, se: 2 },
        sheen,
    );
    painter.rect_stroke(rect, rounding, Stroke::new(1.0, rim), StrokeKind::Inside);
}

pub fn paint_glass_overlay(painter: &egui::Painter, rect: egui::Rect, selected: bool, hovered: bool) {
    use egui::StrokeKind;
    if selected || hovered {
        painter.rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(rect.width(), 10.0)),
            CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 },
            Color32::from_rgba_unmultiplied(255, 255, 255, 16),
        );
        painter.rect_stroke(
            rect,
            leaf_radius(),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(46, 220, 240, if selected { 90 } else { 40 })),
            StrokeKind::Inside,
        );
    }
}
