//! Transferencia del estado 3D entre dos mallas de la misma pieza, para que
//! al editar el patrón la prenda se reajuste en vez de empezar de cero.
//!
//! Cada punto de la malla nueva se busca (en 2D) en la malla vieja y su
//! posición 3D se interpola con las coordenadas baricéntricas del triángulo
//! que lo contiene. Si queda fuera (la pieza ha crecido), se usa el
//! triángulo más cercano y se extrapola con su misma transformación.

use std::collections::HashMap;

use glam::{DVec2, Vec3};
use mesher::Mesh2D;

/// La malla vieja de una pieza con la posición 3D de cada vértice.
pub struct Remap<'a> {
    mesh: &'a Mesh2D,
    positions: &'a [Vec3],
    /// Desplazamiento (mm) del patrón nuevo respecto al viejo: se resta a los
    /// puntos nuevos antes de buscarlos.
    offset: DVec2,
    cell: f64,
    grid: HashMap<[i64; 2], Vec<u32>>,
}

impl<'a> Remap<'a> {
    /// `positions` son las posiciones 3D actuales de los vértices de `mesh`.
    pub fn new(mesh: &'a Mesh2D, positions: &'a [Vec3], offset: [f64; 2]) -> Self {
        assert_eq!(mesh.positions.len(), positions.len());
        let point = |i: u32| DVec2::from(mesh.positions[i as usize]);
        // Celdas del tamaño de la arista media.
        let edges = mesh.edges();
        let mean = edges.iter().map(|&[a, b]| point(a).distance(point(b))).sum::<f64>() / edges.len().max(1) as f64;
        let cell = mean.max(1.0);
        let mut grid: HashMap<[i64; 2], Vec<u32>> = HashMap::new();
        for (i, t) in mesh.triangles.iter().enumerate() {
            let [a, b, c] = t.map(point);
            let (lo, hi) = (a.min(b).min(c) / cell, a.max(b).max(c) / cell);
            for x in lo.x.floor() as i64..=hi.x.floor() as i64 {
                for y in lo.y.floor() as i64..=hi.y.floor() as i64 {
                    grid.entry([x, y]).or_default().push(i as u32);
                }
            }
        }
        Self { mesh, positions, offset: DVec2::from(offset), cell, grid }
    }

    fn corners(&self, t: u32) -> [DVec2; 3] {
        self.mesh.triangles[t as usize].map(|i| DVec2::from(self.mesh.positions[i as usize]))
    }

    /// Posición 3D del punto `p` (mm) del patrón nuevo.
    pub fn at(&self, p: [f64; 2]) -> Vec3 {
        let p = DVec2::from(p) - self.offset;
        let key = [(p.x / self.cell).floor() as i64, (p.y / self.cell).floor() as i64];
        // Primero el triángulo que lo contiene; si no, el más cercano,
        // buscando en anillos de celdas cada vez más grandes.
        let mut best: Option<(f64, u32)> = None;
        for ring in 0..64i64 {
            for x in key[0] - ring..=key[0] + ring {
                for y in key[1] - ring..=key[1] + ring {
                    if (x - key[0]).abs() != ring && (y - key[1]).abs() != ring {
                        continue; // ya visto en un anillo anterior
                    }
                    for &t in self.grid.get(&[x, y]).into_iter().flatten() {
                        let d = distance_to_triangle(p, self.corners(t));
                        if best.is_none_or(|(b, _)| d < b) {
                            best = Some((d, t));
                        }
                    }
                }
            }
            // Todo lo que queda fuera de este anillo está al menos a
            // `ring × cell` del punto.
            if best.is_some_and(|(d, _)| d <= ring as f64 * self.cell) {
                break;
            }
        }
        let t = match best {
            Some((_, t)) => t,
            None => (0..self.mesh.triangles.len() as u32)
                .min_by(|&a, &b| {
                    distance_to_triangle(p, self.corners(a)).total_cmp(&distance_to_triangle(p, self.corners(b)))
                })
                .expect("malla sin triángulos"),
        };
        let [u, v, w] = barycentric(p, self.corners(t));
        let [a, b, c] = self.mesh.triangles[t as usize].map(|i| self.positions[i as usize]);
        a * u as f32 + b * v as f32 + c * w as f32
    }
}

/// Coordenadas baricéntricas (sin recortar) de `p` en el triángulo.
fn barycentric(p: DVec2, [a, b, c]: [DVec2; 3]) -> [f64; 3] {
    let (e1, e2, q) = (b - a, c - a, p - a);
    let det = e1.perp_dot(e2);
    if det.abs() < 1e-12 {
        return [1.0, 0.0, 0.0];
    }
    let v = q.perp_dot(e2) / det;
    let w = e1.perp_dot(q) / det;
    [1.0 - v - w, v, w]
}

fn distance_to_triangle(p: DVec2, t: [DVec2; 3]) -> f64 {
    let [u, v, w] = barycentric(p, t);
    if u >= 0.0 && v >= 0.0 && w >= 0.0 {
        return 0.0;
    }
    (0..3)
        .map(|k| {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let ab = b - a;
            let s = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            p.distance(a + ab * s)
        })
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pattern::{Pattern, Point};

    fn rect(w: f64, h: f64, size: f64) -> Mesh2D {
        let mut p = Pattern::new("t");
        let id = p.add_rect("r", Point::ZERO, w, h);
        let options = mesher::MeshOptions { target_edge_length: size, ..Default::default() };
        mesher::mesh_piece(p.piece(id).unwrap(), &options).unwrap()
    }

    /// Un rectángulo doblado como un tejado (pliegue en x = 100): los puntos
    /// de otra malla del mismo rectángulo caen sobre el mismo tejado, y los
    /// de una pieza más grande lo continúan.
    #[test]
    fn follows_the_old_shape() {
        let fold = |p: [f64; 2]| {
            let x = p[0] * 1e-3;
            let (along, up) = if x < 0.1 { (x, x) } else { (0.1 + (x - 0.1), 0.2 - x) };
            Vec3::new(along as f32, up as f32, (p[1] * 1e-3) as f32)
        };
        let old = rect(200.0, 100.0, 20.0);
        let positions: Vec<Vec3> = old.positions.iter().map(|&p| fold(p)).collect();
        let remap = Remap::new(&old, &positions, [0.0, 0.0]);

        let new = rect(200.0, 100.0, 9.0);
        for &p in &new.positions {
            let got = remap.at(p);
            assert!(got.distance(fold(p)) < 0.012, "{p:?}: {got} en vez de {}", fold(p));
        }
        // Fuera de la malla vieja: se extrapola por el último plano.
        let got = remap.at([250.0, 50.0]);
        assert!(got.distance(fold([250.0, 50.0])) < 1e-4, "{got}");
        // Con desplazamiento: el patrón nuevo está 500 mm a la derecha.
        let moved = Remap::new(&old, &positions, [500.0, 0.0]);
        assert!(moved.at([550.0, 50.0]).distance(fold([50.0, 50.0])) < 1e-4);
    }
}
