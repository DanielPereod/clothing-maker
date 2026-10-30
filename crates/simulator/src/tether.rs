//! Tirantes: el límite de estiramiento de las telas tejidas.
//!
//! Con una sola pasada por subpaso, las restricciones de distancia de la
//! malla no pueden ser más rígidas de lo que deja el paso de tiempo: una
//! cadena de aristas cede tanto más cuanto más fina es la malla y más peso
//! cuelga de ella (un asa que sostiene todo el bolso se alarga un 30 %).
//! Una tela tejida, en cambio, apenas cede: sus hilos se estiran un poco y
//! luego se bloquean.
//!
//! Los tirantes unen partículas lejanas de la misma pieza con una distancia
//! máxima: la que hay entre ellas en el patrón más lo que la tela puede
//! estirar. Como la tela no puede alejar dos puntos más de lo que mide el
//! camino entre ellos por la tela, un tirante no hace nada mientras la tela
//! se dobla o se arruga (los puntos se acercan); solo frena cuando se
//! estira. Van a varias escalas (cada una unas 2,5 veces más larga que la
//! anterior) y en tres direcciones, así que una corrección llega de un
//! extremo de la pieza al otro en un solo subpaso.
//!
//! Un tirante solo une dos puntos si el segmento entre ellos va por dentro
//! de la pieza: no puede saltar un agujero ni atajar por una muesca.

use std::collections::{HashMap, HashSet};

use glam::{Vec2, Vec3};

/// Longitud del tirante más corto, en aristas de la malla.
const FIRST: f32 = 2.5;
/// Cuántas veces más largo es cada nivel de tirantes que el anterior.
const GROWTH: f32 = 2.5;
/// Hasta dónde puede caer el punto de destino de un tirante del vértice
/// más cercano, en aristas.
const SNAP: f32 = 0.6;

/// Distancia máxima entre dos partículas de la misma pieza.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tether {
    pub a: u32,
    pub b: u32,
    pub max_length: f32,
}

