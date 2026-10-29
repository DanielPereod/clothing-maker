//! Simulación de tela con XPBD (Extended Position Based Dynamics).
//!
//! Todas las piezas de una prenda viven en un solo sistema de partículas
//! ([`Cloth`]): cada pieza aporta sus vértices y triángulos con un
//! desplazamiento, y una costura es solo otra restricción entre partículas
//! de piezas distintas.
//!
//! Restricciones por pieza:
//! - **Estiramiento:** distancia en cada arista de la malla, con la longitud
//!   en reposo sacada del patrón 2D.
//! - **Flexión:** distancia entre los dos vértices opuestos de cada par de
//!   triángulos vecinos. Es más barata y más estable que la de ángulo
//!   diedro y, con una flexibilidad (*compliance*) alta, se comporta igual
//!   para tela normal.
//! - **Colisión** con una lista de [`Collider`]s (suelo, esferas, cajas y
//!   mallas cerradas como el maniquí, vía [`MeshSdf`]), con grosor y
//!   rozamiento.
//! - **Costuras:** distancia entre los puntos emparejados de dos bordes. La
//!   distancia de reposo empieza siendo la de la colocación inicial y baja a
//!   velocidad constante hasta cero ([`SimParams::sew_speed`]), así que las
//!   piezas se acercan poco a poco en vez de saltar. Mientras se cose no hay
//!   gravedad, para que las piezas no se caigan antes de estar unidas.
//!
//! Las restricciones se reparten en «colores» (grupos sin partículas en
//! común) para resolver cada grupo en paralelo con `rayon`.
//!
//! Unidades: metros, segundos y kilogramos. Las mallas vienen en mm y se
//! convierten al añadirlas.

mod collider;
mod sdf;

pub use collider::Collider;
pub use sdf::MeshSdf;

use glam::Vec3;
use mesher::Mesh2D;
use rayon::prelude::*;
use std::collections::HashMap;

/// Por debajo de este número de elementos no compensa repartir el trabajo
/// entre hilos: despertar a `rayon` cuesta decenas de microsegundos y cada
/// subpaso lo hace una vez por color. Con las mallas habituales (unos miles
/// de partículas) todo va en serie; el paralelismo entra con mallas finas.
const PAR_MIN_LEN: usize = 16_384;

/// Propiedades físicas de un tejido.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fabric {
    /// Gramaje en kg/m² (una camiseta de algodón ronda 0,15).
    pub density: f32,
    /// Flexibilidad al estirar (m/N). 0 = inextensible.
    pub stretch_compliance: f32,
    /// Flexibilidad al doblar (m/N). Más alto = cae más blando.
    pub bend_compliance: f32,
    /// Coeficiente de rozamiento contra los colisionadores (0..1).
    pub friction: f32,
    /// Distancia mínima (m) que se mantiene a los colisionadores.
    pub thickness: f32,
}

impl Fabric {
    pub const COTTON: Fabric =
        Fabric { density: 0.15, stretch_compliance: 1e-7, bend_compliance: 2e-3, friction: 0.5, thickness: 0.003 };
    pub const SILK: Fabric =
        Fabric { density: 0.06, stretch_compliance: 2e-7, bend_compliance: 2e-2, friction: 0.2, thickness: 0.002 };
    pub const DENIM: Fabric =
        Fabric { density: 0.40, stretch_compliance: 2e-8, bend_compliance: 1e-4, friction: 0.6, thickness: 0.004 };
}

impl Default for Fabric {
    fn default() -> Self {
        Fabric::COTTON
    }
}

/// Restricción de distancia entre dos partículas.
#[derive(Clone, Copy, Debug)]
pub struct DistanceConstraint {
    pub a: u32,
    pub b: u32,
    pub rest_length: f32,
    /// Inversa de la rigidez (m/N). 0 = inextensible.
    pub compliance: f32,
}

/// Las partículas y triángulos que aporta una pieza a la tela.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieceRange {
    pub first_particle: u32,
    pub particle_count: u32,
    pub first_triangle: u32,
    pub triangle_count: u32,
}

/// Parámetros de la simulación.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimParams {
    pub gravity: Vec3,
    pub substeps: u32,
    /// Amortiguación de la velocidad por segundo (0..1), como el rozamiento
    /// con el aire. Sin ella los pliegues que cuelgan tardan mucho en parar.
    pub damping: f32,
    /// Velocidad (m/s) a la que se cierran las costuras.
    pub sew_speed: f32,
}

