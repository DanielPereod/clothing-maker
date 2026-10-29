//! Iconos vectoriales de la interfaz, dibujados con trazos para que se vean
//! nítidos a cualquier escala y no dependan de la fuente.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    New,
    Open,
    Save,
    Undo,
    Redo,
    Select,
    Pen,
    Rect,
    Seam,
    Duplicate,
    Delete,
    Fit,
    Grid,
    Mesh,
    Play,
    Pause,
    Restart,
    Live,
    Home,
    Settings,
    Pattern,
    Warning,
    Mannequin,
    ZoomIn,
    ZoomOut,
    Layout2D,
    LayoutSplit,
    Layout3D,
    Allowance,
    Flip,
    Order,
    Highlight,
}

/// Dibuja `icon` dentro de `rect` (se usa el cuadrado central).
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let size = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(size, size));
    // Coordenadas normalizadas: (0, 0) arriba a la izquierda, (1, 1) abajo a la derecha.
    let p = |x: f32, y: f32| pos2(r.left() + x * size, r.top() + y * size);
    let w = (size / 14.0).clamp(1.2, 2.0);
    let stroke = Stroke::new(w, color);
    let thin = Stroke::new(w * 0.7, color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
    };
    let closed = |pts: &[(f32, f32)], fill: Color32| {
        painter.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| p(x, y)).collect(), fill, stroke));
    };
    let arc = |cx: f32, cy: f32, rad: f32, from: f32, to: f32| -> Vec<Pos2> {
        let n = 16;
        (0..=n)
            .map(|i| {
                let a = from + (to - from) * i as f32 / n as f32;
                p(cx + rad * a.cos(), cy + rad * a.sin())
            })
            .collect()
    };
    let arrow_head = |tip: Pos2, dir: eframe::egui::Vec2| {
        let d = dir.normalized() * size * 0.16;
        let n = vec2(-d.y, d.x);
        painter.add(Shape::convex_polygon(vec![tip, tip - d + n * 0.8, tip - d - n * 0.8], color, Stroke::NONE));
    };
    let fill_soft = color.gamma_multiply(0.25);

    match icon {
        Icon::New => {
            line(&[
                (0.62, 0.12),
                (0.25, 0.12),
                (0.25, 0.88),
                (0.78, 0.88),
                (0.78, 0.28),
                (0.62, 0.12),
                (0.62, 0.28),
                (0.78, 0.28),
            ]);
            line(&[(0.52, 0.48), (0.52, 0.72)]);
            line(&[(0.40, 0.60), (0.64, 0.60)]);
        }
        Icon::Open => {
            line(&[(0.12, 0.80), (0.12, 0.22), (0.38, 0.22), (0.46, 0.32), (0.80, 0.32), (0.80, 0.44)]);
            closed(&[(0.12, 0.80), (0.26, 0.44), (0.92, 0.44), (0.78, 0.80)], fill_soft);
        }
        Icon::Save => {
            closed(&[(0.15, 0.15), (0.72, 0.15), (0.85, 0.28), (0.85, 0.85), (0.15, 0.85)], Color32::TRANSPARENT);
            line(&[(0.32, 0.15), (0.32, 0.36), (0.64, 0.36), (0.64, 0.15)]);
            painter.rect_stroke(
                Rect::from_min_max(p(0.30, 0.55), p(0.70, 0.85)),
                0.0,
                thin,
                eframe::egui::StrokeKind::Middle,
            );
        }
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let fx = |x: f32| if flip > 0.0 { x } else { 1.0 - x };
            let pts: Vec<Pos2> = arc(0.5, 0.58, 0.28, PI + 0.2, TAU + 0.6)
                .into_iter()
                .map(|q| pos2(r.left() + fx((q.x - r.left()) / size) * size, q.y))
                .collect();
            let start = pts[0];
            painter.add(Shape::line(pts, stroke));
            arrow_head(start + vec2(0.0, size * 0.02), vec2(-0.4 * flip, 1.0));
        }
        Icon::Select => {
            closed(
                &[(0.30, 0.12), (0.30, 0.80), (0.46, 0.64), (0.58, 0.90), (0.68, 0.85), (0.56, 0.60), (0.78, 0.60)],
                Color32::TRANSPARENT,
            );
        }
        Icon::Pen => {
            // Plumilla de dibujo.
            closed(&[(0.50, 0.12), (0.74, 0.50), (0.50, 0.88), (0.26, 0.50)], fill_soft);
            line(&[(0.50, 0.12), (0.50, 0.56)]);
            painter.circle_filled(p(0.50, 0.60), w * 1.3, color);
        }
        Icon::Rect => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.18, 0.25), p(0.82, 0.75)),
                1.0,
                stroke,
                eframe::egui::StrokeKind::Middle,
            );
            for (x, y) in [(0.18, 0.25), (0.82, 0.25), (0.18, 0.75), (0.82, 0.75)] {
                painter.rect_filled(Rect::from_center_size(p(x, y), vec2(w * 2.6, w * 2.6)), 0.0, color);
            }
        }
        Icon::Seam => {
            // Dos bordes de tela y las puntadas que los unen.
            line(&[(0.15, 0.30), (0.85, 0.30)]);
            line(&[(0.15, 0.70), (0.85, 0.70)]);
            for x in [0.25, 0.42, 0.59, 0.76] {
                painter.line_segment([p(x, 0.36), p(x + 0.08, 0.64)], thin);
            }
        }
        Icon::Duplicate => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.14, 0.14), p(0.62, 0.62)),
                1.0,
                thin,
                eframe::egui::StrokeKind::Middle,
            );
            painter.rect(
                Rect::from_min_max(p(0.38, 0.38), p(0.86, 0.86)),
                1.0,
                fill_soft,
                stroke,
                eframe::egui::StrokeKind::Middle,
            );
        }
        Icon::Delete => {
            line(&[(0.18, 0.26), (0.82, 0.26)]);
            line(&[(0.40, 0.26), (0.42, 0.14), (0.58, 0.14), (0.60, 0.26)]);
            line(&[(0.26, 0.26), (0.32, 0.88), (0.68, 0.88), (0.74, 0.26)]);
            line(&[(0.44, 0.40), (0.44, 0.76)]);
            line(&[(0.56, 0.40), (0.56, 0.76)]);
        }
        Icon::Fit => {
            for (sx, sy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let x0 = if sx == 0.0 { 0.14 } else { 0.86 };
                let y0 = if sy == 0.0 { 0.14 } else { 0.86 };
                let dx = if sx == 0.0 { 0.22 } else { -0.22 };
                let dy = if sy == 0.0 { 0.22 } else { -0.22 };
                line(&[(x0 + dx, y0), (x0, y0), (x0, y0 + dy)]);
            }
            painter.rect_filled(Rect::from_min_max(p(0.38, 0.38), p(0.62, 0.62)), 1.0, fill_soft);
        }
        Icon::Grid => {
            for t in [0.18, 0.39, 0.61, 0.82] {
                painter.line_segment([p(t, 0.14), p(t, 0.86)], thin);
                painter.line_segment([p(0.14, t), p(0.86, t)], thin);
            }
        }
        Icon::Mesh => {
            let pts = [(0.15, 0.80), (0.50, 0.80), (0.85, 0.80), (0.32, 0.50), (0.68, 0.50), (0.50, 0.20)];
            for (a, b) in [(0, 1), (1, 2), (0, 3), (1, 3), (1, 4), (2, 4), (3, 4), (3, 5), (4, 5)] {
                painter.line_segment([p(pts[a].0, pts[a].1), p(pts[b].0, pts[b].1)], thin);
            }
        }
        Icon::Play => {
            painter.add(Shape::convex_polygon(vec![p(0.30, 0.18), p(0.82, 0.50), p(0.30, 0.82)], color, Stroke::NONE));
        }
        Icon::Pause => {
            painter.rect_filled(Rect::from_min_max(p(0.26, 0.20), p(0.43, 0.80)), 1.0, color);
            painter.rect_filled(Rect::from_min_max(p(0.57, 0.20), p(0.74, 0.80)), 1.0, color);
        }
        Icon::Restart => {
            let pts = arc(0.5, 0.5, 0.32, -FRAC_PI_2 + 0.5, TAU - FRAC_PI_2 - 0.3);
            let end = *pts.last().unwrap();
            painter.add(Shape::line(pts, stroke));
            arrow_head(end + vec2(size * 0.06, 0.0), vec2(1.0, 0.25));
        }
        Icon::Live => {
            closed(&[(0.58, 0.10), (0.26, 0.56), (0.48, 0.56), (0.42, 0.90), (0.76, 0.42), (0.54, 0.42)], fill_soft);
        }
        Icon::Home => {
            closed(
                &[(0.50, 0.14), (0.86, 0.48), (0.76, 0.48), (0.76, 0.86), (0.24, 0.86), (0.24, 0.48), (0.14, 0.48)],
                fill_soft,
            );
        }
        Icon::Settings => {
            let c = p(0.5, 0.5);
            for k in 0..8 {
                let a = k as f32 * TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * size * 0.26, c + d * size * 0.40], Stroke::new(w * 1.8, color));
            }
            painter.circle_stroke(c, size * 0.24, stroke);
            painter.circle_stroke(c, size * 0.09, thin);
        }
        Icon::Pattern => {
            // Silueta de camiseta.
            closed(
                &[
                    (0.36, 0.14),
                    (0.14, 0.28),
                    (0.22, 0.44),
                    (0.30, 0.40),
                    (0.30, 0.88),
                    (0.70, 0.88),
                    (0.70, 0.40),
                    (0.78, 0.44),
                    (0.86, 0.28),
                    (0.64, 0.14),
                ],
                Color32::TRANSPARENT,
            );
            painter.add(Shape::line(arc(0.5, 0.12, 0.14, 0.3, PI - 0.3), thin));
        }
        Icon::Warning => {
            closed(&[(0.50, 0.12), (0.90, 0.84), (0.10, 0.84)], Color32::TRANSPARENT);
            line(&[(0.50, 0.38), (0.50, 0.62)]);
            painter.circle_filled(p(0.50, 0.73), w, color);
        }
        Icon::Allowance => {
            // Una pieza y, alrededor, la línea de corte discontinua.
            painter.rect_stroke(
                Rect::from_min_max(p(0.30, 0.30), p(0.70, 0.70)),
                1.0,
                stroke,
                eframe::egui::StrokeKind::Middle,
            );
            let outer = [(0.14, 0.14), (0.86, 0.14), (0.86, 0.86), (0.14, 0.86), (0.14, 0.14)];
            let pts: Vec<Pos2> = outer.iter().map(|&(x, y)| p(x, y)).collect();
            painter.extend(Shape::dashed_line(&pts, thin, size * 0.08, size * 0.06));
        }
        Icon::Flip => {
            // Volver del revés: dos flechas que dan la vuelta.
            painter.add(Shape::line(arc(0.5, 0.5, 0.30, 3.6, 6.0), stroke));
            arrow_head(p(0.5 + 0.30 * 6.0f32.cos(), 0.5 + 0.30 * 6.0f32.sin()), vec2(0.3, 1.0));
            painter.add(Shape::line(arc(0.5, 0.5, 0.30, 0.46, 2.86), stroke));
            arrow_head(p(0.5 + 0.30 * 2.86f32.cos(), 0.5 + 0.30 * 2.86f32.sin()), vec2(-0.3, -1.0));
        }
        Icon::Order => {
            for y in [0.25f32, 0.5, 0.75] {
                painter.circle_filled(p(0.2, y), size * 0.05, color);
                line(&[(0.34, y), (0.84, y)]);
            }
        }
        Icon::Highlight => {
            // Una costura resaltada sobre la tela.
            closed(&[(0.12, 0.25), (0.88, 0.25), (0.88, 0.75), (0.12, 0.75)], fill_soft);
            painter.line_segment([p(0.5, 0.18), p(0.5, 0.82)], Stroke::new(w * 2.0, color));
        }
        Icon::Mannequin => {
            painter.circle_stroke(p(0.5, 0.20), size * 0.09, stroke);
            line(&[(0.50, 0.30), (0.50, 0.62)]);
            line(&[(0.24, 0.46), (0.50, 0.36), (0.76, 0.46)]);
            line(&[(0.34, 0.90), (0.50, 0.62), (0.66, 0.90)]);
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            painter.circle_stroke(p(0.43, 0.43), size * 0.26, stroke);
            line(&[(0.62, 0.62), (0.86, 0.86)]);
            line(&[(0.31, 0.43), (0.55, 0.43)]);
            if icon == Icon::ZoomIn {
                line(&[(0.43, 0.31), (0.43, 0.55)]);
            }
        }
        Icon::Layout2D | Icon::LayoutSplit | Icon::Layout3D => {
            let frame = Rect::from_min_max(p(0.12, 0.20), p(0.88, 0.80));
            painter.rect_stroke(frame, 1.0, thin, eframe::egui::StrokeKind::Middle);
            let half = Rect::from_min_max(frame.min, pos2(frame.center().x, frame.max.y));
            let fill = match icon {
                Icon::Layout2D => frame,
                Icon::Layout3D => Rect::from_min_max(pos2(frame.center().x, frame.min.y), frame.max),
                _ => half,
            };
            painter.rect_filled(fill.shrink(w), 0.0, color.gamma_multiply(0.6));
            if icon == Icon::LayoutSplit {
                painter.line_segment([frame.center_top(), frame.center_bottom()], thin);
            }
        }
    }
}
