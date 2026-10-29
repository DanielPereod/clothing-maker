//! Auto-colisión: que la tela no se atraviese a sí misma (un panel con
//! otro, una manga con el cuerpo o las dos caras de una bolsa).
//!
//! Se comprueba cada partícula contra los triángulos cercanos (punto contra
//! triángulo, con un hash espacial). Al principio de cada paso se buscan los
//! pares que pueden tocarse durante el paso y se apunta en qué lado del
//! triángulo está la partícula; en cada subpaso, si la partícula está más
//! cerca que la separación mínima, o ya ha pasado al otro lado, se empuja de
//! vuelta a su lado y el triángulo en sentido contrario.
//!
//! No se comprueban los triángulos cercanos a la partícula: a menos de dos
//! aristas por la malla, o de dos aristas de distancia en el patrón si son
//! de la misma pieza. Tocan a la partícula por construcción, y en un
//! pliegue cerrado se pelearían con ella y la harían vibrar. Las costuras
//! cuentan como si las partículas cosidas fueran la misma, para que los
//! paneles no se repelan en la unión (ni en las esquinas donde se juntan
//! tres piezas).

use std::collections::HashMap;

use glam::Vec3;
use rayon::prelude::*;

/// Margen (en coordenadas baricéntricas) para seguir contando un punto que
/// se proyecta justo fuera del triángulo: el vecino puede no estar en la
/// lista si no comparte celda.
const BARY_MARGIN: f32 = 0.05;

/// En el borde libre de la tela (el que no está cosido) el triángulo se
/// alarga hacia fuera esta fracción, como un faldón: así, una partícula que
/// pasa rozando el borde de otra capa no puede rodearlo y colarse al otro
/// lado (dos bocas de bolsa que se cruzan).
const SKIRT: f32 = 0.6;

/// Lo que se separa como mucho (m) en un subpaso un par que está demasiado
/// cerca pero sin cruzarse.
const MAX_PUSH: f32 = 2e-4;

/// Un posible contacto entre una partícula y un triángulo durante un paso.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Contact {
    particle: u32,
    triangle: u32,
    /// +1 o -1: el lado del triángulo (según su normal) en el que estaba la
    /// partícula al empezar el paso.
    side: f32,
    /// Distancia mínima (m) entre la partícula y el triángulo.
    separation: f32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SelfCollision {
    /// Hay que recalcular `root` y `ring` (nuevas piezas o costuras).
    pub dirty: bool,
    /// Representante de cada partícula: las partículas cosidas comparten uno.
    root: Vec<u32>,
    /// Vecinos (por representante, ordenados) de cada representante.
    ring: Vec<Vec<u32>>,
    /// Posición en reposo (m, en el plano del patrón) y pieza de cada
    /// partícula.
    rest: Vec<(Vec3, u32)>,
    /// Por debajo de esta distancia en reposo (m), dos puntos de la misma
    /// pieza no chocan.
    rest_exclusion: f32,
    pub contacts: Vec<Contact>,
    /// Margen hacia fuera de cada lado de cada triángulo (el lado `k` es el
    /// opuesto al vértice `k`).
    margins: Vec<[f32; 3]>,
    /// Caja (ampliada) de cada triángulo en este paso.
    boxes: Vec<TriBox>,
    /// Hash espacial de los triángulos: inicio de cada celda en `entries`.
    start: Vec<u32>,
    entries: Vec<u32>,
    /// Arista más larga de la malla en reposo (m).
    max_edge: f32,
    /// Arista media de la malla en reposo (m).
    mean_edge: f32,
}

pub(crate) struct Topology<'a> {
    pub particles: usize,
    pub triangles: &'a [[u32; 3]],
    pub seams: &'a [[u32; 2]],
    pub rest_edges: &'a [f32],
    /// Posición en reposo y pieza de cada partícula.
    pub rest: &'a [(Vec3, u32)],
}

