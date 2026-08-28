//! Quiet geometric icons (no font glyphs — those fail on many Windows installs).

use egui::{Color32, Pos2, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::theme::{pointer, CYAN, INK, MUTED};

#[derive(Clone, Copy)]
pub enum Icon {
    Overview,
    Send,
    Receive,
    History,
    Mine,
    Network,
    Sync,
    Copy,
}

pub fn icon(ui: &mut Ui, kind: Icon, size: f32, color: Color32) {
    let (resp, painter) = ui.allocate_painter(Vec2::splat(size), Sense::hover());
    let r = resp.rect;
    let c = r.center();
    let s = size * 0.38;
    let stroke = Stroke::new((size * 0.08).clamp(1.2, 2.2), color);

    match kind {
        Icon::Overview => {
            // hexagon
            let pts = hex_points(c, s);
            for i in 0..6 {
                painter.line_segment([pts[i], pts[(i + 1) % 6]], stroke);
            }
            painter.circle_filled(c, size * 0.08, color);
        }
        Icon::Send => {
            // arrow right
            painter.line_segment([c + Vec2::new(-s, 0.0), c + Vec2::new(s * 0.55, 0.0)], stroke);
            painter.line_segment([c + Vec2::new(s * 0.15, -s * 0.55), c + Vec2::new(s * 0.7, 0.0)], stroke);
            painter.line_segment([c + Vec2::new(s * 0.15, s * 0.55), c + Vec2::new(s * 0.7, 0.0)], stroke);
        }
        Icon::Receive => {
            // arrow down into tray
            painter.line_segment([c + Vec2::new(0.0, -s), c + Vec2::new(0.0, s * 0.25)], stroke);
            painter.line_segment([c + Vec2::new(-s * 0.45, -s * 0.2), c + Vec2::new(0.0, s * 0.25)], stroke);
            painter.line_segment([c + Vec2::new(s * 0.45, -s * 0.2), c + Vec2::new(0.0, s * 0.25)], stroke);
            painter.line_segment(
                [c + Vec2::new(-s * 0.7, s * 0.55), c + Vec2::new(s * 0.7, s * 0.55)],
                stroke,
            );
            painter.line_segment(
                [c + Vec2::new(-s * 0.7, s * 0.55), c + Vec2::new(-s * 0.7, s * 0.25)],
                stroke,
            );
            painter.line_segment(
                [c + Vec2::new(s * 0.7, s * 0.55), c + Vec2::new(s * 0.7, s * 0.25)],
                stroke,
            );
        }
        Icon::History => {
            // stacked lines
            for i in 0..3 {
                let y = -s + i as f32 * (s * 0.85);
                painter.line_segment(
                    [c + Vec2::new(-s * 0.75, y), c + Vec2::new(s * 0.75, y)],
                    stroke,
                );
                painter.circle_filled(c + Vec2::new(-s * 0.95, y), size * 0.05, color);
            }
        }
        Icon::Mine => {
            // pickaxe-ish diamond
            let pts = [
                c + Vec2::new(0.0, -s),
                c + Vec2::new(s * 0.7, 0.0),
                c + Vec2::new(0.0, s),
                c + Vec2::new(-s * 0.7, 0.0),
            ];
            for i in 0..4 {
                painter.line_segment([pts[i], pts[(i + 1) % 4]], stroke);
            }
            painter.line_segment([c + Vec2::new(0.0, -s * 0.2), c + Vec2::new(0.0, s * 0.85)], stroke);
        }
        Icon::Network => {
            // three nodes
            let a = c + Vec2::new(-s * 0.7, s * 0.35);
            let b = c + Vec2::new(s * 0.7, s * 0.35);
            let top = c + Vec2::new(0.0, -s * 0.55);
            painter.line_segment([a, top], stroke);
            painter.line_segment([b, top], stroke);
            painter.line_segment([a, b], stroke);
            for p in [a, b, top] {
                painter.circle_filled(p, size * 0.1, color);
            }
        }
        Icon::Sync => {
            // circular arrows (simple arc approximation)
            painter.circle_stroke(c, s * 0.85, stroke);
            painter.line_segment(
                [c + Vec2::new(s * 0.85, -s * 0.15), c + Vec2::new(s * 0.45, -s * 0.55)],
                stroke,
            );
            painter.line_segment(
                [c + Vec2::new(-s * 0.85, s * 0.15), c + Vec2::new(-s * 0.45, s * 0.55)],
                stroke,
            );
        }
        Icon::Copy => {
            let o = size * 0.12;
            painter.rect_stroke(
                egui::Rect::from_center_size(c + Vec2::new(-o, -o), Vec2::splat(s * 1.2)),
                2.0,
                stroke,
                StrokeKind::Middle,
            );
            painter.rect_stroke(
                egui::Rect::from_center_size(c + Vec2::new(o, o), Vec2::splat(s * 1.2)),
                2.0,
                stroke,
                StrokeKind::Middle,
            );
        }
    }
}

fn hex_points(c: Pos2, r: f32) -> [Pos2; 6] {
    let mut pts = [Pos2::ZERO; 6];
    for i in 0..6 {
        let a = std::f32::consts::TAU * (i as f32) / 6.0 - std::f32::consts::FRAC_PI_2;
        pts[i] = c + Vec2::angled(a) * r;
    }
    pts
}

pub fn icon_btn(ui: &mut Ui, kind: Icon, label: &str, selected: bool) -> egui::Response {
    let fill = if selected {
        Color32::from_rgba_unmultiplied(46, 220, 240, 28)
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if selected {
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(46, 220, 240, 120))
    } else {
        Stroke::NONE
    };
    let color = if selected { CYAN } else { MUTED };

    let resp = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(crate::theme::leaf_radius())
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icon(ui, kind, 18.0, color);
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(label)
                        .color(if selected { INK } else { MUTED })
                        .size(13.5),
                );
            });
        })
        .response
        .interact(Sense::click());

    crate::theme::paint_glass_overlay(ui.painter(), resp.rect, selected, resp.hovered());
    pointer(resp)
}
