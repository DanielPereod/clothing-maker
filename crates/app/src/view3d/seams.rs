//! Geometría de las costuras en la vista 3D: el margen tumbado por su lado,
//! los pespuntes vistos y, si se quiere, un cordón de color sobre la línea
//! de costura para encontrarlas.

use glam::Vec3;
use pattern::{AllowanceSide, SeamKind};

use super::render::Vertex;
use super::scene::{FastenerDraw, SeamDraw, SeamSideDraw};

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
/// Dientes de una cremallera: color (lineal), cuánto salen de cada cinta,
/// paso entre dientes y qué parte del paso ocupa cada uno (m).
const TEETH: [f32; 3] = [0.55, 0.55, 0.58];
const TOOTH_DEPTH: f32 = 0.003;
const TOOTH_PITCH: f32 = 0.003;
const TOOTH_FILL: f32 = 0.55;
/// El tirador de la cremallera: color, largo, ancho y alto (m).
const SLIDER: [f32; 3] = [0.25, 0.25, 0.27];
const SLIDER_SIZE: [f32; 3] = [0.014, 0.009, 0.003];
/// Radio de un broche y largo y ancho de una presilla (m).
const SNAP_RADIUS: f32 = 0.006;
const BAR_TACK: [f32; 2] = [0.012, 0.003];

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
        if draw.kind == SeamKind::Zipper {
            zipper(draw, positions, normals, n, out);
        }
        for (j, side) in draw.sides.iter().enumerate().filter(|_| draw.kind != SeamKind::Zipper) {
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

/// Broches (un disco de metal por el derecho de cada pieza) y presillas
/// (una barra de hilo por las dos caras de cada una).
pub fn fasteners(draws: &[FastenerDraw], positions: &[Vec3], normals: &[Vec3], out: &mut SeamMesh) {
    for draw in draws {
        for (k, &particle) in draw.particles.iter().enumerate() {
            let p = positions[particle as usize];
            let n = normals[particle as usize].normalize_or_zero();
            // Dos direcciones sobre la tela.
            let u = n.any_orthonormal_vector();
            let v = n.cross(u);
            let faces: &[f32] = match draw.kind {
                pattern::FastenerKind::Snap => &[1.0],
                pattern::FastenerKind::BarTack => &[1.0, -1.0],
            };
            for &face in faces {
                let up = n * draw.right_sign[k] * face;
                let base = p + up * LIFT * 2.0;
                match draw.kind {
                    pattern::FastenerKind::Snap => {
                        // Un octógono.
                        let corner = |i: usize| {
                            let a = i as f32 * std::f32::consts::TAU / 8.0;
                            base + (u * a.cos() + v * a.sin()) * SNAP_RADIUS
                        };
                        for i in 0..8 {
                            out.quad([base, corner(i), corner(i + 1), base], up, SLIDER);
                        }
                    }
                    pattern::FastenerKind::BarTack => {
                        let (a, w) = (u * BAR_TACK[0] / 2.0, v * BAR_TACK[1] / 2.0);
                        out.quad([base - a - w, base + a - w, base + a + w, base - a + w], up, THREAD);
                    }
                }
            }
        }
    }
}

/// Los dientes de cada cinta de una cremallera, por las dos caras (los de
/// una cinta, medio paso desplazados de los de la otra: encajan), y el
/// tirador donde empieza la parte cerrada.
fn zipper(draw: &SeamDraw, positions: &[Vec3], normals: &[Vec3], n: usize, out: &mut SeamMesh) {
    for (j, side) in draw.sides.iter().enumerate() {
        for face in [1.0f32, -1.0] {
            let path: Vec<_> = (0..n)
                .map(|k| {
                    let nrm = normals[side.particles[k] as usize];
                    let dir = inward(side, k, positions, nrm)?;
                    let up = nrm * face;
                    // Desde un poco más allá del canto: los dientes de las dos
                    // cintas se tocan en el centro.
                    let p = positions[side.particles[k] as usize] + up * (side.thickness / 2.0 + LIFT) - dir * 0.0005;
                    Some((p, dir * TOOTH_DEPTH, up))
                })
                .collect();
            teeth(&path, j as f32 * 0.5 * TOOTH_PITCH, out);
        }
    }
    // El tirador, sobre la primera cinta por su cara de delante.
    let (Some(first), Some(k)) = (draw.sides.first(), draw.slider) else { return };
    let k = k.min(n.saturating_sub(1));
    if n < 2 {
        return;
    }
    let p = positions[first.particles[k] as usize];
    let nrm = normals[first.particles[k] as usize] * first.right_sign;
    let next = positions[first.particles[(k + 1).min(n - 1)] as usize];
    let prev = positions[first.particles[k.saturating_sub(1)] as usize];
    let along = (next - prev).normalize_or_zero();
    let across = along.cross(nrm).normalize_or_zero();
    let [length, width, height] = SLIDER_SIZE;
    let base = p + nrm * (first.thickness / 2.0 + LIFT * 2.0);
    let (a, w, h) = (along * length / 2.0, across * width / 2.0, nrm * height);
    // Una caja: la tapa y los cuatro lados.
    let corners = |z: Vec3| [base - a - w + z, base + a - w + z, base + a + w + z, base - a + w + z];
    let (bottom, top) = (corners(Vec3::ZERO), corners(h));
    out.quad(top, nrm, SLIDER);
    for i in 0..4 {
        let j = (i + 1) % 4;
        let outward = ((bottom[i] + bottom[j]) / 2.0 - base).normalize_or_zero();
        out.quad([bottom[i], bottom[j], top[j], top[i]], outward, SLIDER);
    }
}

/// Dientes a lo largo de `path` (punto, hacia dónde sale el diente y hacia
/// dónde mira), empezando `phase` metros dentro del primer paso.
fn teeth(path: &[Option<(Vec3, Vec3, Vec3)>], phase: f32, out: &mut SeamMesh) {
    let mut phase = phase % TOOTH_PITCH;
    for pair in path.windows(2) {
        let (Some((p0, a0, n0)), Some((p1, a1, n1))) = (pair[0], pair[1]) else { continue };
        let length = p0.distance(p1);
        if length < 1e-6 {
            continue;
        }
        let up = (n0 + n1).normalize_or_zero();
        let mut t = 0.0;
        while t < length {
            let tooth = TOOTH_PITCH * TOOTH_FILL;
            let in_tooth = phase < tooth;
            let step = if in_tooth { tooth - phase } else { TOOTH_PITCH - phase }.min(length - t);
            if in_tooth {
                let (s0, s1) = (t / length, (t + step) / length);
                let (a, b) = (p0.lerp(p1, s0), p0.lerp(p1, s1));
                let (da, db) = (a0.lerp(a1, s0), a0.lerp(a1, s1));
                // El diente sale hacia el otro lado de la cinta.
                out.quad([a, b, b - db, a - da], up, TEETH);
            }
            t += step;
            phase = (phase + step) % TOOTH_PITCH;
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
            firm: false,
            thickness: 0.003,
            allowance: 0.01,
            right_sign: 1.0,
            right_color: [1.0, 0.0, 0.0],
            wrong_color: [0.5, 0.5, 0.5],
        };
        let draw = SeamDraw { index: 0, kind, topstitch, pressed: None, sides: vec![side(1), side(2)], slider: None };
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