impl SelfCollision {
    pub fn rebuild(&mut self, topo: Topology) {
        // Unión de las partículas cosidas.
        let mut parent: Vec<u32> = (0..topo.particles as u32).collect();
        fn find(parent: &mut [u32], mut i: u32) -> u32 {
            while parent[i as usize] != i {
                parent[i as usize] = parent[parent[i as usize] as usize];
                i = parent[i as usize];
            }
            i
        }
        for &[a, b] in topo.seams {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
        self.root = (0..topo.particles as u32).map(|i| find(&mut parent, i)).collect();
        self.ring = vec![Vec::new(); topo.particles];
        for t in topo.triangles {
            let r = t.map(|i| self.root[i as usize]);
            for k in 0..3 {
                let (a, b) = (r[k], r[(k + 1) % 3]);
                self.ring[a as usize].push(b);
                self.ring[b as usize].push(a);
            }
        }
        for ring in &mut self.ring {
            ring.sort_unstable();
            ring.dedup();
        }
        // Lados de un solo triángulo (contando las costuras como unión).
        let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
        let side = |t: &[u32; 3], k: usize, root: &[u32]| {
            let (a, b) = (root[t[(k + 1) % 3] as usize], root[t[(k + 2) % 3] as usize]);
            (a.min(b), a.max(b))
        };
        for t in topo.triangles {
            for k in 0..3 {
                *uses.entry(side(t, k, &self.root)).or_default() += 1;
            }
        }
        self.margins = topo
            .triangles
            .iter()
            .map(|t| std::array::from_fn(|k| if uses[&side(t, k, &self.root)] == 1 { SKIRT } else { BARY_MARGIN }))
            .collect();
        self.max_edge = topo.rest_edges.iter().copied().fold(0.0, f32::max);
        self.mean_edge = if topo.rest_edges.is_empty() {
            0.0
        } else {
            topo.rest_edges.iter().sum::<f32>() / topo.rest_edges.len() as f32
        };
        self.rest = topo.rest.to_vec();
        self.rest_exclusion = 2.0 * self.max_edge;
        self.dirty = false;
    }

    /// `true` si la partícula `p` está junto al triángulo `t` en el patrón:
    /// a dos aristas o menos de uno de sus vértices (contando las costuras),
    /// o cerca en la misma pieza.
    pub fn adjacent(&self, p: u32, t: &[u32; 3]) -> bool {
        let rp = self.root[p as usize];
        let ring = &self.ring[rp as usize];
        let (rest, piece) = self.rest[p as usize];
        let limit = self.rest_exclusion * self.rest_exclusion;
        t.iter().any(|&v| {
            let rv = self.root[v as usize];
            let (rest_v, piece_v) = self.rest[v as usize];
            (piece == piece_v && rest.distance_squared(rest_v) < limit)
                || rv == rp
                || ring.binary_search(&rv).is_ok()
                || share_any(ring, &self.ring[rv as usize])
        })
    }

    /// Separación mínima para una partícula de grosor `thickness`: dos
    /// grosores, pero menos de media arista, para no chocar con los
    /// triángulos que hay justo después de sus vecinos.
    fn separation(&self, thickness: f32) -> f32 {
        (2.0 * thickness).min(0.5 * self.mean_edge)
    }

    /// Busca los pares partícula-triángulo que pueden tocarse antes de la
    /// próxima búsqueda. `travel` es lo que puede moverse como mucho cada
    /// partícula hasta entonces.
    pub fn gather(
        &mut self,
        positions: &[Vec3],
        travel: &[f32],
        inv_mass: &[f32],
        thickness: &[f32],
        triangles: &[[u32; 3]],
    ) {
        self.contacts.clear();
        if triangles.is_empty() || self.max_edge <= 0.0 {
            return;
        }
        let max_sep = thickness.iter().map(|&t| self.separation(t)).fold(0.0, f32::max);

        // Hash espacial de celdas del tamaño de la arista más larga.
        let inv_cell = 1.0 / self.max_edge;
        let key = |p: Vec3| {
            let f = p * inv_cell;
            // `floor` sin llamar a la función de la biblioteca, que aquí es
            // lo que más cuesta.
            let i = f.as_ivec3();
            i - glam::IVec3::select(f.cmplt(i.as_vec3()), glam::IVec3::ONE, glam::IVec3::ZERO)
        };
        let table = (triangles.len() * 2).next_power_of_two();
        let hash = move |x: i32, y: i32, z: i32| {
            (x.wrapping_mul(92_837_111) ^ y.wrapping_mul(689_287_499) ^ z.wrapping_mul(283_923_481)) as u32 as usize
                & (table - 1)
        };

        // Caja de cada triángulo, ampliada con lo que pueden moverse sus
        // vértices y la separación, y las celdas que ocupa.
        self.boxes.clear();
        self.boxes.extend(triangles.iter().map(|t| {
            let [a, b, c] = t.map(|v| positions[v as usize]);
            let grow = Vec3::splat(max_sep + t.iter().map(|&v| travel[v as usize]).fold(0.0, f32::max));
            let (lo, hi) = (a.min(b).min(c) - grow, a.max(b).max(c) + grow);
            TriBox { lo, hi, cell_lo: key(lo), cell_hi: key(hi) }
        }));

        // Ordenación por recuento: `start[h]..start[h + 1]` son los
        // triángulos de las celdas con hash `h`.
        self.start.clear();
        self.start.resize(table + 1, 0);
        for b in &self.boxes {
            for_cells(b.cell_lo, b.cell_hi, |x, y, z| self.start[hash(x, y, z)] += 1);
        }
        let mut sum = 0;
        for s in &mut self.start {
            sum += *s;
            *s = sum;
        }
        self.entries.clear();
        self.entries.resize(sum as usize, 0);
        for (i, b) in self.boxes.iter().enumerate() {
            for_cells(b.cell_lo, b.cell_hi, |x, y, z| {
                let h = hash(x, y, z);
                self.start[h] -= 1;
                self.entries[self.start[h] as usize] = i as u32;
            });
        }

        let this = &*self;
        let find = |p: usize, seen: &mut Vec<u32>, out: &mut Vec<Contact>| {
            let x = positions[p];
            let reach = Vec3::splat(travel[p]);
            let (lo, hi) = (x - reach, x + reach);
            let (cell_lo, cell_hi) = (key(lo), key(hi));
            seen.clear();
            for_cells(cell_lo, cell_hi, |cx, cy, cz| {
                let h = hash(cx, cy, cz);
                for &ti in &this.entries[this.start[h] as usize..this.start[h + 1] as usize] {
                    let b = &this.boxes[ti as usize];
                    // Cada par se cuenta solo en la primera celda que
                    // comparten las dos cajas.
                    if (lo.cmple(b.hi) & hi.cmpge(b.lo)).all() && glam::IVec3::new(cx, cy, cz) == cell_lo.max(b.cell_lo)
                    {
                        seen.push(ti);
                    }
                }
            });
            let separation = this.separation(thickness[p]);
            for &ti in seen.iter() {
                let t = &triangles[ti as usize];
                if inv_mass[p] + t.iter().map(|&v| inv_mass[v as usize]).sum::<f32>() == 0.0 {
                    continue;
                }
                let tri = t.map(|v| positions[v as usize]);
                let Some((_, d, _)) = project(x, tri, this.margins[ti as usize]) else { continue };
                let reach = separation + travel[p] + t.iter().map(|&v| travel[v as usize]).fold(0.0, f32::max);
                if d.abs() < reach && !this.adjacent(p as u32, t) {
                    let side = if d >= 0.0 { 1.0 } else { -1.0 };
                    out.push(Contact { particle: p as u32, triangle: ti, side, separation });
                }
            }
        };
        let chunk = |range: std::ops::Range<usize>| {
            let (mut seen, mut out) = (Vec::new(), Vec::new());
            for p in range {
                find(p, &mut seen, &mut out);
            }
            out
        };
        const CHUNK: usize = 256;
        let n = positions.len();
        self.contacts = if n >= 4 * CHUNK {
            (0..n.div_ceil(CHUNK))
                .into_par_iter()
                .flat_map_iter(|k| chunk(k * CHUNK..((k + 1) * CHUNK).min(n)))
                .collect()
        } else {
            chunk(0..n)
        };
    }

    /// Separa los pares que están demasiado cerca o se han cruzado.
    /// `prev` son las posiciones al empezar el subpaso.
    pub fn solve(&self, positions: &mut [Vec3], prev: &[Vec3], inv_mass: &[f32], triangles: &[[u32; 3]]) {
        for c in &self.contacts {
            let t = triangles[c.triangle as usize];
            let p = c.particle as usize;
            let tri = t.map(|v| positions[v as usize]);
            let Some((bary, d, n)) = project(positions[p], tri, self.margins[c.triangle as usize]) else { continue };
            let constraint = c.side * d - c.separation;
            if constraint >= 0.0 {
                continue;
            }
            let w = t.map(|v| inv_mass[v as usize]);
            let wp = inv_mass[p];
            let denom = wp + bary[0] * bary[0] * w[0] + bary[1] * bary[1] * w[1] + bary[2] * bary[2] * w[2];
            if denom <= 0.0 {
                continue;
            }
            // Si ya ha cruzado, vuelve entera a su lado. Si no, se deshace lo
            // que se haya acercado en este subpaso, y lo que ya estuviera
            // demasiado cerca se separa poco a poco, para no meter energía.
            let depth = if c.side * d < 0.0 {
                -constraint
            } else {
                let [a0, b0, c0] = t.map(|v| prev[v as usize]);
                let n0 = (b0 - a0).cross(c0 - a0).normalize_or_zero();
                let approach = (c.side * ((prev[p] - a0).dot(n0) - d)).max(0.0);
                (-constraint).min(approach + MAX_PUSH)
            };
            let push = n * (c.side * depth / denom);
            positions[p] += push * wp;
            for k in 0..3 {
                positions[t[k] as usize] -= push * (w[k] * bary[k]);
            }
        }
    }
}

/// Proyecta `p` sobre el plano del triángulo. Devuelve las coordenadas
/// baricéntricas de la proyección (recortadas al triángulo), la distancia
/// con signo al plano y la normal, o `None` si la proyección cae fuera del
/// triángulo o el triángulo está degenerado.
fn project(p: Vec3, [a, b, c]: [Vec3; 3], margin: [f32; 3]) -> Option<([f32; 3], f32, Vec3)> {
    let (e1, e2) = (b - a, c - a);
    let n = e1.cross(e2);
    let area2 = n.length_squared();
    if area2 < 1e-16 {
        return None;
    }
    let n = n / area2.sqrt();
    let ap = p - a;
    let d = ap.dot(n);
    let q = ap - n * d;
    // Coordenadas baricéntricas de q en (e1, e2).
    let (d11, d12, d22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let (dq1, dq2) = (q.dot(e1), q.dot(e2));
    let det = d11 * d22 - d12 * d12;
    if det.abs() < 1e-20 {
        return None;
    }
    let v = (d22 * dq1 - d12 * dq2) / det;
    let w = (d11 * dq2 - d12 * dq1) / det;
    let u = 1.0 - v - w;
    if u < -margin[0] || v < -margin[1] || w < -margin[2] {
        return None;
    }
    let (u, v, w) = (u.max(0.0), v.max(0.0), w.max(0.0));
    let s = u + v + w;
    Some(([u / s, v / s, w / s], d, n))
}

/// `true` si el segmento `p`-`q` atraviesa el triángulo.
pub(crate) fn segment_hits_triangle(p: Vec3, q: Vec3, [a, b, c]: [Vec3; 3]) -> bool {
    let n = (b - a).cross(c - a);
    let (dp, dq) = ((p - a).dot(n), (q - a).dot(n));
    if dp * dq > 0.0 || dp == dq {
        return false;
    }
    let x = p + (q - p) * (dp / (dp - dq));
    let inside = |u: Vec3, v: Vec3| (v - u).cross(x - u).dot(n) >= 0.0;
    inside(a, b) && inside(b, c) && inside(c, a)
}

#[derive(Clone, Copy, Debug)]
struct TriBox {
    lo: Vec3,
    hi: Vec3,
    cell_lo: glam::IVec3,
    cell_hi: glam::IVec3,
}

fn for_cells(lo: glam::IVec3, hi: glam::IVec3, mut f: impl FnMut(i32, i32, i32)) {
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                f(x, y, z);
            }
        }
    }
}

/// `true` si dos listas ordenadas tienen algún elemento en común.
fn share_any(a: &[u32], b: &[u32]) -> bool {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}
