//! Campo de distancia con signo (SDF) de una malla cerrada, precalculado en
//! una rejilla 3D. Así la colisión contra un cuerpo de decenas de miles de
//! triángulos cuesta lo mismo que contra una esfera: una consulta en la
//! rejilla con interpolación trilineal.
//!
//! Para calcular la rejilla se busca el triángulo más cercano a cada nodo
//! con un BVH, y el signo sale de la pseudonormal ponderada por ángulos de
//! la parte del triángulo más cercana (cara, arista o vértice), que es
//! exacta para mallas cerradas (Bærentzen y Aanæs, 2005).

use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;

/// Distancia con signo a una malla, muestreada en una rejilla regular.
#[derive(Clone)]
pub struct MeshSdf {
    origin: Vec3,
    cell: f32,
    dims: [usize; 3],
    values: Vec<f32>,
}

impl std::fmt::Debug for MeshSdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshSdf")
            .field("origin", &self.origin)
            .field("cell", &self.cell)
            .field("dims", &self.dims)
            .finish()
    }
}

impl MeshSdf {
    /// Calcula el campo de una malla cerrada con nodos cada `cell` metros
    /// y `margin` metros de rejilla alrededor de la malla. Fuera de la
    /// rejilla la distancia es aproximada (pero siempre positiva).
    pub fn from_mesh(positions: &[Vec3], triangles: &[[u32; 3]], cell: f32, margin: f32) -> Self {
        let mesh = ClosestMesh::new(positions, triangles);
        let (lo, hi) = positions
            .iter()
            .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(a, b), p| (a.min(*p), b.max(*p)));
        let origin = lo - Vec3::splat(margin);
        let size = hi - lo + Vec3::splat(2.0 * margin);
        let grid = |cell: f32, exact: &(dyn Fn(Vec3) -> Option<f32> + Sync)| {
            let dims = [0, 1, 2].map(|k| (size[k] / cell).ceil() as usize + 2);
            let (nx, ny) = (dims[0], dims[1]);
            let values = (0..dims[0] * dims[1] * dims[2])
                .into_par_iter()
                .with_min_len(256)
                .map(|i| {
                    let p = origin + Vec3::new((i % nx) as f32, ((i / nx) % ny) as f32, (i / (nx * ny)) as f32) * cell;
                    exact(p).unwrap_or_else(|| mesh.signed_distance(p))
                })
                .collect();
            Self { origin, cell, dims, values }
        };
        // Primero una rejilla gruesa; en la fina solo se calcula de verdad
        // cerca de la superficie, y lejos basta con interpolar la gruesa.
        let coarse_cell = cell * 4.0;
        let coarse = grid(coarse_cell, &|_| None);
        let band = 2.0 * coarse_cell;
        grid(cell, &|p| {
            let (d, _) = coarse.sample(p);
            (d.abs() > band).then_some(d)
        })
    }

    /// Esquinas de la rejilla.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let [nx, ny, nz] = self.dims;
        (self.origin, self.origin + Vec3::new((nx - 1) as f32, (ny - 1) as f32, (nz - 1) as f32) * self.cell)
    }

    fn at(&self, x: usize, y: usize, z: usize) -> f32 {
        self.values[x + self.dims[0] * (y + self.dims[1] * z)]
    }

    /// Distancia con signo (negativa dentro) y dirección hacia fuera.
    pub fn signed_distance(&self, p: Vec3) -> (f32, Vec3) {
        let (lo, hi) = self.bounds();
        let inside = p.clamp(lo, hi);
        let outside = p - inside;
        if outside != Vec3::ZERO {
            // Fuera de la rejilla: la distancia en el borde más lo que falta
            // hasta él. El borde está a `margin` de la malla, así que basta.
            let (d, _) = self.sample(inside);
            let len = outside.length();
            return (d + len, outside / len);
        }
        self.sample(p)
    }

    /// Interpolación trilineal y su gradiente.
    fn sample(&self, p: Vec3) -> (f32, Vec3) {
        let g = (p - self.origin) / self.cell;
        let max = [self.dims[0] - 2, self.dims[1] - 2, self.dims[2] - 2];
        let i = [0, 1, 2].map(|k| (g[k].floor().max(0.0) as usize).min(max[k]));
        let t = Vec3::new(g.x - i[0] as f32, g.y - i[1] as f32, g.z - i[2] as f32);
        let c = |dx, dy, dz| self.at(i[0] + dx, i[1] + dy, i[2] + dz);
        let (c000, c100, c010, c110) = (c(0, 0, 0), c(1, 0, 0), c(0, 1, 0), c(1, 1, 0));
        let (c001, c101, c011, c111) = (c(0, 0, 1), c(1, 0, 1), c(0, 1, 1), c(1, 1, 1));
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        // Interpolación en x para las cuatro aristas, luego en y, luego en z.
        let (x00, x10) = (lerp(c000, c100, t.x), lerp(c010, c110, t.x));
        let (x01, x11) = (lerp(c001, c101, t.x), lerp(c011, c111, t.x));
        let (y0, y1) = (lerp(x00, x10, t.y), lerp(x01, x11, t.y));
        let d = lerp(y0, y1, t.z);
        let dx = lerp(lerp(c100 - c000, c110 - c010, t.y), lerp(c101 - c001, c111 - c011, t.y), t.z);
        let dy = lerp(x10 - x00, x11 - x01, t.z);
        let dz = y1 - y0;
        (d, Vec3::new(dx, dy, dz).normalize_or(Vec3::Y))
    }
}

