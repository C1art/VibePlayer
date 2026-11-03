//! Vector icons drawn with the painter, so they stay crisp at any scale without an icon font.

use eframe::egui::{Color32, Painter, Pos2, Rect, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Replay,
    StepBack,
    StepForward,
    SkipBack,
    SkipForward,
    /// Ten seconds back or forward.
    SeekBack,
    SeekForward,
    Volume,
    VolumeLow,
    Muted,
    Fullscreen,
    ExitFullscreen,
    Info,
    Scissors,
    Close,
    Folder,
    Subtitles,
    Settings,
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) / 2.0;
    let stroke = Stroke::new((s * 0.16).max(1.5), color);
    let p = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    match icon {
        Icon::Play => triangle(painter, [p(-0.45, -0.7), p(0.7, 0.0), p(-0.45, 0.7)], color),
        Icon::Pause => {
            let bar = vec2(0.32 * s, 1.4 * s);
            for x in [-0.4, 0.4] {
                painter.rect_filled(Rect::from_center_size(p(x, 0.0), bar), s * 0.08, color);
            }
        }
        Icon::Replay => {
            arc(painter, c, s * 0.65, 0.6, 5.6, stroke);
            triangle(
                painter,
                [p(0.25, -0.95), p(0.75, -0.55), p(0.2, -0.3)],
                color,
            );
        }
        Icon::StepBack | Icon::StepForward => {
            let flip = if icon == Icon::StepBack { -1.0 } else { 1.0 };
            triangle(
                painter,
                [
                    p(-0.5 * flip, -0.6),
                    p(0.35 * flip, 0.0),
                    p(-0.5 * flip, 0.6),
                ],
                color,
            );
            painter.line_segment([p(0.55 * flip, -0.6), p(0.55 * flip, 0.6)], stroke);
        }
        Icon::SkipBack | Icon::SkipForward => {
            let flip = if icon == Icon::SkipBack { -1.0 } else { 1.0 };
            for offset in [-0.55, 0.15] {
                triangle(
                    painter,
                    [
                        p((offset - 0.1) * flip, -0.55),
                        p((offset + 0.6) * flip, 0.0),
                        p((offset - 0.1) * flip, 0.55),
                    ],
                    color,
                );
            }
        }
        Icon::SeekBack | Icon::SeekForward => {
            // A circular arrow around the number of seconds, open at the top on the side the
            // arrow points away from.
            use std::f32::consts::{FRAC_PI_2, PI};
            let flip = if icon == Icon::SeekBack { 1.0 } else { -1.0 };
            let (from, to) = if icon == Icon::SeekBack {
                (-FRAC_PI_2, PI)
            } else {
                (0.0, 1.5 * PI)
            };
            arc(painter, c, s * 0.85, from, to, stroke);
            triangle(
                painter,
                [
                    p(-0.42 * flip, -0.85),
                    p(0.05 * flip, -1.2),
                    p(0.05 * flip, -0.5),
                ],
                color,
            );
            painter.text(
                p(0.0, 0.06),
                eframe::egui::Align2::CENTER_CENTER,
                "10",
                eframe::egui::FontId::proportional(s * 0.85),
                color,
            );
        }
        Icon::Volume | Icon::VolumeLow | Icon::Muted => {
            speaker(painter, &p, color);
            match icon {
                Icon::Muted => {
                    painter.line_segment([p(0.3, -0.35), p(0.9, 0.35)], stroke);
                    painter.line_segment([p(0.3, 0.35), p(0.9, -0.35)], stroke);
                }
                Icon::VolumeLow => arc(painter, p(0.05, 0.0), s * 0.45, -0.8, 0.8, stroke),
                _ => {
                    arc(painter, p(0.05, 0.0), s * 0.45, -0.8, 0.8, stroke);
                    arc(painter, p(0.05, 0.0), s * 0.85, -0.85, 0.85, stroke);
                }
            }
        }
        Icon::Fullscreen | Icon::ExitFullscreen => {
            // Corner brackets pointing outwards, or inwards when exiting.
            let (at, reach) = if icon == Icon::Fullscreen {
                (0.75, -0.45)
            } else {
                (0.3, 0.45)
            };
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = p(sx * at, sy * at);
                painter.line_segment([corner, p(sx * (at + reach), sy * at)], stroke);
                painter.line_segment([corner, p(sx * at, sy * (at + reach))], stroke);
            }
        }
        Icon::Info => {
            painter.circle_stroke(c, s * 0.8, stroke);
            painter.line_segment([p(0.0, -0.05), p(0.0, 0.45)], stroke);
            painter.circle_filled(p(0.0, -0.38), s * 0.11, color);
        }
        Icon::Scissors => {
            painter.circle_stroke(p(-0.45, 0.5), s * 0.25, stroke);
            painter.circle_stroke(p(0.45, 0.5), s * 0.25, stroke);
            painter.line_segment([p(-0.3, 0.3), p(0.45, -0.75)], stroke);
            painter.line_segment([p(0.3, 0.3), p(-0.45, -0.75)], stroke);
        }
        Icon::Close => {
            painter.line_segment([p(-0.55, -0.55), p(0.55, 0.55)], stroke);
            painter.line_segment([p(-0.55, 0.55), p(0.55, -0.55)], stroke);
        }
        Icon::Folder => {
            let points = vec![
                p(-0.8, -0.55),
                p(-0.25, -0.55),
                p(-0.1, -0.35),
                p(0.8, -0.35),
                p(0.8, 0.6),
                p(-0.8, 0.6),
                p(-0.8, -0.55),
            ];
            painter.add(eframe::egui::Shape::line(points, stroke));
        }
        Icon::Subtitles => {
            painter.rect_stroke(
                Rect::from_center_size(c, vec2(1.6 * s, 1.2 * s)),
                s * 0.15,
                stroke,
                eframe::egui::StrokeKind::Middle,
            );
            painter.line_segment([p(-0.5, 0.2), p(0.1, 0.2)], stroke);
            painter.line_segment([p(0.25, 0.2), p(0.5, 0.2)], stroke);
        }
        Icon::Settings => {
            for tooth in 0..8 {
                let angle = tooth as f32 * std::f32::consts::FRAC_PI_4;
                let (sin, cos) = angle.sin_cos();
                painter.line_segment(
                    [p(cos * 0.55, sin * 0.55), p(cos * 0.85, sin * 0.85)],
                    Stroke::new(stroke.width * 1.6, color),
                );
            }
            painter.circle_stroke(c, s * 0.55, stroke);
            painter.circle_filled(c, s * 0.18, color);
        }
    }
}

fn triangle(painter: &Painter, points: [Pos2; 3], color: Color32) {
    painter.add(eframe::egui::Shape::convex_polygon(
        points.to_vec(),
        color,
        Stroke::NONE,
    ));
}

fn speaker(painter: &Painter, p: &impl Fn(f32, f32) -> Pos2, color: Color32) {
    let body = vec![
        p(-0.85, -0.25),
        p(-0.5, -0.25),
        p(-0.05, -0.65),
        p(-0.05, 0.65),
        p(-0.5, 0.25),
        p(-0.85, 0.25),
    ];
    painter.add(eframe::egui::Shape::convex_polygon(
        body,
        color,
        Stroke::NONE,
    ));
}

/// An arc of radius `radius` from angle `from` to `to` (radians, 0 = right, clockwise).
fn arc(painter: &Painter, center: Pos2, radius: f32, from: f32, to: f32, stroke: Stroke) {
    let steps = 16;
    let points = (0..=steps)
        .map(|i| {
            let angle = from + (to - from) * i as f32 / steps as f32;
            pos2(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect();
    painter.add(eframe::egui::Shape::line(points, stroke));
}
