//! Dibujo de la sección de una costura (el corte transversal, como en los
//! libros de patronaje): cómo quedan las capas de tela, el margen y las
//! puntadas según el tipo de costura.

use eframe::egui::{self, Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use pattern::{SeamKind, Topstitch};

use crate::theme;

/// Una capa de tela: la línea que sigue y hacia qué lado tiene el derecho.
struct Ply {
    points: Vec<(f32, f32)>,
    /// 1 si el derecho queda a la izquierda del avance (arriba al ir hacia
    /// la derecha), −1 si queda a la derecha.
    right: f32,
    /// 0 = pieza del primer lado (a la izquierda), 1 = la del segundo.
    piece: usize,
}

/// Capas de cada tipo de costura, con el margen hacia la pieza 0 (la de la
/// izquierda) si va a un lado. Unidades: el dibujo ocupa x ∈ [−90, 90] e
/// y ∈ [−12, 14], con y hacia abajo y el derecho de la prenda arriba.
fn plies(kind: SeamKind, open: bool) -> (Vec<Ply>, Vec<(f32, f32, f32)>) {
    let ply = |piece: usize, right: f32, points: &[(f32, f32)]| Ply { points: points.to_vec(), right, piece };
    // Puntadas: (x, y desde, y hasta).
    match kind {
        SeamKind::Plain | SeamKind::Overlocked if open && kind == SeamKind::Plain => (
            vec![
                ply(0, 1.0, &[(-90.0, 0.0), (-3.0, 0.0), (-3.0, 5.0), (-35.0, 5.0)]),
                ply(1, -1.0, &[(90.0, 0.0), (3.0, 0.0), (3.0, 5.0), (35.0, 5.0)]),
            ],
            vec![(0.0, -1.0, 4.0)],
        ),
        SeamKind::Plain | SeamKind::Overlocked => (
            vec![
                ply(0, 1.0, &[(-90.0, 0.0), (-3.0, 0.0), (-3.0, 5.0), (-30.0, 5.0)]),
                ply(1, -1.0, &[(90.0, 0.0), (3.0, 0.0), (3.0, 10.0), (-28.0, 10.0)]),
            ],
            vec![(0.0, -1.0, 4.0)],
        ),
        SeamKind::Exposed if open => (
            vec![
                ply(0, 1.0, &[(-90.0, 0.0), (-3.0, 0.0), (-3.0, -5.0), (-35.0, -5.0)]),
                ply(1, -1.0, &[(90.0, 0.0), (3.0, 0.0), (3.0, -5.0), (35.0, -5.0)]),
            ],
            vec![(0.0, -4.0, 1.0)],
        ),
        SeamKind::Exposed => (
            vec![
                ply(0, 1.0, &[(-90.0, 0.0), (-3.0, 0.0), (-3.0, -10.0), (-28.0, -10.0)]),
                ply(1, -1.0, &[(90.0, 0.0), (3.0, 0.0), (3.0, -5.0), (-30.0, -5.0)]),
            ],
            vec![(0.0, -4.0, 1.0)],
        ),
        SeamKind::French => (
            vec![
                ply(0, 1.0, &[(-90.0, 0.0), (-3.0, 0.0), (-3.0, 5.0), (-18.0, 5.0)]),
                ply(1, -1.0, &[(90.0, 0.0), (3.0, 0.0), (3.0, 12.0), (-24.0, 12.0), (-24.0, 8.0), (-8.0, 8.0)]),
            ],
            vec![(0.0, -1.0, 4.0), (-20.0, 7.0, 13.0)],
        ),
        SeamKind::FlatFelled => (
            vec![
                ply(0, 1.0, &[(-90.0, 0.0), (14.0, 0.0), (14.0, 10.0), (-14.0, 10.0)]),
                ply(1, -1.0, &[(90.0, 0.0), (18.0, 0.0), (18.0, 5.0), (-10.0, 5.0)]),
            ],
            vec![],
        ),
        SeamKind::Lapped => {
            (vec![ply(0, 1.0, &[(-90.0, 0.0), (14.0, 0.0)]), ply(1, -1.0, &[(90.0, 5.0), (-14.0, 5.0)])], vec![])
        }
        SeamKind::Bagged => (
            vec![
                ply(0, 1.0, &[(-90.0, -6.0), (30.0, -6.0), (36.0, -1.0), (14.0, -1.0)]),
                ply(1, -1.0, &[(-90.0, 8.0), (30.0, 8.0), (36.0, 3.0), (14.0, 3.0)]),
            ],
            vec![(34.0, -2.0, 4.0)],
        ),
        // Las dos cintas a tope; los dientes se dibujan aparte.
        SeamKind::Zipper => {
            (vec![ply(0, 1.0, &[(-90.0, 0.0), (-4.0, 0.0)]), ply(1, -1.0, &[(90.0, 0.0), (4.0, 0.0)])], vec![])
        }
    }
}

/// Dibuja la sección de una costura. `colors` son los de las dos primeras
/// piezas que une; `pressed` el lado al que va el margen.
pub fn show(ui: &mut Ui, kind: SeamKind, pressed: Option<usize>, topstitch: Topstitch, colors: [Color32; 2]) {
    let width = ui.available_width().min(300.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 78.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);

    let open = pressed.is_none();
    let (mut plies, mut stitches) = plies(kind, open);
    // Margen hacia el segundo lado: el mismo dibujo, en espejo.
    let mirror = pressed == Some(1);
    if mirror {
        for ply in &mut plies {
            ply.points.iter_mut().for_each(|p| p.0 = -p.0);
            ply.right = -ply.right;
            ply.piece = 1 - ply.piece;
        }
        stitches.iter_mut().for_each(|s| s.0 = -s.0);
    }
    let scale = (width - 20.0) / 180.0;
    let at = |(x, y): (f32, f32)| pos2(rect.center().x + x * scale, rect.top() + 34.0 + y * scale * 1.6);

    // Cada capa: el revés en un tono apagado y, encima, una línea fina del
    // color de la pieza por el lado del derecho.
    for ply in &plies {
        let pts: Vec<Pos2> = ply.points.iter().map(|&p| at(p)).collect();
        let color = colors[ply.piece];
        let wrong = color.gamma_multiply(0.45).blend(Color32::from_gray(150).gamma_multiply(0.5));
        painter.add(Shape::line(pts.clone(), Stroke::new(5.0, wrong)));
        let face: Vec<Pos2> = (0..pts.len())
            .map(|k| {
                let prev = pts[k.saturating_sub(1)];
                let next = pts[(k + 1).min(pts.len() - 1)];
                let d = (next - prev).normalized();
                pts[k] + Vec2::new(d.y, -d.x) * (1.8 * ply.right)
            })
            .collect();
        painter.add(Shape::line(face, Stroke::new(2.0, color)));
    }

    let thread = Color32::from_rgb(214, 160, 60);
    for &(x, y0, y1) in &stitches {
        painter.line_segment([at((x, y0)), at((x, y1))], Stroke::new(2.0, thread));
    }
    // Pespuntes: atraviesan todas las capas del lado del margen.
    let side = if mirror { 1.0 } else { -1.0 };
    let depth = plies.iter().flat_map(|p| p.points.iter().map(|q| q.1)).fold(0.0f32, f32::max);
    for k in 0..topstitch.lines() {
        let x = side * (6.0 + 8.0 * k as f32);
        let pts = [at((x, -2.0)), at((x, depth + 2.0))];
        painter.extend(Shape::dashed_line(&pts, Stroke::new(1.6, thread), 3.0, 2.0));
    }
    if kind == SeamKind::Zipper {
        // Los dientes, que encajan unos en otros entre las cintas.
        let teeth = Color32::from_gray(190);
        for k in 0..2 {
            let x = if k == 0 { -10.0 } else { 0.0 };
            let r = egui::Rect::from_two_pos(at((x, -4.0)), at((x + 10.0, 4.0)));
            painter.rect_filled(r, 1.5, teeth.gamma_multiply(if k == 0 { 1.0 } else { 0.8 }));
        }
    }
    if kind == SeamKind::Overlocked {
        // El remallado: zigzag sobre los cantos del margen.
        let x0 = if mirror { 26.0 } else { -34.0 };
        let pts: Vec<Pos2> = (0..9).map(|k| at((x0 + k as f32, if k % 2 == 0 { 3.0 } else { 12.0 }))).collect();
        painter.add(Shape::line(pts, Stroke::new(1.0, thread)));
    }

    let small = egui::FontId::proportional(10.0);
    painter.text(rect.left_top() + vec2(6.0, 4.0), egui::Align2::LEFT_TOP, "fuera ↑", small.clone(), theme::TEXT_WEAK);
    painter.text(rect.left_bottom() + vec2(6.0, -4.0), egui::Align2::LEFT_BOTTOM, "dentro ↓", small, theme::TEXT_WEAK);
}