impl Default for SimParams {
    fn default() -> Self {
        Self { gravity: Vec3::new(0.0, -9.81, 0.0), substeps: 20, damping: 0.5, sew_speed: 0.4 }
    }
}

/// Una tela formada por partículas (puede contener varias piezas).
#[derive(Clone, Debug, Default)]
pub struct Cloth {
    pub positions: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    /// Inversa de la masa; 0 = partícula fija.
    pub inv_mass: Vec<f32>,
    /// Triángulos de todas las piezas, con índices globales. Su normal
    /// `(b - a) × (c - a)` apunta hacia el derecho de la tela.
    pub triangles: Vec<[u32; 3]>,
    pub pieces: Vec<PieceRange>,
    friction: Vec<f32>,
    thickness: Vec<f32>,
    prev_positions: Vec<Vec3>,
    /// Restricciones de distancia ordenadas por color.
    constraints: Vec<DistanceConstraint>,
    /// `colors[k]..colors[k + 1]` es el rango del color `k`.
    colors: Vec<usize>,
    /// Restricciones añadidas desde el último coloreado.
    pending: Vec<DistanceConstraint>,
    /// Costuras: pocas comparadas con la tela, así que van en serie.
    seams: Vec<DistanceConstraint>,
    scratch: Vec<(Vec3, Vec3)>,
    /// Normal y profundidad del contacto de cada partícula en este subpaso.
    contacts: Vec<(Vec3, f32)>,
}