/// Tirantes de una pieza. `rest` son sus vértices en el patrón (m, en el
/// plano XY), `triangles` sus triángulos (índices locales) y `base` el
/// índice de su primera partícula en la tela. Cada tirante deja estirar
/// `max_strain` (0,01 = un 1 %).
pub(crate) fn build(rest: &[Vec3], triangles: &[[u32; 3]], base: u32, max_strain: f32) -> Vec<Tether> {
    let p: Vec<Vec2> = rest.iter().map(|r| r.truncate()).collect();
    let mut edges: HashMap<(u32, u32), u32> = HashMap::with_capacity(triangles.len() * 3);
    for t in triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    if edges.is_empty() {
        return Vec::new();
    }
    let edge = edges.keys().map(|&(a, b)| p[a as usize].distance(p[b as usize])).sum::<f32>() / edges.len() as f32;
    let (lo, hi) = p
        .iter()
        .fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(lo, hi), &q| (lo.min(q), hi.max(q)));
    let extent = lo.distance(hi);
    if edge <= 0.0 || extent < FIRST * edge {
        return Vec::new();
    }

    let grid = Grid::new(lo, edge);
    // Solo los vértices que son tela (los de algún triángulo).
    let mut used = vec![false; p.len()];
    triangles.iter().flatten().for_each(|&v| used[v as usize] = true);
    let mut vertices: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, &q) in p.iter().enumerate().filter(|&(i, _)| used[i]) {
        vertices.entry(grid.cell(q)).or_default().push(i as u32);
    }
    let mut cells_of_triangles: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, t) in triangles.iter().enumerate() {
        let [a, b, c] = t.map(|v| p[v as usize]);
        for cell in grid.cells(a.min(b).min(c), a.max(b).max(c)) {
            cells_of_triangles.entry(cell).or_default().push(i as u32);
        }
    }
    let boundary: Vec<[u32; 2]> = edges.iter().filter(|&(_, &n)| n == 1).map(|(&(a, b), _)| [a, b]).collect();
    let mut cells_of_boundary: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, &[a, b]) in boundary.iter().enumerate() {
        let (a, b) = (p[a as usize], p[b as usize]);
        for cell in grid.cells(a.min(b), a.max(b)) {
            cells_of_boundary.entry(cell).or_default().push(i as u32);
        }
    }

    let nearest = |target: Vec2| -> Option<u32> {
        let (cx, cy) = grid.cell(target);
        let mut best = None;
        let mut best_d = SNAP * edge;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &v in vertices.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    let d = p[v as usize].distance(target);
                    if d < best_d {
                        (best, best_d) = (Some(v), d);
                    }
                }
            }
        }
        best
    };
    let in_mesh = |q: Vec2| {
        cells_of_triangles.get(&grid.cell(q)).into_iter().flatten().any(|&t| {
            let [a, b, c] = triangles[t as usize].map(|v| p[v as usize]);
            let (d1, d2, d3) = (cross(b - a, q - a), cross(c - b, q - b), cross(a - c, q - c));
            let tolerance = -1e-6 * edge * edge;
            (d1 >= tolerance && d2 >= tolerance && d3 >= tolerance)
                || (d1 <= -tolerance && d2 <= -tolerance && d3 <= -tolerance)
        })
    };
    let inside = |i: u32, j: u32| {
        let (a, b) = (p[i as usize], p[j as usize]);
        // Puntos a lo largo del segmento, cada media arista, y que no corte
        // el borde: así tampoco se escapa por un vértice del borde.
        let samples = (2.0 * a.distance(b) / edge).ceil() as usize;
        if !(1..samples).all(|k| in_mesh(a.lerp(b, k as f32 / samples as f32))) {
            return false;
        }
        let mut seen = HashSet::new();
        for cell in grid.cells(a.min(b), a.max(b)) {
            for &e in cells_of_boundary.get(&cell).into_iter().flatten() {
                let [c, d] = boundary[e as usize];
                if !seen.insert(e) || [c, d].iter().any(|v| *v == i || *v == j) {
                    continue;
                }
                if crosses(a, b, p[c as usize], p[d as usize]) {
                    return false;
                }
            }
        }
        true
    };

    let mut pairs = HashSet::new();
    let mut tethers = Vec::new();
    let mut radius = FIRST * edge;
    let mut level = 0;
    while radius < extent {
        for i in (0..p.len() as u32).filter(|&i| used[i as usize]) {
            for k in 0..3 {
                let angle = level as f32 * 0.6 + k as f32 * std::f32::consts::FRAC_PI_3;
                let target = p[i as usize] + Vec2::from_angle(angle) * radius;
                let Some(j) = nearest(target) else { continue };
                if j == i || !pairs.insert((i.min(j), i.max(j))) || !inside(i, j) {
                    continue;
                }
                let length = p[i as usize].distance(p[j as usize]);
                tethers.push(Tether { a: base + i, b: base + j, max_length: length * (1.0 + max_strain) });
            }
        }
        radius *= GROWTH;
        level += 1;
    }
    tethers
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Si los segmentos `a b` y `c d` se cortan en un punto de dentro de los dos.
fn crosses(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let (d1, d2) = (cross(b - a, c - a), cross(b - a, d - a));
    let (d3, d4) = (cross(d - c, a - c), cross(d - c, b - c));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Rejilla uniforme sobre el plano del patrón.
struct Grid {
    origin: Vec2,
    cell: f32,
}

impl Grid {
    fn new(origin: Vec2, cell: f32) -> Self {
        Self { origin, cell }
    }

    fn cell(&self, q: Vec2) -> (i32, i32) {
        let c = ((q - self.origin) / self.cell).floor();
        (c.x as i32, c.y as i32)
    }

    /// Las celdas que toca la caja `lo`–`hi`.
    fn cells(&self, lo: Vec2, hi: Vec2) -> impl Iterator<Item = (i32, i32)> {
        let ((x0, y0), (x1, y1)) = (self.cell(lo), self.cell(hi));
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una rejilla de `n × n` vértices separados `step` m, sin los
    /// triángulos que caen en `hole` (en celdas).
    fn grid_mesh(n: u32, step: f32, hole: impl Fn(u32, u32) -> bool) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let rest = (0..n * n).map(|i| Vec3::new((i % n) as f32 * step, (i / n) as f32 * step, 0.0)).collect();
        let mut triangles = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                if hole(x, y) {
                    continue;
                }
                let v = y * n + x;
                triangles.push([v, v + 1, v + n + 1]);
                triangles.push([v, v + n + 1, v + n]);
            }
        }
        (rest, triangles)
    }

    #[test]
    fn tethers_span_the_piece_at_several_scales() {
        let (rest, triangles) = grid_mesh(21, 0.01, |_, _| false);
        let tethers = build(&rest, &triangles, 100, 0.02);
        assert!(tethers.len() > rest.len(), "{} tirantes", tethers.len());
        let lengths: Vec<f32> = tethers.iter().map(|t| t.max_length / 1.02).collect();
        assert!(lengths.iter().any(|&l| l < 0.03) && lengths.iter().any(|&l| l > 0.1));
        for t in &tethers {
            let (a, b) = ((t.a - 100) as usize, (t.b - 100) as usize);
            assert!((rest[a].distance(rest[b]) * 1.02 - t.max_length).abs() < 1e-6);
        }
    }

    /// Una pieza en U: ningún tirante ataja por el hueco.
    #[test]
    fn tethers_stay_inside_the_piece() {
        let gap = |x: u32, y: u32| (8..12).contains(&x) && y >= 5;
        let (rest, triangles) = grid_mesh(21, 0.01, gap);
        let tethers = build(&rest, &triangles, 0, 0.02);
        assert!(!tethers.is_empty());
        for t in &tethers {
            let (a, b) = (rest[t.a as usize], rest[t.b as usize]);
            // Ningún punto del segmento cae en el hueco.
            for k in 1..20 {
                let q = a.lerp(b, k as f32 / 20.0) / 0.01;
                let in_gap = q.x > 8.0 + 1e-3 && q.x < 12.0 - 1e-3 && q.y > 5.0 + 1e-3;
                assert!(!in_gap, "tirante por el hueco: {a} → {b}");
            }
        }
    }
}
