//! Dimmed brand art so text stays readable.

use egui::{Color32, Pos2, Rect, Sense, TextureHandle, Ui, Vec2};

use crate::theme::{BG0, DANGER, GOLD, OK};

pub fn paint_backdrop(ui: &mut Ui, hero: Option<&TextureHandle>, _time: f64) {
    let rect = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(rect, 0.0, BG0);
    if let Some(tex) = hero {
        let size = tex.size_vec2();
        let scale = (rect.width() / size.x).max(rect.height() / size.y);
        let draw = size * scale;
        let origin = rect.center() - draw * 0.5;
        p.image(
            tex.id(),
            Rect::from_min_size(origin, draw),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::from_rgba_unmultiplied(160, 190, 210, 255),
        );
        p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(9, 14, 20, 210));
    }
}

pub fn status_dot(ui: &mut Ui, online: bool, busy: bool) {
    let (resp, painter) = ui.allocate_painter(Vec2::splat(10.0), Sense::hover());
    let c = if !online {
        DANGER
    } else if busy {
        GOLD
    } else {
        OK
    };
    painter.circle_filled(resp.rect.center(), 4.0, c);
}

pub fn hairline(ui: &mut Ui) {
    let w = ui.available_width();
    let (resp, painter) = ui.allocate_painter(Vec2::new(w, 1.0), Sense::hover());
    painter.rect_filled(resp.rect, 0.0, Color32::from_rgb(36, 50, 64));
    painter.rect_filled(
        Rect::from_min_size(resp.rect.left_top(), Vec2::new(56.0, 1.0)),
        0.0,
        GOLD,
    );
}