/// Malla con BVH para buscar el punto más cercano.
struct ClosestMesh {
    positions: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    face_normals: Vec<Vec3>,
    /// Pseudonormales de cada vértice y de cada arista (por par de vértices
    /// ordenado).
    vertex_normals: Vec<Vec3>,
    edge_normals: HashMap<(u32, u32), Vec3>,
    nodes: Vec<BvhNode>,
    /// Triángulos reordenados según las hojas del BVH.
    order: Vec<u32>,
}

#[derive(Clone, Copy)]
struct BvhNode {
    lo: Vec3,
    hi: Vec3,
    /// Hoja: `count > 0` triángulos desde `first` en `order`. Interno:
    /// hijos en `first` y `first + 1`.
    first: u32,
    count: u32,
}

const LEAF_SIZE: usize = 4;

impl ClosestMesh {
    fn new(positions: &[Vec3], triangles: &[[u32; 3]]) -> Self {
        // Con las normales hacia dentro el signo saldría al revés: se mira
        // el volumen y, si es negativo, se dan la vuelta los triángulos.
        let volume: f32 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| positions[i as usize]);
                a.dot(b.cross(c))
            })
            .sum();
        let triangles: Vec<[u32; 3]> =
            triangles.iter().map(|&[a, b, c]| if volume < 0.0 { [a, c, b] } else { [a, b, c] }).collect();

        let mut face_normals = Vec::with_capacity(triangles.len());
        let mut vertex_normals = vec![Vec3::ZERO; positions.len()];
        let mut edge_normals: HashMap<(u32, u32), Vec3> = HashMap::with_capacity(triangles.len() * 3 / 2);
        for t in &triangles {
            let p = t.map(|i| positions[i as usize]);
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            face_normals.push(n);
            for k in 0..3 {
                let (a, b, c) = (p[k], p[(k + 1) % 3], p[(k + 2) % 3]);
                let angle = (b - a).angle_between(c - a);
                if angle.is_finite() {
                    vertex_normals[t[k] as usize] += n * angle;
                }
                let (i, j) = (t[k], t[(k + 1) % 3]);
                *edge_normals.entry((i.min(j), i.max(j))).or_default() += n;
            }
        }

        let mut mesh = Self {
            positions: positions.to_vec(),
            triangles,
            face_normals,
            vertex_normals,
            edge_normals,
            nodes: Vec::new(),
            order: Vec::new(),
        };
        mesh.build_bvh();
        mesh
    }

    fn triangle_bounds(&self, t: u32) -> (Vec3, Vec3) {
        let p = self.triangles[t as usize].map(|i| self.positions[i as usize]);
        (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]))
    }

    fn build_bvh(&mut self) {
        let centers: Vec<Vec3> = (0..self.triangles.len() as u32)
            .map(|t| {
                let (lo, hi) = self.triangle_bounds(t);
                (lo + hi) * 0.5
            })
            .collect();
        self.order = (0..self.triangles.len() as u32).collect();
        self.nodes = vec![BvhNode { lo: Vec3::ZERO, hi: Vec3::ZERO, first: 0, count: 0 }];
        // Pila de (nodo, inicio, fin) por dividir.
        let mut stack = vec![(0usize, 0usize, self.order.len())];
        while let Some((node, start, end)) = stack.pop() {
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for &t in &self.order[start..end] {
                let (a, b) = self.triangle_bounds(t);
                lo = lo.min(a);
                hi = hi.max(b);
            }
            self.nodes[node].lo = lo;
            self.nodes[node].hi = hi;
            if end - start <= LEAF_SIZE {
                self.nodes[node].first = start as u32;
                self.nodes[node].count = (end - start) as u32;
                continue;
            }
            // Divide por la mediana del eje más largo.
            let axis =
                (hi - lo).to_array().iter().enumerate().fold(0, |m, (k, &v)| if v > (hi - lo)[m] { k } else { m });
            let mid = (start + end) / 2;
            self.order[start..end].select_nth_unstable_by(mid - start, |&a, &b| {
                centers[a as usize][axis].total_cmp(&centers[b as usize][axis])
            });
            let child = self.nodes.len();
            self.nodes.push(BvhNode { lo, hi, first: 0, count: 0 });
            self.nodes.push(BvhNode { lo, hi, first: 0, count: 0 });
            self.nodes[node].first = child as u32;
            stack.push((child, start, mid));
            stack.push((child + 1, mid, end));
        }
    }

    fn signed_distance(&self, p: Vec3) -> f32 {
        let mut best = (f32::INFINITY, 0u32, Feature::Face, p);
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = self.nodes[n as usize];
            if box_distance2(p, node.lo, node.hi) >= best.0 {
                continue;
            }
            if node.count > 0 {
                for &t in &self.order[node.first as usize..(node.first + node.count) as usize] {
                    let tri = self.triangles[t as usize].map(|i| self.positions[i as usize]);
                    let (q, feature) = closest_on_triangle(p, tri);
                    let d2 = p.distance_squared(q);
                    if d2 < best.0 {
                        best = (d2, t, feature, q);
                    }
                }
            } else {
                // Primero el hijo más cercano, para podar antes.
                let (a, b) = (node.first, node.first + 1);
                let da = box_distance2(p, self.nodes[a as usize].lo, self.nodes[a as usize].hi);
                let db = box_distance2(p, self.nodes[b as usize].lo, self.nodes[b as usize].hi);
                if da < db {
                    stack.extend([b, a]);
                } else {
                    stack.extend([a, b]);
                }
            }
        }
        let (d2, t, feature, q) = best;
        let tri = self.triangles[t as usize];
        let normal = match feature {
            Feature::Face => self.face_normals[t as usize],
            Feature::Edge(k) => {
                let (i, j) = (tri[k], tri[(k + 1) % 3]);
                self.edge_normals[&(i.min(j), i.max(j))]
            }
            Feature::Vertex(k) => self.vertex_normals[tri[k] as usize],
        };
        let d = d2.sqrt();
        if (p - q).dot(normal) < 0.0 { -d } else { d }
    }
}