impl Cloth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Crea una tela de una sola pieza, colocada en el plano XY (el eje Y de
    /// la pieza apunta hacia abajo, así que se invierte).
    pub fn from_mesh(mesh: &Mesh2D, fabric: &Fabric) -> Self {
        let mut cloth = Self::new();
        cloth.add_piece(mesh, fabric, |p| Vec3::new(p[0] as f32 * 1e-3, -p[1] as f32 * 1e-3, 0.0), Vec3::Z);
        cloth
    }

    /// Añade una pieza. `place` convierte un punto del patrón (mm) en su
    /// posición 3D inicial (m); `front` es hacia dónde debe mirar el derecho
    /// de la tela en esa posición. La forma en reposo siempre sale del
    /// patrón 2D, no de la posición inicial.
    pub fn add_piece(
        &mut self,
        mesh: &Mesh2D,
        fabric: &Fabric,
        place: impl Fn([f64; 2]) -> Vec3,
        front: Vec3,
    ) -> PieceRange {
        let base = self.positions.len() as u32;
        let first_triangle = self.triangles.len() as u32;
        let rest: Vec<Vec3> = mesh.positions.iter().map(|p| Vec3::new(p[0] as f32, p[1] as f32, 0.0) * 1e-3).collect();
        let placed: Vec<Vec3> = mesh.positions.iter().map(|&p| place(p)).collect();

        // Orienta los triángulos para que su normal mire hacia `front`.
        let flip = mesh
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| placed[i as usize]);
                (b - a).cross(c - a).dot(front)
            })
            .sum::<f32>()
            < 0.0;
        let triangles: Vec<[u32; 3]> =
            mesh.triangles.iter().map(|&[a, b, c]| if flip { [a, c, b] } else { [a, b, c] }).collect();

        // Reparte la masa de cada triángulo entre sus vértices.
        let mut mass = vec![0.0f32; rest.len()];
        for t in &triangles {
            let [a, b, c] = t.map(|i| rest[i as usize]);
            let m = 0.5 * (b - a).cross(c - a).length() * fabric.density;
            for &i in t {
                mass[i as usize] += m / 3.0;
            }
        }

        let distance = |a: u32, b: u32, compliance| DistanceConstraint {
            a: a + base,
            b: b + base,
            rest_length: rest[a as usize].distance(rest[b as usize]),
            compliance,
        };
        for [a, b] in mesh.edges() {
            self.pending.push(distance(a, b, fabric.stretch_compliance));
        }
        for [a, b] in bending_pairs(&triangles) {
            self.pending.push(distance(a, b, fabric.bend_compliance));
        }

        let n = rest.len();
        self.positions.extend(&placed);
        self.prev_positions.extend(&placed);
        self.velocities.extend(std::iter::repeat_n(Vec3::ZERO, n));
        self.inv_mass.extend(mass.iter().map(|&m| if m > 0.0 { 1.0 / m } else { 0.0 }));
        self.friction.extend(std::iter::repeat_n(fabric.friction, n));
        self.thickness.extend(std::iter::repeat_n(fabric.thickness, n));
        self.triangles.extend(triangles.iter().map(|t| t.map(|i| i + base)));
        let range = PieceRange {
            first_particle: base,
            particle_count: n as u32,
            first_triangle,
            triangle_count: triangles.len() as u32,
        };
        self.pieces.push(range);
        range
    }

    /// Fija una partícula en su sitio.
    pub fn pin(&mut self, i: usize) {
        self.inv_mass[i] = 0.0;
        self.velocities[i] = Vec3::ZERO;
    }

    pub fn constraint_count(&self) -> usize {
        self.constraints.len() + self.pending.len()
    }

    /// Une dos partículas con una costura. Empieza a la distancia a la que
    /// estén ahora y se va cerrando al simular.
    pub fn add_seam(&mut self, a: u32, b: u32) {
        if a == b {
            return;
        }
        let rest_length = self.positions[a as usize].distance(self.positions[b as usize]);
        self.seams.push(DistanceConstraint { a, b, rest_length, compliance: 0.0 });
    }

    pub fn seam_count(&self) -> usize {
        self.seams.len()
    }

    /// `true` mientras quede alguna costura por cerrar.
    pub fn is_sewing(&self) -> bool {
        self.seams.iter().any(|c| c.rest_length > 0.0)
    }

    /// Separación máxima (m) que queda entre dos puntos cosidos.
    pub fn max_seam_gap(&self) -> f32 {
        self.seams
            .iter()
            .map(|c| self.positions[c.a as usize].distance(self.positions[c.b as usize]))
            .fold(0.0, f32::max)
    }

    /// Normales por vértice, hacia el derecho de la tela.
    pub fn vertex_normals(&self) -> Vec<Vec3> {
        vertex_normals(&self.positions, &self.triangles)
    }

    /// Avanza la simulación `dt` segundos.
    pub fn step(&mut self, dt: f32, params: &SimParams, colliders: &[Collider]) {
        if !self.pending.is_empty() {
            self.recolor();
        }
        let substeps = params.substeps.max(1);
        let h = dt / substeps as f32;
        let damping = (1.0 - params.damping.clamp(0.0, 1.0)).powf(h);
        let parallel = self.positions.len() >= PAR_MIN_LEN;
        self.contacts.resize(self.positions.len(), (Vec3::ZERO, 0.0));
        for _ in 0..substeps {
            let gravity = if self.is_sewing() { Vec3::ZERO } else { params.gravity };
            let shrink = params.sew_speed.max(0.0) * h;
            for c in &mut self.seams {
                c.rest_length = (c.rest_length - shrink).max(0.0);
            }
            self.prev_positions.copy_from_slice(&self.positions);
            let integrate = |(x, v, w): (&mut Vec3, &mut Vec3, &f32)| {
                if *w > 0.0 {
                    *v += gravity * h;
                    *x += *v * h;
                }
            };
            if parallel {
                (&mut self.positions, &mut self.velocities, &self.inv_mass).into_par_iter().for_each(integrate);
            } else {
                self.positions
                    .iter_mut()
                    .zip(&mut self.velocities)
                    .zip(&self.inv_mass)
                    .for_each(|((x, v), w)| integrate((x, v, w)));
            }
            // Colisiones antes que la tela, para que la tela tenga la última
            // palabra (si no, se estira al pasar por un borde). Puede volver
            // a meter un poco las partículas, pero siempre dentro del grosor.
            self.solve_collisions(colliders, parallel);
            self.solve_distances(h);
            self.solve_seams(h);
            self.apply_friction(parallel);
            let update = |(v, x, prev): (&mut Vec3, &Vec3, &Vec3)| *v = (*x - *prev) / h * damping;
            if parallel {
                (&mut self.velocities, &self.positions, &self.prev_positions).into_par_iter().for_each(update);
            } else {
                self.velocities
                    .iter_mut()
                    .zip(&self.positions)
                    .zip(&self.prev_positions)
                    .for_each(|((v, x), p)| update((v, x, p)));
            }
        }
    }

    /// Reparte las restricciones en grupos sin partículas compartidas
    /// (coloreado voraz de grafo).
    fn recolor(&mut self) {
        let mut all = std::mem::take(&mut self.constraints);
        all.append(&mut self.pending);
        // Máscara de colores usados por cada partícula. Si se agotan los 63
        // primeros, el resto va al último grupo, que se resuelve en serie.
        const SERIAL: u32 = 63;
        let mut used = vec![0u64; self.positions.len()];
        let mut color_of = Vec::with_capacity(all.len());
        for c in &all {
            let (a, b) = (c.a as usize, c.b as usize);
            let free = !(used[a] | used[b]);
            let color = free.trailing_zeros().min(SERIAL);
            if color < SERIAL {
                used[a] |= 1 << color;
                used[b] |= 1 << color;
            }
            color_of.push(color);
        }
        let mut order: Vec<usize> = (0..all.len()).collect();
        order.sort_by_key(|&i| color_of[i]);
        self.constraints = order.iter().map(|&i| all[i]).collect();
        self.colors.clear();
        let mut k = 0;
        for (i, &idx) in order.iter().enumerate() {
            while k <= color_of[idx] as usize {
                self.colors.push(i);
                k += 1;
            }
        }
        while self.colors.len() <= SERIAL as usize + 1 {
            self.colors.push(self.constraints.len());
        }
        self.scratch = vec![(Vec3::ZERO, Vec3::ZERO); self.constraints.len()];
    }

    fn solve_distances(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        let groups = self.colors.len() - 1;
        for k in 0..groups {
            let range = self.colors[k]..self.colors[k + 1];
            if range.is_empty() {
                continue;
            }
            if k == groups - 1 || range.len() < PAR_MIN_LEN {
                // Último grupo (puede compartir partículas) o grupo pequeño: en serie.
                for c in &self.constraints[range] {
                    let (da, db) = distance_correction(c, &self.positions, &self.inv_mass, inv_h2);
                    self.positions[c.a as usize] += da;
                    self.positions[c.b as usize] += db;
                }
                continue;
            }
            let (positions, inv_mass) = (&self.positions, &self.inv_mass);
            self.constraints[range.clone()]
                .par_iter()
                .zip(&mut self.scratch[range.clone()])
                .with_min_len(PAR_MIN_LEN / 2)
                .for_each(|(c, out)| *out = distance_correction(c, positions, inv_mass, inv_h2));
            for (c, &(da, db)) in self.constraints[range.clone()].iter().zip(&self.scratch[range]) {
                self.positions[c.a as usize] += da;
                self.positions[c.b as usize] += db;
            }
        }
    }

    fn solve_seams(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for c in &self.seams {
            let (da, db) = distance_correction(c, &self.positions, &self.inv_mass, inv_h2);
            self.positions[c.a as usize] += da;
            self.positions[c.b as usize] += db;
        }
    }

    /// Saca las partículas de los colisionadores y apunta, para cada una,
    /// la normal y cuánto se ha empujado (para el rozamiento).
    fn solve_collisions(&mut self, colliders: &[Collider], parallel: bool) {
        let collide = |(x, contact, w, thickness): (&mut Vec3, &mut (Vec3, f32), &f32, &f32)| {
            *contact = (Vec3::ZERO, 0.0);
            if *w == 0.0 {
                return;
            }
            for collider in colliders {
                let (d, n) = collider.signed_distance(*x);
                let depth = thickness - d;
                if depth > 0.0 {
                    *x += n * depth;
                    if depth > contact.1 {
                        *contact = (n, depth);
                    }
                }
            }
        };
        if parallel {
            (&mut self.positions, &mut self.contacts, &self.inv_mass, &self.thickness)
                .into_par_iter()
                .for_each(collide);
        } else {
            for (i, contact) in self.contacts.iter_mut().enumerate() {
                collide((&mut self.positions[i], contact, &self.inv_mass[i], &self.thickness[i]));
            }
        }
    }

    /// Rozamiento de Coulomb en posiciones: frena el desplazamiento
    /// tangencial de todo el subpaso (también el que han causado las
    /// restricciones de la tela) hasta `rozamiento × empuje normal`. Va
    /// después de la tela para que una pieza apoyada no resbale poco a poco
    /// cuando una parte cuelga.
    fn apply_friction(&mut self, parallel: bool) {
        let friction = |(x, prev, contact, mu): (&mut Vec3, &Vec3, &(Vec3, f32), &f32)| {
            let (n, depth) = *contact;
            if depth <= 0.0 {
                return;
            }
            let dx = *x - *prev;
            let tangential = dx - n * dx.dot(n);
            let len = tangential.length();
            if len < mu * depth {
                *x -= tangential;
            } else if len > 1e-12 {
                *x -= tangential * (mu * depth / len);
            }
        };
        if parallel {
            (&mut self.positions, &self.prev_positions, &self.contacts, &self.friction)
                .into_par_iter()
                .for_each(friction);
        } else {
            for (i, x) in self.positions.iter_mut().enumerate() {
                friction((x, &self.prev_positions[i], &self.contacts[i], &self.friction[i]));
            }
        }
    }
}

