//! Cubo de vista: un cubo pequeño en la esquina que gira con la cámara.
//! Al pulsar una cara, la cámara vuela hasta esa vista (frente, lados,
//! detrás o arriba).

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, vec2};
use glam::Vec3;

use super::Camera;
use crate::theme;

const FLY_TIME: Duration = Duration::from_millis(350);
/// Inclinación de la vista desde arriba: casi vertical, que la cámara no
/// puede pasar del cenit.
const TOP_PITCH: f32 = 1.5;

/// Transición animada de la cámara entre dos vistas.
#[derive(Clone, Copy, Debug)]
pub struct Fly {
    from: Camera,
    to: Camera,
    start: Instant,
}

impl Fly {
    pub fn new(from: Camera, to: Camera) -> Self {
        // Gira por el camino corto.
        let mut to = to;
        let delta = (to.yaw - from.yaw).rem_euclid(TAU);
        to.yaw = from.yaw + if delta > PI { delta - TAU } else { delta };
        Self { from, to, start: Instant::now() }
    }

    /// Cámara en este instante; `None` cuando ha llegado.
    pub fn camera(&self) -> Option<Camera> {
        let t = self.start.elapsed().as_secs_f32() / FLY_TIME.as_secs_f32();
        if t >= 1.0 {
            return None;
        }
        let s = t * t * (3.0 - 2.0 * t);
        let (a, b) = (&self.from, &self.to);
        Some(Camera {
            target: a.target.lerp(b.target, s),
            yaw: a.yaw + (b.yaw - a.yaw) * s,
            pitch: a.pitch + (b.pitch - a.pitch) * s,
            distance: a.distance + (b.distance - a.distance) * s,
        })
    }

    pub fn end(&self) -> Camera {
        self.to
    }
}

struct Face {
    normal: Vec3,
    /// Ejes de la cara (u a la derecha, v hacia arriba vista desde fuera).
    u: Vec3,
    v: Vec3,
    label: &'static str,
    /// Orientación de la cámara al pulsarla (giro, inclinación).
    view: Option<(f32, f32)>,
}

const FACES: [Face; 6] = [
    Face { normal: Vec3::Z, u: Vec3::X, v: Vec3::Y, label: "FRENTE", view: Some((0.0, 0.0)) },
    Face { normal: Vec3::NEG_Z, u: Vec3::NEG_X, v: Vec3::Y, label: "DETRÁS", view: Some((PI, 0.0)) },
    Face { normal: Vec3::X, u: Vec3::NEG_Z, v: Vec3::Y, label: "DCHA.", view: Some((FRAC_PI_2, 0.0)) },
    Face { normal: Vec3::NEG_X, u: Vec3::Z, v: Vec3::Y, label: "IZDA.", view: Some((-FRAC_PI_2, 0.0)) },
    Face { normal: Vec3::Y, u: Vec3::X, v: Vec3::NEG_Z, label: "ARRIBA", view: Some((0.0, TOP_PITCH)) },
    Face { normal: Vec3::NEG_Y, u: Vec3::X, v: Vec3::Z, label: "ABAJO", view: None },
];

/// Dibuja el cubo en la esquina superior derecha de `viewport`. Devuelve la
/// vista elegida si se ha pulsado una cara.
pub fn show(ui: &mut Ui, viewport: Rect, camera: &Camera) -> Option<(f32, f32)> {
    let size = 92.0;
    let rect = Rect::from_min_size(egui::pos2(viewport.right() - size - 10.0, viewport.top() + 10.0), vec2(size, size));
    let response = ui.interact(rect, ui.scope_id().with("viewcube"), Sense::click());
    let painter = ui.painter_at(rect.expand(2.0));

    // Base de la cámara: con ella el cubo gira igual que la escena.
    let eye_dir = (camera.eye() - camera.target).normalize_or(Vec3::Z);
    let forward = -eye_dir;
    let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(forward);
    let scale = size * 0.29;
    let center = rect.center();
    let project = |p: Vec3| -> Pos2 { center + vec2(p.dot(right), -p.dot(up)) * scale };

    // Sombra en el suelo del cubo.
    painter.add(Shape::ellipse_filled(
        center + vec2(0.0, scale * 1.55),
        vec2(scale * 1.4, scale * 0.35),
        Color32::from_black_alpha(50),
    ));

    let mut faces: Vec<(f32, &Face)> =
        FACES.iter().map(|f| (f.normal.dot(eye_dir), f)).filter(|(facing, _)| *facing > 0.02).collect();
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));

    let hover = response.hover_pos();
    let mut hovered = None;
    let mut polys = Vec::new();
    for &(facing, face) in &faces {
        let corners: Vec<Pos2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .iter()
            .map(|&(a, b)| project(face.normal + face.u * a + face.v * b))
            .collect();
        if hover.is_some_and(|h| inside(&corners, h)) && face.view.is_some() {
            hovered = Some(face.label);
        }
        polys.push((facing, face, corners));
    }
    for (facing, face, corners) in &polys {
        let lit = 0.55 + 0.45 * facing;
        let base = if hovered == Some(face.label) { theme::ACCENT } else { Color32::from_gray((175.0 * lit) as u8) };
        painter.add(Shape::convex_polygon(corners.clone(), base, Stroke::new(1.0, Color32::from_gray(70))));
        if *facing > 0.5 {
            // El texto, en el centro de la cara, encogido según lo de frente que esté.
            let c = project(face.normal);
            let font = FontId::proportional(7.0 + 4.0 * facing);
            painter.text(
                c,
                Align2::CENTER_CENTER,
                face.label,
                font,
                Color32::from_gray(40).gamma_multiply(facing.min(1.0)),
            );
        }
    }

    if response.hovered() {
        ui.ctx().set_cursor_icon(if hovered.is_some() {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        });
    }
    if response.clicked() {
        return FACES.iter().find(|f| Some(f.label) == hovered).and_then(|f| f.view);
    }
    None
}

fn inside(poly: &[Pos2], p: Pos2) -> bool {
    let mut sign = 0.0;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let cross = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}
