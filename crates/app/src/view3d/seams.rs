//! Geometría de las costuras en la vista 3D: el margen tumbado por su lado,
//! los pespuntes vistos y, si se quiere, un cordón de color sobre la línea
//! de costura para encontrarlas.

use glam::Vec3;
use pattern::{AllowanceSide, SeamKind};

use super::render::Vertex;
use super::scene::{SeamDraw, SeamSideDraw};

/// Color (lineal) del hilo de los pespuntes y del remallado.
const THREAD: [f32; 3] = [0.80, 0.52, 0.12];
/// Separación (m) entre la tela y lo que se dibuja encima, para que no se
/// mezclen al pintarlos.
const LIFT: f32 = 0.0015;
/// Separación extra entre capas de margen.
const PLY: f32 = 0.0008;
/// Ancho (m) del cordón que resalta la costura.
const CORD: f32 = 0.004;
/// Ancho (m) del hilo de los pespuntes.
const THREAD_WIDTH: f32 = 0.0012;
/// Distancias (m) de los pespuntes a la costura.
const TOPSTITCH_AT: [f32; 2] = [0.003, 0.009];
/// Largo de cada puntada visible y de cada hueco (m).
const DASH: f32 = 0.004;
const GAP: f32 = 0.002;

/// Malla que se va llenando.
#[derive(Default)]
pub struct SeamMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl SeamMesh {
    /// Un cuadrilátero `a b c d` (en orden alrededor) que mira hacia `normal`.
    fn quad(&mut self, [a, b, c, d]: [Vec3; 4], normal: Vec3, color: [f32; 3]) {
        let base = self.vertices.len() as u32;
        for p in [a, b, c, d] {
            self.vertices.push(Vertex::plain(p, normal, color));
        }
        // Que la cara de delante sea la que mira hacia la normal.
        if (b - a).cross(c - a).dot(normal) >= 0.0 {
            self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        } else {
            self.indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }

    /// Una cinta a lo largo de `points`: cada punto se ensancha hacia
    /// `across[k]` (de `0` a `across[k]`) y la cinta mira hacia `normals[k]`.
    fn strip(&mut self, points: &[Option<(Vec3, Vec3, Vec3)>], color: [f32; 3]) {
        for pair in points.windows(2) {
            let (Some((p0, a0, n0)), Some((p1, a1, n1))) = (pair[0], pair[1]) else { continue };
            self.quad([p0, p1, p1 + a1, p0 + a0], (n0 + n1).normalize_or_zero(), color);
        }
    }
}

/// Dirección (unitaria, sobre la tela) de un punto de la costura hacia el
/// interior de su pieza.
fn inward(side: &SeamSideDraw, k: usize, positions: &[Vec3], normal: Vec3) -> Option<Vec3> {
    let p = positions[side.particles[k] as usize];
    let q = positions[side.inward[k]? as usize];
    let d = q - p;
    let d = d - normal * d.dot(normal);
    let d = d.normalize_or_zero();
    (d != Vec3::ZERO).then_some(d)
}

/// Añade a `out` todo lo que se dibuja de las costuras con la tela en
/// `positions` (con sus normales). `color_of(i)` es el color de la costura
/// `i` del patrón, para el cordón.
pub fn build(
    draws: &[SeamDraw],
    positions: &[Vec3],
    normals: &[Vec3],
    highlight: bool,
    color_of: impl Fn(usize) -> [f32; 3],
    out: &mut SeamMesh,
) {
    for draw in draws {
        let Some(first) = draw.sides.first() else { continue };
        let n = draw.sides.iter().map(|s| s.particles.len()).min().unwrap_or(0);
        let normal = |side: &SeamSideDraw, k: usize| normals[side.particles[k] as usize];

        // El margen de cada lado, tumbado hacia su pieza o hacia la del
        // lado al que va planchado, por la cara que toque.
        let face = match draw.kind.allowance_side() {
            AllowanceSide::WrongSide => -1.0,
            AllowanceSide::RightSide => 1.0,
        };
        for (j, side) in draw.sides.iter().enumerate() {
            let toward = draw.pressed.and_then(|p| draw.sides.get(p)).unwrap_or(side);
            // La cara del margen que queda a la vista.
            let visible = if face > 0.0 { side.right_color } else { side.wrong_color };
            let strip: Vec<_> = (0..n)
                .map(|k| {
                    let nrm = normal(side, k);
                    let dir = inward(toward, k, positions, normal(toward, k))?;
                    let up = nrm * (face * side.right_sign);
                    let p = positions[side.particles[k] as usize] + up * (LIFT + PLY * j as f32);
                    Some((p, dir * side.allowance, up))
                })
                .collect();
            out.strip(&strip, visible);
            if draw.kind == SeamKind::Overlocked {
                // El remallado: hilo en el canto del margen.
                let edge: Vec<_> =
                    strip.iter().map(|s| s.map(|(p, a, up)| (p + a * 0.7 + up * 0.0002, a * 0.3, up))).collect();
                out.strip(&edge, THREAD);
            }
        }

        // Pespuntes por el derecho, hacia el lado planchado.
        let lines = draw.topstitch.lines();
        if lines > 0 {
            let side = draw.pressed.and_then(|p| draw.sides.get(p)).unwrap_or(first);
            for &dist in &TOPSTITCH_AT[..lines] {
                let path: Vec<Option<(Vec3, Vec3)>> = (0..n)
                    .map(|k| {
                        let nrm = normal(side, k);
                        let dir = inward(side, k, positions, nrm)?;
                        let up = nrm * side.right_sign;
                        Some((positions[side.particles[k] as usize] + dir * dist + up * LIFT, up))
                    })
                    .collect();
                dashes(&path, out);
            }
        }

        // Cordón de color sobre la línea de costura, por las dos caras.
        if highlight {
            let color = color_of(draw.index);
            for sign in [1.0f32, -1.0] {
                let strip: Vec<_> = (0..n)
                    .map(|k| {
                        let nrm = normal(first, k) * sign;
                        let p = positions[first.particles[k] as usize];
                        let prev = positions[first.particles[k.saturating_sub(1)] as usize];
                        let next = positions[first.particles[(k + 1).min(n - 1)] as usize];
                        let across = (next - prev).cross(nrm).normalize_or_zero() * CORD;
                        Some((p + nrm * (LIFT * 1.5) - across / 2.0, across, nrm))
                    })
                    .collect();
                out.strip(&strip, color);
            }
        }
    }
}

/// Puntadas a lo largo de `path` (punto y hacia dónde mira la tela).
fn dashes(path: &[Option<(Vec3, Vec3)>], out: &mut SeamMesh) {
    // Distancia recorrida dentro del ciclo puntada + hueco.
    let mut phase = 0.0f32;
    for pair in path.windows(2) {
        let (Some((p0, n0)), Some((p1, n1))) = (pair[0], pair[1]) else { continue };
        let length = p0.distance(p1);
        if length < 1e-6 {
            continue;
        }
        let dir = (p1 - p0) / length;
        let up = (n0 + n1).normalize_or_zero();
        let across = dir.cross(up).normalize_or_zero() * (THREAD_WIDTH / 2.0);
        let mut t = 0.0;
        while t < length {
            let in_dash = phase < DASH;
            let step = if in_dash { DASH - phase } else { DASH + GAP - phase }.min(length - t);
            if in_dash {
                let (a, b) = (p0 + dir * t, p0 + dir * (t + step));
                out.quad([a - across, b - across, b + across, a + across], up, THREAD);
            }
            t += step;
            phase = (phase + step) % (DASH + GAP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pattern::Topstitch;

    /// Una costura recta de 10 cm en el plano XZ, entre dos piezas tumbadas.
    fn straight(kind: SeamKind, topstitch: Topstitch) -> (SeamDraw, Vec<Vec3>, Vec<Vec3>) {
        let n = 11;
        let mut positions = Vec::new();
        for k in 0..n {
            let z = k as f32 * 0.01;
            positions.extend([Vec3::new(0.0, 0.0, z), Vec3::new(-0.02, 0.0, z), Vec3::new(0.02, 0.0, z)]);
        }
        let normals = vec![Vec3::Y; positions.len()];
        let side = |offset: u32| SeamSideDraw {
            particles: (0..n as u32).map(|k| 3 * k).collect(),
            inward: (0..n as u32).map(|k| Some(3 * k + offset)).collect(),
            inward_rest: vec![0.02; n],
            compliance: 1e-3,
            thickness: 0.003,
            allowance: 0.01,
            right_sign: 1.0,
            right_color: [1.0, 0.0, 0.0],
            wrong_color: [0.5, 0.5, 0.5],
        };
        let draw = SeamDraw { index: 0, kind, topstitch, pressed: None, sides: vec![side(1), side(2)] };
        (draw, positions, normals)
    }

    #[test]
    fn allowance_lies_on_the_wrong_side() {
        let (draw, positions, normals) = straight(SeamKind::Plain, Topstitch::None);
        let mut out = SeamMesh::default();
        build(&[draw], &positions, &normals, false, |_| [0.0; 3], &mut out);
        // Dos márgenes de 10 tramos, dos triángulos por tramo.
        assert_eq!(out.indices.len(), 2 * 10 * 6);
        assert!(out.vertices.iter().all(|v| v.position[1] < 0.0), "el margen va por debajo (el revés)");
        // Cada margen hacia su pieza, 1 cm.
        let x = out.vertices.iter().map(|v| v.position[0]);
        let (min, max) = x.fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), x| (a.min(x), b.max(x)));
        assert!((min + 0.01).abs() < 1e-5 && (max - 0.01).abs() < 1e-5, "{min} {max}");
    }

    #[test]
    fn exposed_seams_and_topstitching_show_on_the_right_side() {
        let (draw, positions, normals) = straight(SeamKind::Exposed, Topstitch::Double);
        let mut out = SeamMesh::default();
        build(&[draw], &positions, &normals, true, |_| [0.0; 3], &mut out);
        let above = out.vertices.iter().filter(|v| v.position[1] > 0.0).count();
        let below = out.vertices.iter().filter(|v| v.position[1] < 0.0).count();
        // Todo por encima salvo el cordón de la cara de abajo.
        assert!(above > 3 * below, "{above} arriba, {below} abajo");
        assert!(out.vertices.iter().any(|v| v.color == THREAD));
    }
}