/// Normales por vértice: media de las de sus triángulos, ponderada por área.
pub fn vertex_normals(positions: &[Vec3], triangles: &[[u32; 3]]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for t in triangles {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        let n = (b - a).cross(c - a);
        for &i in t {
            normals[i as usize] += n;
        }
    }
    normals.iter().map(|n| n.normalize_or_zero()).collect()
}

fn distance_correction(c: &DistanceConstraint, positions: &[Vec3], inv_mass: &[f32], inv_h2: f32) -> (Vec3, Vec3) {
    let (a, b) = (c.a as usize, c.b as usize);
    let (wa, wb) = (inv_mass[a], inv_mass[b]);
    let w = wa + wb;
    if w == 0.0 {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let d = positions[a] - positions[b];
    let len = d.length();
    if len < 1e-9 {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let alpha = c.compliance * inv_h2;
    let lambda = -(len - c.rest_length) / (w + alpha);
    let corr = d / len * lambda;
    (corr * wa, -corr * wb)
}

/// Para cada arista interior, los dos vértices opuestos de sus triángulos.
fn bending_pairs(triangles: &[[u32; 3]]) -> Vec<[u32; 2]> {
    let mut opposite: HashMap<(u32, u32), u32> = HashMap::with_capacity(triangles.len() * 3);
    let mut pairs = Vec::new();
    for t in triangles {
        for k in 0..3 {
            let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let key = (a.min(b), a.max(b));
            if let Some(other) = opposite.remove(&key) {
                pairs.push([other, c]);
            } else {
                opposite.insert(key, c);
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use pattern::{Pattern, Point};

    fn square_mesh(size: f64) -> Mesh2D {
        let mut p = Pattern::new("t");
        let id = p.add_rect("r", Point::ZERO, size, size);
        mesher::mesh_piece(p.piece(id).unwrap(), &Default::default()).unwrap()
    }

    #[test]
    fn hanging_cloth_stays_stable() {
        let mesh = square_mesh(200.0);
        let mut cloth = Cloth::from_mesh(&mesh, &Fabric { stretch_compliance: 0.0, ..Fabric::COTTON });
        // Cuelga de las dos esquinas superiores.
        for (i, pos) in mesh.positions.iter().enumerate() {
            if pos[1] == 0.0 && (pos[0] == 0.0 || pos[0] == 200.0) {
                cloth.pin(i);
            }
        }
        for _ in 0..120 {
            cloth.step(1.0 / 60.0, &SimParams::default(), &[]);
        }
        assert!(cloth.positions.iter().all(|p| p.is_finite()));
        // La tela cae pero no se estira de forma absurda (< 10 %).
        let lowest = cloth.positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        assert!(lowest < -0.15 && lowest > -0.31, "punto más bajo: {lowest}");
    }

    #[test]
    fn coloring_has_no_shared_particles() {
        let mut cloth = Cloth::from_mesh(&square_mesh(300.0), &Fabric::COTTON);
        let total = cloth.constraint_count();
        cloth.recolor();
        assert_eq!(cloth.constraints.len(), total);
        assert_eq!(cloth.colors.len(), 65);
        for k in 0..cloth.colors.len() - 2 {
            let mut seen = std::collections::HashSet::new();
            for c in &cloth.constraints[cloth.colors[k]..cloth.colors[k + 1]] {
                assert!(seen.insert(c.a) && seen.insert(c.b), "el color {k} comparte partículas");
            }
        }
        let serial = cloth.colors[64] - cloth.colors[63];
        assert_eq!(serial, 0, "no debería hacer falta el grupo en serie");
    }

    #[test]
    fn triangles_face_front() {
        let mesh = square_mesh(100.0);
        let mut cloth = Cloth::new();
        cloth.add_piece(&mesh, &Fabric::COTTON, |p| Vec3::new(p[0] as f32, 0.5, p[1] as f32) * 1e-3, Vec3::Y);
        assert!(cloth.vertex_normals().iter().all(|n| n.dot(Vec3::Y) > 0.99));
    }

    /// Dos cuadrados de 20 cm, uno frente a otro a 30 cm, cosidos por un
    /// borde: el borde se cierra y, al acabar de coser, caen al suelo.
    #[test]
    fn seam_closes_and_then_gravity_acts() {
        let mesh = square_mesh(200.0);
        let mut cloth = Cloth::new();
        let place = |z: f32| move |p: [f64; 2]| Vec3::new(p[0] as f32 * 1e-3, 1.0 - p[1] as f32 * 1e-3, z);
        let a = cloth.add_piece(&mesh, &Fabric::COTTON, place(0.15), Vec3::Z);
        let b = cloth.add_piece(&mesh, &Fabric::COTTON, place(-0.15), -Vec3::Z);
        // Borde izquierdo de los dos (x = 0), punto a punto.
        let left: Vec<u32> =
            (0..mesh.positions.len() as u32).filter(|&i| mesh.positions[i as usize][0] == 0.0).collect();
        assert!(left.len() > 5);
        for &i in &left {
            cloth.add_seam(a.first_particle + i, b.first_particle + i);
        }
        assert_eq!(cloth.seam_count(), left.len());
        assert!(cloth.is_sewing());
        let start_y = min_y(&cloth);
        // 0,3 m a 0,4 m/s: en 0,75 s está cerrada.
        for _ in 0..50 {
            cloth.step(1.0 / 60.0, &SimParams::default(), &[Collider::ground(0.0)]);
        }
        assert!(!cloth.is_sewing());
        assert!(cloth.max_seam_gap() < 1e-3, "la costura sigue abierta: {}", cloth.max_seam_gap());
        // Sin gravedad mientras cosía: no se ha caído.
        assert!(min_y(&cloth) > start_y - 0.05, "ha caído mientras cosía: {}", min_y(&cloth));
        for _ in 0..240 {
            cloth.step(1.0 / 60.0, &SimParams::default(), &[Collider::ground(0.0)]);
        }
        assert!(cloth.positions.iter().all(|p| p.is_finite()));
        assert!(min_y(&cloth) < 0.01, "no ha llegado al suelo: {}", min_y(&cloth));
        assert!(cloth.max_seam_gap() < 2e-3, "la costura se ha abierto: {}", cloth.max_seam_gap());
    }

    /// Deja caer un cuadrado de tela de 80 cm desde 50 cm sobre el suelo y
    /// `obstacle` durante 8 s, y comprueba que nada atraviesa nada.
    fn drop_square(obstacle: Collider, fabric: &Fabric) -> Cloth {
        let mesh = square_mesh(800.0);
        let mut cloth = Cloth::new();
        cloth.add_piece(&mesh, fabric, |p| Vec3::new(p[0] as f32 * 1e-3 - 0.4, 0.5, p[1] as f32 * 1e-3 - 0.4), Vec3::Y);
        let colliders = [Collider::ground(0.0), obstacle];
        for _ in 0..480 {
            cloth.step(1.0 / 60.0, &SimParams::default(), &colliders);
        }
        assert!(cloth.positions.iter().all(|p| p.is_finite()));
        for p in &cloth.positions {
            for c in &colliders {
                let (d, _) = c.signed_distance(*p);
                // El grosor es de 3 mm; en las esquinas la tela puede
                // apretar un poco más, pero sin llegar a cruzar.
                assert!(d > -0.001, "partícula dentro de un colisionador: {p} ({d})");
            }
        }
        cloth
    }

    fn max_y(cloth: &Cloth) -> f32 {
        cloth.positions.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max)
    }

    fn min_y(cloth: &Cloth) -> f32 {
        cloth.positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min)
    }

    /// Estiramiento de cada arista de la malla (1 = longitud en reposo), de
    /// menor a mayor.
    fn stretch_ratios(cloth: &Cloth, fabric: &Fabric) -> Vec<f32> {
        let mut ratios: Vec<f32> = cloth
            .constraints
            .iter()
            .filter(|c| c.compliance == fabric.stretch_compliance)
            .map(|c| cloth.positions[c.a as usize].distance(cloth.positions[c.b as usize]) / c.rest_length)
            .collect();
        ratios.sort_by(f32::total_cmp);
        ratios
    }

    /// Un mantel sobre una mesa de 40 × 40 cm: se apoya arriba, los bordes
    /// cuelgan, apenas se estira y acaba quieto.
    #[test]
    fn drapes_over_table() {
        let fabric = Fabric::COTTON;
        let table = Collider::Cuboid { center: Vec3::new(0.0, 0.2, 0.0), half_extents: Vec3::splat(0.2) };
        let cloth = drop_square(table, &fabric);
        assert!((max_y(&cloth) - 0.403).abs() < 0.01, "altura máxima: {}", max_y(&cloth));
        assert!(min_y(&cloth) < 0.3, "punto más bajo: {}", min_y(&cloth));
        // En las esquinas de la mesa la tela tiene que estirarse algo para
        // doblarse; en el resto, casi nada.
        let ratios = stretch_ratios(&cloth, &fabric);
        let at = |q: f32| ratios[((ratios.len() - 1) as f32 * q) as usize];
        assert!(at(0.5) < 1.01, "mediana del estiramiento: {}", at(0.5));
        assert!(at(0.99) < 1.06, "percentil 99 del estiramiento: {}", at(0.99));
        assert!(at(1.0) < 1.2, "estiramiento máximo: {}", at(1.0));
        let speed = cloth.velocities.iter().map(|v| v.length()).fold(0.0, f32::max);
        assert!(speed < 0.05, "velocidad máxima: {speed}");
    }

    /// Sobre una esfera (que una tela plana no puede envolver sin arrugarse)
    /// basta con que se apoye arriba y llegue al suelo.
    #[test]
    fn drapes_over_sphere() {
        let ball = Collider::Sphere { center: Vec3::new(0.0, 0.2, 0.0), radius: 0.2 };
        let cloth = drop_square(ball, &Fabric::COTTON);
        assert!((max_y(&cloth) - 0.403).abs() < 0.01, "altura máxima: {}", max_y(&cloth));
        assert!(min_y(&cloth) < 0.01, "punto más bajo: {}", min_y(&cloth));
    }

    /// Una tira de 45 cm sobre el borde de una mesa con 10 cm colgando: el
    /// rozamiento la sujeta. Sin rozamiento, se cae.
    #[test]
    fn friction_holds_overhang() {
        let edge_on_table = |friction: f32| {
            let mut p = Pattern::new("t");
            let id = p.add_rect("r", Point::ZERO, 450.0, 300.0);
            let mesh = mesher::mesh_piece(p.piece(id).unwrap(), &Default::default()).unwrap();
            let mut cloth = Cloth::new();
            let fabric = Fabric { friction, ..Fabric::COTTON };
            // La mesa acaba en x = 0.
            cloth.add_piece(
                &mesh,
                &fabric,
                |p| Vec3::new(p[0] as f32 * 1e-3 - 0.35, 0.52, p[1] as f32 * 1e-3 - 0.15),
                Vec3::Y,
            );
            let table =
                Collider::Cuboid { center: Vec3::new(-0.5, 0.25, 0.0), half_extents: Vec3::new(0.5, 0.25, 0.5) };
            for _ in 0..300 {
                cloth.step(1.0 / 60.0, &SimParams::default(), &[Collider::ground(0.0), table.clone()]);
            }
            cloth.positions.iter().map(|p| p.x).fold(f32::INFINITY, f32::min)
        };
        let held = edge_on_table(Fabric::COTTON.friction);
        assert!((held + 0.35).abs() < 0.01, "la tela ha resbalado hasta {held}");
        let slipped = edge_on_table(0.0);
        assert!(slipped > 0.0, "sin rozamiento debería caerse: {slipped}");
    }
}