/// Qué parte del triángulo es la más cercana.
#[derive(Clone, Copy)]
enum Feature {
    Face,
    /// Arista de `k` a `k + 1`.
    Edge(usize),
    Vertex(usize),
}

fn box_distance2(p: Vec3, lo: Vec3, hi: Vec3) -> f32 {
    (lo - p).max(p - hi).max(Vec3::ZERO).length_squared()
}

/// Punto más cercano de un triángulo (Ericson, *Real-Time Collision
/// Detection*, 5.1.5).
fn closest_on_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> (Vec3, Feature) {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, Feature::Vertex(0));
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, Feature::Vertex(1));
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return (a + ab * (d1 / (d1 - d3)), Feature::Edge(0));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, Feature::Vertex(2));
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return (a + ac * (d2 / (d2 - d6)), Feature::Edge(2));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return (b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6))), Feature::Edge(1));
    }
    let denom = 1.0 / (va + vb + vc);
    (a + ab * (vb * denom) + ac * (vc * denom), Feature::Face)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Icosaedro subdividido proyectado sobre una esfera.
    fn sphere_mesh(radius: f32, subdivisions: usize) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let t = (1.0 + 5f32.sqrt()) / 2.0;
        let mut positions: Vec<Vec3> = [
            (-1.0, t, 0.0),
            (1.0, t, 0.0),
            (-1.0, -t, 0.0),
            (1.0, -t, 0.0),
            (0.0, -1.0, t),
            (0.0, 1.0, t),
            (0.0, -1.0, -t),
            (0.0, 1.0, -t),
            (t, 0.0, -1.0),
            (t, 0.0, 1.0),
            (-t, 0.0, -1.0),
            (-t, 0.0, 1.0),
        ]
        .iter()
        .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
        .collect();
        let mut triangles: Vec<[u32; 3]> = vec![
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        for _ in 0..subdivisions {
            let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
            let mut midpoint = |a: u32, b: u32, positions: &mut Vec<Vec3>| {
                *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
                    positions.push(((positions[a as usize] + positions[b as usize]) / 2.0).normalize());
                    positions.len() as u32 - 1
                })
            };
            triangles = triangles
                .iter()
                .flat_map(|&[a, b, c]| {
                    let ab = midpoint(a, b, &mut positions);
                    let bc = midpoint(b, c, &mut positions);
                    let ca = midpoint(c, a, &mut positions);
                    [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]
                })
                .collect();
        }
        (positions.iter().map(|p| *p * radius).collect(), triangles)
    }

    #[test]
    fn sphere_distances() {
        let (positions, triangles) = sphere_mesh(0.2, 3);
        // También con los triángulos al revés: el signo no debe cambiar.
        let flipped: Vec<[u32; 3]> = triangles.iter().map(|&[a, b, c]| [a, c, b]).collect();
        for tris in [triangles, flipped] {
            let sdf = MeshSdf::from_mesh(&positions, &tris, 0.01, 0.05);
            // Cerca de la superficie (hasta 8 cm, la banda exacta) la distancia
            // es la de verdad; más lejos se interpola la rejilla gruesa.
            for p in [
                Vec3::new(0.1, 0.05, 0.0),
                Vec3::new(0.0, 0.25, 0.0),
                Vec3::new(0.15, -0.15, 0.1),
                Vec3::new(-0.05, 0.0, 0.17),
            ] {
                let (d, n) = sdf.signed_distance(p);
                let exact = p.length() - 0.2;
                // El icosaedro subdividido queda un poco por dentro de la esfera.
                assert!((d - exact).abs() < 0.006, "{p}: {d} frente a {exact}");
                assert!(n.dot(p.normalize()) > 0.95, "{p}: normal {n}");
            }
            let (d, _) = sdf.signed_distance(Vec3::ZERO);
            assert!(d < -0.15, "centro: {d}");
            // Lejos de la rejilla sigue siendo positiva y aproximada.
            let (d, n) = sdf.signed_distance(Vec3::new(2.0, 0.0, 0.0));
            assert!((d - 1.8).abs() < 0.02 && n.x > 0.99, "{d} {n}");
        }
    }
}
