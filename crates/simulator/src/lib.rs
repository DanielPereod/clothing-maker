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
//! - **Flexión:** modelo cuadrático de Bergou et al. (*A Quadratic Bending
//!   Model for Inextensible Surfaces*, 2006). Cada arista interior con sus
//!   dos triángulos forma una bisagra; la curvatura de la bisagra es una
//!   combinación lineal fija de sus cuatro vértices (con pesos de
//!   cotangentes sacados del patrón plano), así que resiste desde el primer
//!   milímetro que se dobla y la firmeza de la tela ([`Fabric::bend_stiffness`])
//!   se nota de verdad en cómo cae.
//! - **Colisión** con una lista de [`Collider`]s (suelo, esferas, cajas y
//!   mallas cerradas como el maniquí, vía [`MeshSdf`]), con grosor y
//!   rozamiento.
//! - **Costuras:** distancia entre los puntos emparejados de dos bordes. La
//!   distancia de reposo empieza siendo la de la colocación inicial y baja a
//!   velocidad constante hasta cero ([`SimParams::sew_speed`]), así que las
//!   piezas se acercan poco a poco en vez de saltar. Mientras se cose no hay
//!   gravedad, para que las piezas no se caigan antes de estar unidas.
//! - **Auto-colisión** ([`SimParams::self_collision`]): las partículas no
//!   se acercan a menos de una separación mínima de los triángulos de la
//!   propia tela, así que los paneles no se atraviesan entre sí.
//! - **Presión** ([`SimParams::pressure`]): empuja cada triángulo hacia
//!   fuera, como un relleno, para que una prenda cerrada tenga volumen. Las
//!   bocas sin coser se tapan con una tapa imaginaria (ver `pressure`).
//!
//! Las restricciones se reparten en «colores» (grupos sin partículas en
//! común) para resolver cada grupo en paralelo con `rayon`.
//!
//! Unidades: metros, segundos y kilogramos. Las mallas vienen en mm y se
//! convierten al añadirlas.

mod collider;
mod pressure;
mod remap;
mod sdf;
mod self_collision;
mod tether;

pub use collider::Collider;
pub use remap::Remap;
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

/// Flexibilidad de cada arista de la malla por cada unidad de flexibilidad
/// de la tela ([`Fabric::stretch_compliance`]), con la cizalla incluida:
/// así una tira estira lo que dice su módulo, sea cual sea la malla.
const EDGE_COMPLIANCE: f32 = 1.6;

/// Flexibilidad mínima (m/N) de la cizalla (que los triángulos se deformen
/// dentro del plano). Es la que tenía antes la flexión del algodón, cuando
/// se hacía con estas mismas distancias.
const SHEAR_COMPLIANCE: f32 = 2e-2;
/// La cizalla cede al menos estas veces lo que cede una arista al estirar:
/// en un punto blando, no puede sujetar más que la propia tela.
const SHEAR_RATIO: f32 = 2.0;

/// Tensión (N/m) que tiene que soportar una tela antes de llegar a su
/// límite de estiramiento: cada tela estira como mucho lo que le haría
/// esta tensión.
const STRAIN_LIMIT_TENSION: f32 = 20.0;
/// Lo menos que deja estirar el límite (las telas muy firmes).
const MIN_STRAIN: f32 = 0.01;
/// Las telas que estiran más que esto bajo [`STRAIN_LIMIT_TENSION`] (el
/// punto, los elásticos) no tienen límite: ceden lo que diga su módulo.
const MAX_LIMITED_STRAIN: f32 = 0.1;

/// Pasadas por subpaso sobre las restricciones de distancia (acumulando su
/// multiplicador, como manda XPBD) y las costuras: con una, una tela firme
/// cede ante una blanda mucho más de lo que dice su módulo, y una costura
/// fruncida o que sostiene el peso de la bolsa puede quedar abierta unos
/// milímetros. Cuestan poco al lado de la auto-colisión.
const DISTANCE_ITERATIONS: usize = 2;

/// Cada cuántos subpasos se buscan los pares de la auto-colisión.
const SELF_COLLISION_EVERY: u32 = 5;

/// Propiedades físicas de un tejido.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fabric {
    /// Gramaje en kg/m² (una camiseta de algodón ronda 0,15).
    pub density: f32,
    /// Flexibilidad al estirar (m/N): la inversa del módulo de la tela, o
    /// cuánto se alarga una tira por cada newton por metro de ancho que
    /// tira de ella (una tira de punto de 5 cm con 100 g colgando se
    /// alarga un 10 % con 5·10⁻³; una lona, menos de un 1 %). 0 =
    /// inextensible. Las telas firmes, además, tienen un límite de
    /// estiramiento (ver [`Fabric::max_strain`]).
    pub stretch_compliance: f32,
    /// Rigidez a la flexión (N·m): la firmeza. Más alto = más tieso; una
    /// camiseta de algodón ronda 5·10⁻⁶ y un cuero de bolso 10⁻³.
    pub bend_stiffness: f32,
    /// Coeficiente de rozamiento contra los colisionadores (0..1).
    pub friction: f32,
    /// Distancia mínima (m) que se mantiene a los colisionadores.
    pub thickness: f32,
}

impl Fabric {
    pub const COTTON: Fabric =
        Fabric { density: 0.15, stretch_compliance: 3e-4, bend_stiffness: 5e-6, friction: 0.5, thickness: 0.003 };
    pub const SILK: Fabric =
        Fabric { density: 0.06, stretch_compliance: 7e-4, bend_stiffness: 8e-7, friction: 0.2, thickness: 0.002 };
    pub const DENIM: Fabric =
        Fabric { density: 0.40, stretch_compliance: 4e-5, bend_stiffness: 5e-5, friction: 0.6, thickness: 0.004 };
    pub const LEATHER: Fabric =
        Fabric { density: 0.90, stretch_compliance: 2e-5, bend_stiffness: 1e-3, friction: 0.7, thickness: 0.0015 };
    /// Forro: ligero y muy blando.
    pub const LINING: Fabric =
        Fabric { density: 0.07, stretch_compliance: 6e-4, bend_stiffness: 6e-7, friction: 0.2, thickness: 0.001 };
    /// Punto de algodón: cede mucho y sin límite.
    pub const KNIT: Fabric =
        Fabric { density: 0.18, stretch_compliance: 6e-3, bend_stiffness: 2.5e-6, friction: 0.5, thickness: 0.003 };

    /// Varias capas de tela unidas en una sola lámina (p. ej. cuero por fuera
    /// y forro por dentro), de fuera hacia dentro. Las rigideces se suman
    /// (la flexibilidad al estirar es su inversa), igual que el gramaje y el
    /// grosor; el rozamiento es el de la capa de fuera. Sin capas, devuelve
    /// la tela por defecto.
    pub fn layered(layers: &[Fabric]) -> Fabric {
        let Some(outer) = layers.first() else { return Fabric::default() };
        // Una capa inextensible (flexibilidad 0) hace inextensible el conjunto.
        let combine = |compliance: fn(&Fabric) -> f32| -> f32 {
            if layers.iter().any(|l| compliance(l) == 0.0) {
                0.0
            } else {
                1.0 / layers.iter().map(|l| 1.0 / compliance(l)).sum::<f32>()
            }
        };
        Fabric {
            density: layers.iter().map(|l| l.density).sum(),
            stretch_compliance: combine(|l| l.stretch_compliance),
            bend_stiffness: layers.iter().map(|l| l.bend_stiffness).sum(),
            friction: outer.friction,
            thickness: layers.iter().map(|l| l.thickness).sum(),
        }
    }

    /// Lo más que puede estirar la tela (0,01 = un 1 %), o `None` si no tiene
    /// límite (el punto y los elásticos ceden lo que diga su módulo).
    pub fn max_strain(&self) -> Option<f32> {
        let strain = (STRAIN_LIMIT_TENSION * self.stretch_compliance).max(MIN_STRAIN);
        (strain <= MAX_LIMITED_STRAIN).then_some(strain)
    }

    /// Firmeza al doblar comparada con la del algodón (1 = igual de firme).
    pub fn relative_firmness(&self) -> f32 {
        self.bend_stiffness / Fabric::COTTON.bend_stiffness
    }
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

/// Restricción de flexión sobre una bisagra (dos triángulos que comparten
/// la arista `v[0]`–`v[1]`; `v[2]` y `v[3]` son los vértices opuestos). La
/// curvatura es `Σ k[i]·x[v[i]]`, que vale 0 con la bisagra plana.
#[derive(Clone, Copy, Debug)]
struct BendConstraint {
    v: [u32; 4],
    k: [f32; 4],
    /// Inversa de la rigidez de esta bisagra.
    compliance: f32,
}

/// Un doblez sobre una bisagra (como en [`BendConstraint`]): la distancia
/// entre los vértices opuestos `v[2]` y `v[3]` marca el ángulo, y `side`
/// dice hacia qué lado del triángulo `v[0] v[1] v[3]` (según su normal)
/// tiene que quedar `v[2]`: -1 detrás, 1 delante, 0 da igual.
#[derive(Clone, Copy, Debug)]
struct FoldConstraint {
    v: [u32; 4],
    rest_length: f32,
    compliance: f32,
    side: f32,
}

/// Cómo se dobla la tela por una arista de la malla (ver [`Cloth::crease`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crease {
    /// Ángulo (rad) al que tiende: 0 = plana, π = doblada del todo.
    /// `None` = bisagra libre, que dobla sin resistencia.
    pub angle: Option<f32>,
    /// La solapa se dobla hacia la cara de atrás de los triángulos (la
    /// contraria a su normal, que en una pieza con el derecho fuera es el
    /// revés); si no, hacia la de delante.
    pub toward_back: bool,
    /// Flexibilidad del doblez cuando tiene ángulo.
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
    /// Evitar que la tela se atraviese a sí misma.
    pub self_collision: bool,
    /// Presión interior (Pa) para dar volumen a las prendas cerradas, como
    /// un relleno. 0 = vacía. El peso del algodón equivale a 1,5 Pa, así que
    /// con unas decenas de pascales un cojín ya se hincha.
    pub pressure: f32,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            substeps: 20,
            damping: 0.5,
            sew_speed: 0.4,
            self_collision: true,
            pressure: 0.0,
        }
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
    /// Capa de cada partícula: 0 la de fuera, y más hacia dentro.
    layer: Vec<u8>,
    prev_positions: Vec<Vec3>,
    /// Restricciones de distancia ordenadas por color.
    constraints: Vec<DistanceConstraint>,
    /// `colors[k]..colors[k + 1]` es el rango del color `k`.
    colors: Vec<usize>,
    /// Restricciones añadidas desde el último coloreado.
    pending: Vec<DistanceConstraint>,
    /// Flexión, en serie (cada bisagra toca cuatro partículas).
    bends: Vec<BendConstraint>,
    /// Bisagras de la malla (índices globales), para doblarlas.
    hinges: Vec<[u32; 4]>,
    /// Límite de estiramiento de las telas firmes, en serie.
    tethers: Vec<tether::Tether>,
    /// Hacia dónde empuja el relleno a cada triángulo (ver
    /// [`Cloth::set_fill`]).
    fill: Vec<f32>,
    /// Dobleces planchados, en serie.
    folds: Vec<FoldConstraint>,
    /// Costuras: pocas comparadas con la tela, así que van en serie.
    seams: Vec<DistanceConstraint>,
    /// Turno de montaje de cada costura: solo se cierran las del turno más
    /// bajo que quede abierto.
    seam_order: Vec<u32>,
    scratch: Vec<(Vec3, Vec3, f32)>,
    /// Multiplicador acumulado de cada restricción de distancia en este
    /// subpaso.
    lambdas: Vec<f32>,
    /// Normal y profundidad del contacto de cada partícula en este subpaso.
    contacts: Vec<(Vec3, f32)>,
    /// Aristas de la malla y su longitud en reposo (m).
    mesh_edges: Vec<[u32; 2]>,
    rest_edges: Vec<f32>,
    /// Posición en el patrón (m) y pieza de cada partícula.
    rest: Vec<(Vec3, u32)>,
    self_collision: self_collision::SelfCollision,
    pressure: pressure::Pressure,
    /// Partículas que se fijarán cuando se cierren las costuras.
    pins_after_sewing: Vec<usize>,
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
        let stretch = fabric.stretch_compliance * EDGE_COMPLIANCE;
        let shear = SHEAR_COMPLIANCE.max(stretch * SHEAR_RATIO);
        for [a, b] in mesh.edges() {
            let c = distance(a, b, stretch);
            self.mesh_edges.push([c.a, c.b]);
            self.rest_edges.push(c.rest_length);
            self.pending.push(c);
        }
        for hinge in hinges(&triangles) {
            self.hinges.push(hinge.map(|i| i + base));
            // La distancia entre los vértices opuestos de cada bisagra apenas
            // frena la flexión mientras es suave, pero sujeta la cizalla (que
            // los triángulos se deformen dentro del plano).
            self.pending.push(distance(hinge[2], hinge[3], shear));
            if fabric.bend_stiffness > 0.0
                && let Some(bend) = bend_constraint(hinge.map(|i| rest[i as usize]), fabric.bend_stiffness)
            {
                self.bends.push(BendConstraint { v: hinge.map(|i| i + base), ..bend });
            }
        }

        if let Some(strain) = fabric.max_strain() {
            self.tethers.extend(tether::build(&rest, &triangles, base, strain));
        }

        let n = rest.len();
        self.positions.extend(&placed);
        self.prev_positions.extend(&placed);
        self.velocities.extend(std::iter::repeat_n(Vec3::ZERO, n));
        self.inv_mass.extend(mass.iter().map(|&m| if m > 0.0 { 1.0 / m } else { 0.0 }));
        self.friction.extend(std::iter::repeat_n(fabric.friction, n));
        self.thickness.extend(std::iter::repeat_n(fabric.thickness, n));
        self.layer.extend(std::iter::repeat_n(0, n));
        let piece = self.pieces.len() as u32;
        self.rest.extend(rest.iter().map(|&r| (r, piece)));
        self.triangles.extend(triangles.iter().map(|t| t.map(|i| i + base)));
        self.fill.extend(std::iter::repeat_n(1.0, triangles.len()));
        let range = PieceRange {
            first_particle: base,
            particle_count: n as u32,
            first_triangle,
            triangle_count: triangles.len() as u32,
        };
        self.pieces.push(range);
        self.self_collision.dirty = true;
        self.pressure.dirty = true;
        range
    }

    /// Fija una partícula en su sitio.
    pub fn pin(&mut self, i: usize) {
        self.inv_mass[i] = 0.0;
        self.velocities[i] = Vec3::ZERO;
    }

    /// Fija una partícula donde esté cuando se terminen de cerrar las
    /// costuras (o en el primer paso, si no hay). Sirve para colgar una
    /// prenda: mientras se cose, los puntos fijos no la deforman.
    pub fn pin_after_sewing(&mut self, i: usize) {
        self.pins_after_sewing.push(i);
    }

    pub fn constraint_count(&self) -> usize {
        self.constraints.len() + self.pending.len() + self.bends.len()
    }

    /// Une dos partículas con una costura. Empieza a la distancia a la que
    /// estén ahora y se va cerrando al simular.
    pub fn add_seam(&mut self, a: u32, b: u32) {
        self.add_seam_in_turn(a, b, 0);
    }

    /// Como [`Cloth::add_seam`], pero la costura no empieza a cerrarse hasta
    /// que estén cerradas todas las de turnos anteriores (orden de montaje).
    pub fn add_seam_in_turn(&mut self, a: u32, b: u32, turn: u32) {
        if a == b {
            return;
        }
        let rest_length = self.positions[a as usize].distance(self.positions[b as usize]);
        self.seams.push(DistanceConstraint { a, b, rest_length, compliance: 0.0 });
        self.seam_order.push(turn);
        self.self_collision.dirty = true;
        self.pressure.dirty = true;
    }

    /// Añade una restricción de distancia entre dos partículas cualesquiera,
    /// por ejemplo para que una costura sea más rígida o tienda a quedar
    /// plana o doblada.
    pub fn add_constraint(&mut self, a: u32, b: u32, rest_length: f32, compliance: f32) {
        if a != b {
            self.pending.push(DistanceConstraint { a, b, rest_length, compliance });
        }
    }

    /// Cambia la forma en reposo de una pieza: `rest` recibe la posición en
    /// el patrón (m) de los dos extremos de cada restricción de distancia
    /// de la pieza y devuelve su nueva longitud en reposo (m), o `None`
    /// para dejarla. Sirve, por ejemplo, para fruncir la tela junto a una
    /// costura.
    pub fn reshape(&mut self, piece: &PieceRange, rest: impl Fn(Vec3, Vec3) -> Option<f32>) {
        let range = piece.first_particle..piece.first_particle + piece.particle_count;
        let at = |i: u32| self.rest[i as usize].0;
        let new_length = |a: u32, b: u32| {
            if range.contains(&a) && range.contains(&b) { rest(at(a), at(b)) } else { None }
        };
        for c in self.pending.iter_mut().chain(&mut self.constraints) {
            if let Some(length) = new_length(c.a, c.b) {
                c.rest_length = length;
            }
        }
        for (&[a, b], length) in self.mesh_edges.iter().zip(&mut self.rest_edges) {
            if let Some(new) = new_length(a, b) {
                *length = new;
            }
        }
        // Si la tela queda estirada en reposo (un puño cosido estirado), su
        // límite de estiramiento se alarga con ella.
        for t in &mut self.tethers {
            let flat = at(t.a).distance(at(t.b));
            if let Some(new) = new_length(t.a, t.b)
                && new > flat
            {
                t.max_length *= new / flat;
            }
        }
    }

    /// Dobla la tela por la arista de la malla `a`–`b`: deja de resistirse a
    /// doblar por ahí (una bisagra libre) o, con ángulo, tiende a quedarse
    /// doblada a ese ángulo y hacia ese lado (un doblez planchado).
    /// Devuelve `false` si `a`–`b` no es una arista interior de la malla.
    pub fn crease(&mut self, a: u32, b: u32, crease: Crease) -> bool {
        let on_edge = |h: &[u32; 4]| (h[0], h[1]) == (a.min(b), a.max(b));
        let Some(&hinge) = self.hinges.iter().find(|h| on_edge(h)) else { return false };
        self.bends.retain(|c| !(c.v[0] == hinge[0] && c.v[1] == hinge[1]));
        // La distancia entre los vértices opuestos sujetaba la bisagra plana.
        let wings = (hinge[2].min(hinge[3]), hinge[2].max(hinge[3]));
        let mut all = std::mem::take(&mut self.constraints);
        all.append(&mut self.pending);
        all.retain(|c| (c.a.min(c.b), c.a.max(c.b)) != wings);
        self.pending = all;
        let Some(angle) = crease.angle else { return true };

        let r = hinge.map(|i| self.rest[i as usize].0);
        let axis = (r[1] - r[0]).normalize_or_zero();
        let along = |p: Vec3| (p - r[0]).dot(axis);
        let off = |p: Vec3| (p - r[0] - axis * along(p)).length();
        let (h2, h3) = (off(r[2]), off(r[3]));
        let gap = (along(r[2]) - along(r[3])).powi(2) + h2 * h2 + h3 * h3 + 2.0 * h2 * h3 * angle.cos();
        let thick = self.thickness[hinge[2] as usize] + self.thickness[hinge[3] as usize];
        let rest_length = gap.max(0.0).sqrt().max(thick);
        let side = if crease.toward_back { -1.0 } else { 1.0 };
        // La normal del triángulo v0 v1 v3 tal y como está en la tela.
        let right_way =
            self.triangles.iter().find(|t| [hinge[0], hinge[1], hinge[3]].iter().all(|v| t.contains(v))).is_none_or(
                |t| {
                    let i = t.iter().position(|&v| v == hinge[0]).unwrap_or(0);
                    t[(i + 1) % 3] == hinge[1]
                },
            );
        let side = if right_way { side } else { -side };
        self.folds.push(FoldConstraint { v: hinge, rest_length, compliance: crease.compliance, side });
        true
    }

    /// Suma masa (kg) a una partícula, como el margen de una costura. Las
    /// partículas fijas siguen fijas.
    pub fn add_mass(&mut self, i: usize, mass: f32) {
        let w = self.inv_mass[i];
        if w > 0.0 && mass > 0.0 {
            self.inv_mass[i] = 1.0 / (1.0 / w + mass);
        }
    }

    /// Grosor (m) de una partícula frente a los colisionadores y la propia tela.
    pub fn thickness(&self, i: usize) -> f32 {
        self.thickness[i]
    }

    /// Ensancha una partícula (donde se juntan varias capas de tela). Nunca
    /// la hace más fina de lo que ya es.
    pub fn thicken(&mut self, i: usize, thickness: f32) {
        self.thickness[i] = self.thickness[i].max(thickness);
    }

    /// Pone una pieza en una capa: 0 la de fuera (el exterior), 1 la
    /// siguiente hacia dentro (el forro), etc. Donde dos capas distintas
    /// van en paralelo (sus normales miran hacia el mismo lado), la de
    /// fuera se queda siempre por el lado de la normal de la otra: si se
    /// cruzan, vuelven a su sitio en vez de seguir cruzadas.
    pub fn set_layer(&mut self, piece: &PieceRange, layer: u8) {
        let start = piece.first_particle as usize;
        self.layer[start..start + piece.particle_count as usize].fill(layer);
    }

    /// Hacia dónde empuja el relleno ([`SimParams::pressure`]) a una pieza:
    /// 1 hacia su derecho (la normal de sus triángulos), −1 hacia su revés
    /// (un forro con el derecho hacia dentro, que el relleno aprieta contra
    /// el exterior) y 0 nada (un bolsillo o un asa, que no encierran el
    /// relleno). Por defecto, 1.
    pub fn set_fill(&mut self, piece: &PieceRange, sign: f32) {
        let start = piece.first_triangle as usize;
        self.fill[start..start + piece.triangle_count as usize].fill(sign);
        self.pressure.dirty = true;
    }

    /// Quita todas las costuras.
    pub fn clear_seams(&mut self) {
        self.seams.clear();
        self.seam_order.clear();
        self.self_collision.dirty = true;
        self.pressure.dirty = true;
    }

    pub fn seam_count(&self) -> usize {
        self.seams.len()
    }

    /// `true` mientras quede alguna costura por cerrar.
    pub fn is_sewing(&self) -> bool {
        self.seams.iter().any(|c| c.rest_length > 0.0)
    }

    /// El turno de montaje que se está cosiendo ahora (el más bajo que
    /// queda abierto), o `None` si ya está todo cerrado.
    pub fn sewing_turn(&self) -> Option<u32> {
        self.seams.iter().zip(&self.seam_order).filter(|(c, _)| c.rest_length > 0.0).map(|(_, &t)| t).min()
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
        for substep in 0..substeps {
            // Buscar pares para todo el paso costaría mucho cuando la tela se
            // mueve deprisa (habría que mirar muy lejos): se buscan cada
            // pocos subpasos.
            if params.self_collision && substep % SELF_COLLISION_EVERY == 0 {
                let span = SELF_COLLISION_EVERY.min(substeps - substep) as f32 * h;
                self.gather_self_contacts(span, params.gravity.length());
            }
            let sewing = self.is_sewing();
            if !sewing && !self.pins_after_sewing.is_empty() {
                for i in std::mem::take(&mut self.pins_after_sewing) {
                    self.pin(i);
                }
            }
            let gravity = if sewing { Vec3::ZERO } else { params.gravity };
            // Mientras se cose la prenda aún no está cerrada.
            if !sewing && (params.pressure != 0.0 || self.pressure.current != 0.0) {
                self.rebuild_pressure();
                self.pressure.apply(
                    params.pressure,
                    h,
                    &self.positions,
                    &mut self.velocities,
                    &self.inv_mass,
                    &self.triangles,
                );
            }
            let shrink = params.sew_speed.max(0.0) * h;
            let turn = self.sewing_turn();
            for (c, &t) in self.seams.iter_mut().zip(&self.seam_order) {
                if Some(t) == turn {
                    c.rest_length = (c.rest_length - shrink).max(0.0);
                }
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
            // Los tirantes, antes y después: antes, para que la tela empiece
            // sin estirar de más; después, para que acabe así. Mientras se
            // cose no actúan: las piezas empiezan colocadas más o menos (un
            // asa en arco) y las costuras las llevan a su sitio.
            if !sewing {
                self.solve_tethers();
            }
            self.lambdas.fill(0.0);
            for _ in 0..DISTANCE_ITERATIONS {
                self.solve_distances(h);
                self.solve_seams(h);
            }
            if !sewing {
                self.solve_tethers();
            }
            self.solve_bending(h);
            self.solve_folds(h);
            self.solve_seams(h);
            if params.self_collision {
                self.self_collision.solve(&mut self.positions, &self.prev_positions, &self.inv_mass, &self.triangles);
                self.self_collision.solve_edges(&mut self.positions, &self.prev_positions, &self.inv_mass);
            }
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

    fn rebuild_pressure(&mut self) {
        if self.pressure.dirty {
            let seams: Vec<[u32; 2]> = self.seams.iter().map(|c| [c.a, c.b]).collect();
            let pieces: Vec<u32> = self.pieces.iter().map(|p| p.first_triangle).collect();
            self.pressure.rebuild(self.positions.len(), &self.triangles, &seams, &self.fill, &pieces);
        }
    }

    /// Volumen (m³) que encierra la tela, con las bocas sin coser tapadas.
    pub fn volume(&mut self) -> f32 {
        self.rebuild_pressure();
        self.pressure.volume(&self.positions, &self.triangles)
    }

    /// Bocas (tramos de borde sin coser) que tiene la tela.
    pub fn opening_count(&mut self) -> usize {
        self.rebuild_pressure();
        self.pressure.opening_count()
    }

    /// Presión (Pa) que se está aplicando ahora: tras coser, sube poco a
    /// poco hasta [`SimParams::pressure`].
    pub fn applied_pressure(&self) -> f32 {
        self.pressure.current
    }

    /// Empieza con esta presión ya aplicada, por ejemplo al seguir con la
    /// misma prenda tras editarla, para que no se deshinche y vuelva a subir.
    pub fn set_applied_pressure(&mut self, pressure: f32) {
        self.pressure.current = pressure;
    }

    fn rebuild_self_collision(&mut self) {
        if self.self_collision.dirty {
            let seams: Vec<[u32; 2]> = self.seams.iter().map(|c| [c.a, c.b]).collect();
            self.self_collision.rebuild(self_collision::Topology {
                particles: self.positions.len(),
                triangles: &self.triangles,
                seams: &seams,
                rest_edges: &self.rest_edges,
                rest: &self.rest,
            });
        }
    }

    fn gather_self_contacts(&mut self, dt: f32, gravity: f32) {
        self.rebuild_self_collision();
        // Lo que puede llegar a moverse cada partícula hasta la próxima
        // búsqueda, con holgura.
        let travel: Vec<f32> =
            self.velocities.iter().map(|v| ((v.length() * dt + gravity * dt * dt) * 1.5).min(0.1)).collect();
        let normals = vertex_normals(&self.positions, &self.triangles);
        self.self_collision.gather(
            &self.positions,
            &travel,
            &self.inv_mass,
            &self_collision::Particles { thickness: &self.thickness, layer: &self.layer, normal: &normals },
            &self.triangles,
        );
        self.self_collision.gather_edges(&self.positions, &travel, &self.inv_mass, &self.thickness, &self.mesh_edges);
    }

    /// Número de pares partícula-triángulo vigilados en el último paso.
    pub fn self_contact_count(&self) -> usize {
        self.self_collision.contacts.len() + self.self_collision.edge_contacts.len()
    }

    /// Cuántas aristas de la malla atraviesan un triángulo que no es vecino
    /// suyo: 0 si la tela no se cruza consigo misma. Es lento (compara todo
    /// con todo); sirve para las pruebas.
    pub fn count_crossings(&mut self) -> usize {
        self.crossings().len()
    }

    /// Las aristas que atraviesan un triángulo (que no sea vecino suyo), con
    /// ese triángulo.
    pub fn crossings(&mut self) -> Vec<([u32; 2], [u32; 3])> {
        self.rebuild_self_collision();
        let (positions, triangles, collision) = (&self.positions, &self.triangles, &self.self_collision);
        self.mesh_edges
            .par_iter()
            .flat_map_iter(|&[a, b]| {
                let (p, q) = (positions[a as usize], positions[b as usize]);
                triangles
                    .iter()
                    .filter(move |t| {
                        !collision.adjacent(a, t)
                            && !collision.adjacent(b, t)
                            && self_collision::segment_hits_triangle(p, q, t.map(|v| positions[v as usize]))
                    })
                    .map(move |&t| ([a, b], t))
            })
            .collect()
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
        self.scratch = vec![(Vec3::ZERO, Vec3::ZERO, 0.0); self.constraints.len()];
        self.lambdas = vec![0.0; self.constraints.len()];
    }

    fn solve_distances(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        // Sin restricciones (una tela vacía) no hay ningún grupo.
        let groups = self.colors.len().saturating_sub(1);
        for k in 0..groups {
            let range = self.colors[k]..self.colors[k + 1];
            if range.is_empty() {
                continue;
            }
            if k == groups - 1 || range.len() < PAR_MIN_LEN {
                // Último grupo (puede compartir partículas) o grupo pequeño: en serie.
                for (c, lambda) in self.constraints[range.clone()].iter().zip(&mut self.lambdas[range]) {
                    let (da, db, l) = xpbd_correction(c, *lambda, &self.positions, &self.inv_mass, inv_h2);
                    *lambda = l;
                    self.positions[c.a as usize] += da;
                    self.positions[c.b as usize] += db;
                }
                continue;
            }
            let (positions, inv_mass) = (&self.positions, &self.inv_mass);
            self.constraints[range.clone()]
                .par_iter()
                .zip(&self.lambdas[range.clone()])
                .zip(&mut self.scratch[range.clone()])
                .with_min_len(PAR_MIN_LEN / 2)
                .for_each(|((c, &l), out)| *out = xpbd_correction(c, l, positions, inv_mass, inv_h2));
            let lambdas = &mut self.lambdas[range.clone()];
            for ((c, &(da, db, l)), lambda) in
                self.constraints[range.clone()].iter().zip(&self.scratch[range]).zip(lambdas)
            {
                *lambda = l;
                self.positions[c.a as usize] += da;
                self.positions[c.b as usize] += db;
            }
        }
    }

    /// Los tirantes solo tiran: acercan sus dos partículas si están más
    /// lejos que su máximo, sin flexibilidad.
    fn solve_tethers(&mut self) {
        for t in &self.tethers {
            let (a, b) = (t.a as usize, t.b as usize);
            let (wa, wb) = (self.inv_mass[a], self.inv_mass[b]);
            let d = self.positions[a] - self.positions[b];
            let len = d.length();
            if len <= t.max_length || wa + wb == 0.0 {
                continue;
            }
            let corr = d * ((len - t.max_length) / (len * (wa + wb)));
            self.positions[a] -= corr * wa;
            self.positions[b] += corr * wb;
        }
    }

    fn solve_bending(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for c in &self.bends {
            let x = c.v.map(|i| self.positions[i as usize]);
            let w = c.v.map(|i| self.inv_mass[i as usize]);
            let curvature: Vec3 = x.iter().zip(c.k).map(|(&x, k)| x * k).sum();
            let denom: f32 = w.iter().zip(c.k).map(|(w, k)| w * k * k).sum::<f32>() + c.compliance * inv_h2;
            if denom == 0.0 {
                continue;
            }
            // C = |curvatura|, con gradiente k[i]·n respecto a cada vértice
            // (n = curvatura / C): la corrección es -w[i]·k[i]·curvatura / denom.
            let step = -curvature / denom;
            for ((&v, w), k) in c.v.iter().zip(w).zip(c.k) {
                self.positions[v as usize] += step * (w * k);
            }
        }
    }

    fn solve_folds(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        for f in &self.folds {
            let [a, b, c, d] = f.v.map(|i| i as usize);
            let c2 =
                DistanceConstraint { a: c as u32, b: d as u32, rest_length: f.rest_length, compliance: f.compliance };
            let (dc, dd) = distance_correction(&c2, &self.positions, &self.inv_mass, inv_h2);
            self.positions[c] += dc;
            self.positions[d] += dd;
            if f.side == 0.0 {
                continue;
            }
            // La solapa no puede pasar al otro lado: `c` queda al menos a un
            // grosor del triángulo `a b d`, por el lado del doblez. Así,
            // aunque la tela empiece plana, ya sabe hacia dónde doblarse.
            let x = |i: usize| self.positions[i];
            let n = (x(b) - x(a)).cross(x(d) - x(a)).normalize_or_zero();
            let depth = f.side * n.dot(x(c) - x(a)) - self.thickness[c];
            let (wc, wd) = (self.inv_mass[c], self.inv_mass[d]);
            if depth < 0.0 && wc + wd > 0.0 {
                let push = n * (f.side * -depth / (wc + wd));
                self.positions[c] += push * wc;
                self.positions[d] -= push * wd;
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

/// Una iteración de XPBD sobre una restricción de distancia que ya lleva
/// acumulado el multiplicador `lambda` en este subpaso. Devuelve los
/// desplazamientos y el multiplicador nuevo.
fn xpbd_correction(
    c: &DistanceConstraint,
    lambda: f32,
    positions: &[Vec3],
    inv_mass: &[f32],
    inv_h2: f32,
) -> (Vec3, Vec3, f32) {
    let (a, b) = (c.a as usize, c.b as usize);
    let (wa, wb) = (inv_mass[a], inv_mass[b]);
    let w = wa + wb;
    if w == 0.0 {
        return (Vec3::ZERO, Vec3::ZERO, lambda);
    }
    let d = positions[a] - positions[b];
    let len = d.length();
    if len < 1e-9 {
        return (Vec3::ZERO, Vec3::ZERO, lambda);
    }
    let alpha = c.compliance * inv_h2;
    let delta = -(len - c.rest_length + alpha * lambda) / (w + alpha);
    let corr = d / len * delta;
    (corr * wa, -corr * wb, lambda + delta)
}

/// Bisagras de la malla: para cada arista interior `[a, b, c, d]`, con la
/// arista `a`–`b` y los vértices opuestos `c` y `d` de sus dos triángulos.
fn hinges(triangles: &[[u32; 3]]) -> Vec<[u32; 4]> {
    let mut opposite: HashMap<(u32, u32), u32> = HashMap::with_capacity(triangles.len() * 3);
    let mut hinges = Vec::new();
    for t in triangles {
        for k in 0..3 {
            let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let key = (a.min(b), a.max(b));
            if let Some(other) = opposite.remove(&key) {
                hinges.push([key.0, key.1, other, c]);
            } else {
                opposite.insert(key, c);
            }
        }
    }
    hinges
}

/// Pesos y flexibilidad de una bisagra a partir de su forma en reposo
/// (plana). Con la energía de Bergou `E = 3·B / (A₀ + A₁) · |Σ kᵢ xᵢ|² / 2`,
/// la flexibilidad XPBD es `(A₀ + A₁) / (3·B)`. Devuelve `None` si algún
/// triángulo es degenerado.
fn bend_constraint(x: [Vec3; 4], stiffness: f32) -> Option<BendConstraint> {
    let cot = |apex: Vec3, p: Vec3, q: Vec3| {
        let (u, v) = (p - apex, q - apex);
        let sin = u.cross(v).length();
        (sin > 1e-12).then(|| u.dot(v) / sin)
    };
    let [x0, x1, x2, x3] = x;
    // Ángulos en los extremos de la arista, en cada triángulo.
    let (a0, a1) = (cot(x0, x1, x2)?, cot(x1, x0, x2)?);
    let (b0, b1) = (cot(x0, x1, x3)?, cot(x1, x0, x3)?);
    let k = [-(a1 + b1), -(a0 + b0), a0 + a1, b0 + b1];
    let area = 0.5 * ((x1 - x0).cross(x2 - x0).length() + (x1 - x0).cross(x3 - x0).length());
    Some(BendConstraint { v: [0; 4], k, compliance: area / (3.0 * stiffness) })
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

    /// Dos tiras cosidas por un extremo, con el otro extremo de cada una
    /// para fijar al terminar: mientras se cose, esos puntos se mueven
    /// (la costura los acerca) y después se quedan quietos.
    #[test]
    fn pins_after_sewing_wait_for_the_seams() {
        let mesh = square_mesh(100.0);
        let mut cloth = Cloth::new();
        for x in [0.0, 0.3] {
            cloth.add_piece(
                &mesh,
                &Fabric::COTTON,
                |p| Vec3::new(x + p[0] as f32 * 1e-3, 1.0 - p[1] as f32 * 1e-3, 0.0),
                Vec3::Z,
            );
        }
        let n = mesh.positions.len() as u32;
        let corner = |x: f64, y: f64| mesh.positions.iter().position(|p| p[0] == x && p[1] == y).unwrap() as u32;
        // Lado derecho de la primera con el izquierdo de la segunda.
        for y in [0.0, 100.0] {
            cloth.add_seam(corner(100.0, y), n + corner(0.0, y));
        }
        let top = [corner(0.0, 0.0), corner(100.0, 0.0), n + corner(0.0, 0.0), n + corner(100.0, 0.0)];
        for &i in &top {
            cloth.pin_after_sewing(i as usize);
        }
        let params = SimParams { self_collision: false, ..Default::default() };
        let start = cloth.positions.clone();
        while cloth.is_sewing() {
            cloth.step(1.0 / 60.0, &params, &[]);
        }
        let moved = top.iter().map(|&i| cloth.positions[i as usize].distance(start[i as usize])).fold(0.0, f32::max);
        assert!(moved > 0.05, "los puntos no se han movido al coser: {moved}");
        let sewn: Vec<Vec3> = top.iter().map(|&i| cloth.positions[i as usize]).collect();
        for _ in 0..120 {
            cloth.step(1.0 / 60.0, &params, &[]);
        }
        assert!(cloth.max_seam_gap() < 1e-3, "costura abierta: {}", cloth.max_seam_gap());
        for (&i, p) in top.iter().zip(&sewn) {
            assert!(cloth.inv_mass[i as usize] == 0.0);
            assert!(cloth.positions[i as usize].distance(*p) < 0.01, "punto fijo movido");
        }
        let lowest = cloth.positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        assert!(lowest > 0.7, "se ha caído: {lowest}");
    }

    /// Un patrón sin piezas da una tela vacía: simularla no hace nada.
    #[test]
    fn empty_cloth_steps() {
        let mut cloth = Cloth::new();
        cloth.step(1.0 / 60.0, &SimParams::default(), &[Collider::ground(0.0)]);
        assert!(cloth.positions.is_empty());
    }

    #[test]
    fn coloring_has_no_shared_particles() {
        let mut cloth = Cloth::from_mesh(&square_mesh(300.0), &Fabric::COTTON);
        let total = cloth.constraint_count();
        cloth.recolor();
        assert_eq!(cloth.constraints.len() + cloth.bends.len(), total);
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

    /// Dos costuras en turnos distintos: la segunda no empieza a cerrarse
    /// hasta que la primera está cerrada.
    /// Una tira con un doblez planchado a 180°: con la mitad de abajo fija,
    /// la de arriba se dobla sobre ella hacia el lado que se pide.
    #[test]
    fn creases_fold_toward_their_side() {
        let fold_to = |toward_back: bool| {
            let mut p = Pattern::new("t");
            let id = p.add_rect("tira", Point::ZERO, 300.0, 100.0);
            let fold = p.add_fold(id, Point::new(0.0, 50.0), Point::new(300.0, 50.0)).unwrap();
            let mesh = mesher::mesh_piece_in(&p, p.piece(id).unwrap(), &Default::default()).unwrap();
            let mut cloth = Cloth::from_mesh(&mesh, &Fabric::COTTON);
            let line = &mesh.lines.iter().find(|l| l.line == fold).unwrap().vertices;
            for w in line.windows(2) {
                let crease = Crease { angle: Some(std::f32::consts::PI), toward_back, compliance: 1e-6 };
                assert!(cloth.crease(w[0], w[1], crease));
            }
            for (i, pos) in mesh.positions.iter().enumerate() {
                if pos[1] > 60.0 {
                    cloth.pin(i);
                }
            }
            let params = SimParams { gravity: Vec3::ZERO, ..SimParams::default() };
            for _ in 0..240 {
                cloth.step(1.0 / 60.0, &params, &[]);
            }
            assert!(cloth.positions.iter().all(|p| p.is_finite()));
            // La solapa (la mitad de arriba en el patrón: y > -0,05 en 3D)
            // queda por debajo de la línea y apartada del plano.
            let flap: Vec<Vec3> =
                mesh.positions.iter().zip(&cloth.positions).filter(|(p, _)| p[1] < 40.0).map(|(_, &x)| x).collect();
            let mean = flap.iter().sum::<Vec3>() / flap.len() as f32;
            (mean.y, mean.z)
        };
        let (y, z) = fold_to(true);
        assert!(y < -0.06 && z < 0.0, "hacia atrás: y {y}, z {z}");
        let (y, z) = fold_to(false);
        assert!(y < -0.06 && z > 0.0, "hacia delante: y {y}, z {z}");
    }

    #[test]
    fn seams_close_in_turns() {
        let mesh = square_mesh(200.0);
        let mut cloth = Cloth::new();
        let place = |z: f32| move |p: [f64; 2]| Vec3::new(p[0] as f32 * 1e-3, 1.0 - p[1] as f32 * 1e-3, z);
        let a = cloth.add_piece(&mesh, &Fabric::COTTON, place(0.15), Vec3::Z);
        let b = cloth.add_piece(&mesh, &Fabric::COTTON, place(-0.15), -Vec3::Z);
        let edge = |x: f64| -> Vec<u32> {
            (0..mesh.positions.len() as u32).filter(|&i| mesh.positions[i as usize][0] == x).collect()
        };
        for &i in &edge(0.0) {
            cloth.add_seam_in_turn(a.first_particle + i, b.first_particle + i, 0);
        }
        for &i in &edge(200.0) {
            cloth.add_seam_in_turn(a.first_particle + i, b.first_particle + i, 1);
        }
        let gap = |cloth: &Cloth, x: f64| {
            edge(x)
                .iter()
                .map(|&i| {
                    cloth.positions[(a.first_particle + i) as usize]
                        .distance(cloth.positions[(b.first_particle + i) as usize])
                })
                .fold(0.0, f32::max)
        };
        for _ in 0..30 {
            cloth.step(1.0 / 60.0, &SimParams::default(), &[]);
        }
        // A medio coser la primera, la segunda sigue como estaba (0,3 m).
        assert!(gap(&cloth, 0.0) < 0.2, "primera: {}", gap(&cloth, 0.0));
        assert!(gap(&cloth, 200.0) > 0.25, "segunda: {}", gap(&cloth, 200.0));
        for _ in 0..120 {
            cloth.step(1.0 / 60.0, &SimParams::default(), &[]);
        }
        assert!(!cloth.is_sewing());
        assert!(cloth.max_seam_gap() < 2e-3, "costura abierta: {}", cloth.max_seam_gap());
    }

    #[test]
    fn extra_mass_and_thickness() {
        let mut cloth = Cloth::from_mesh(&square_mesh(100.0), &Fabric::COTTON);
        let w = cloth.inv_mass[0];
        cloth.add_mass(0, 1.0 / w);
        assert!((cloth.inv_mass[0] - w / 2.0).abs() < w * 1e-5);
        cloth.pin(1);
        cloth.add_mass(1, 1.0);
        assert_eq!(cloth.inv_mass[1], 0.0);
        cloth.thicken(0, 0.01);
        cloth.thicken(0, 0.001);
        assert_eq!(cloth.thickness(0), 0.01);
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
    fn stretch_ratios(cloth: &Cloth) -> Vec<f32> {
        let mut ratios: Vec<f32> = cloth
            .mesh_edges
            .iter()
            .zip(&cloth.rest_edges)
            .map(|(&[a, b], rest)| cloth.positions[a as usize].distance(cloth.positions[b as usize]) / rest)
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
        let ratios = stretch_ratios(&cloth);
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

    /// Dos cuadrados (de 40 y 30 cm) que caen uno encima de otro sobre el
    /// suelo: con auto-colisión el de arriba queda encima, separado; sin
    /// ella, se mezclan a la misma altura.
    #[test]
    fn stacked_layers_do_not_pass_through() {
        let drop = |self_collision: bool| {
            let mut cloth = Cloth::new();
            let place = |size: f32, y: f32| {
                move |p: [f64; 2]| Vec3::new(p[0] as f32 * 1e-3 - size / 2.0, y, p[1] as f32 * 1e-3 - size / 2.0)
            };
            cloth.add_piece(&square_mesh(400.0), &Fabric::COTTON, place(0.4, 0.1), Vec3::Y);
            let top = cloth.add_piece(&square_mesh(300.0), &Fabric::COTTON, place(0.3, 0.2), Vec3::Y);
            let params = SimParams { self_collision, ..Default::default() };
            for _ in 0..180 {
                cloth.step(1.0 / 60.0, &params, &[Collider::ground(0.0)]);
            }
            assert!(cloth.positions.iter().all(|p| p.is_finite()));
            let range = top.first_particle as usize..(top.first_particle + top.particle_count) as usize;
            cloth.positions[range].iter().map(|p| p.y).fold(f32::INFINITY, f32::min)
        };
        let with = drop(true);
        assert!(with > 0.0075, "la capa de arriba ha atravesado la de abajo: {with}");
        let without = drop(false);
        assert!(without < 0.005, "sin auto-colisión deberían mezclarse: {without}");
    }

    /// Dos cuadrados de 40 cm, uno sobre otro a 2 cm, cosidos por los
    /// bordes `sewn` (0 arriba, 1 derecha, 2 abajo, 3 izquierda), en vertical
    /// y sin gravedad ni obstáculos. Devuelve la tela tras `seconds`.
    fn sewn_squares(sewn: &[usize], pressure: f32, seconds: f32) -> Cloth {
        let size = 400.0;
        let mesh = square_mesh(size);
        let mut cloth = Cloth::new();
        let place = |z: f32| move |p: [f64; 2]| Vec3::new(p[0] as f32 * 1e-3 - 0.2, 0.6 - p[1] as f32 * 1e-3, z);
        let a = cloth.add_piece(&mesh, &Fabric::COTTON, place(0.01), Vec3::Z);
        let b = cloth.add_piece(&mesh, &Fabric::COTTON, place(-0.01), -Vec3::Z);
        let on_side = |p: [f64; 2], side: usize| match side {
            0 => p[1] == 0.0,
            1 => p[0] == size,
            2 => p[1] == size,
            _ => p[0] == 0.0,
        };
        for (i, &p) in mesh.positions.iter().enumerate() {
            if sewn.iter().any(|&side| on_side(p, side)) {
                cloth.add_seam(a.first_particle + i as u32, b.first_particle + i as u32);
            }
        }
        let params = SimParams { gravity: Vec3::ZERO, pressure, ..Default::default() };
        for _ in 0..(seconds * 60.0) as usize {
            cloth.step(1.0 / 60.0, &params, &[]);
        }
        assert!(cloth.positions.iter().all(|p| p.is_finite()));
        cloth
    }

    fn centroid(cloth: &Cloth) -> Vec3 {
        cloth.positions.iter().copied().sum::<Vec3>() / cloth.positions.len() as f32
    }

    /// Un cojín cosido por los cuatro lados: sin presión queda plano; con
    /// ella se hincha (un cojín de 40 × 40 cm lleno ronda los 10 l) sin
    /// romper las costuras ni estirar mucho la tela.
    #[test]
    fn pressure_inflates_a_cushion() {
        let flat = sewn_squares(&[0, 1, 2, 3], 0.0, 1.0).volume();
        let mut cushion = sewn_squares(&[0, 1, 2, 3], 40.0, 3.0);
        assert_eq!(cushion.opening_count(), 0);
        let volume = cushion.volume();
        eprintln!("cojín: {:.2} l plano, {:.2} l hinchado", flat * 1e3, volume * 1e3);
        // Sin gravedad, la firmeza de la tela ya lo abomba algo al coserlo.
        assert!(flat < volume / 4.0, "sin presión debería quedar casi plano: {flat}");
        assert!(volume > 0.007 && volume < 0.02, "volumen del cojín: {volume}");
        assert!(cushion.max_seam_gap() < 2e-3, "costura abierta: {}", cushion.max_seam_gap());
        let ratios = stretch_ratios(&cushion);
        assert!(ratios[ratios.len() / 2] < 1.02, "mediana del estiramiento: {}", ratios[ratios.len() / 2]);
        // Las fuerzas se compensan: no se desplaza.
        let drift = centroid(&cushion).distance(Vec3::new(0.0, 0.4, 0.0));
        assert!(drift < 0.01, "el cojín se ha movido {drift} m");
    }

    /// Una bolsa abierta por arriba: la boca se tapa con una tapa imaginaria,
    /// así que también se hincha, y la bolsa no sale disparada hacia abajo
    /// (por donde la presión no tiene contrapeso sin tapa).
    #[test]
    fn pressure_fills_an_open_bag() {
        let mut bag = sewn_squares(&[1, 2, 3], 30.0, 3.0);
        assert_eq!(bag.opening_count(), 1);
        let volume = bag.volume();
        eprintln!("bolsa: {:.2} l", volume * 1e3);
        assert!(volume > 0.006, "la bolsa no se ha llenado: {volume}");
        let drift = centroid(&bag).distance(Vec3::new(0.0, 0.4, 0.0));
        assert!(drift < 0.02, "la bolsa se ha movido {drift} m");
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

    #[test]
    fn layers_add_up() {
        let one = Fabric::layered(&[Fabric::COTTON]);
        assert_eq!(one, Fabric::COTTON);
        let two = Fabric::layered(&[Fabric::COTTON, Fabric::COTTON]);
        assert!((two.relative_firmness() - 2.0).abs() < 1e-4, "dos capas iguales: {}", two.relative_firmness());
        assert!((two.density - 0.30).abs() < 1e-6 && (two.thickness - 0.006).abs() < 1e-6);
        // Un bolso de cuero con forro: más firme que el cuero solo.
        let bag = Fabric::layered(&[Fabric::LEATHER, Fabric::LINING]);
        assert_eq!(bag.friction, Fabric::LEATHER.friction, "el rozamiento es el de fuera");
        assert!(bag.bend_stiffness > Fabric::LEATHER.bend_stiffness);
        assert!(bag.stretch_compliance < Fabric::LEATHER.stretch_compliance);
        let rigid = Fabric::layered(&[Fabric::COTTON, Fabric { stretch_compliance: 0.0, ..Fabric::LINING }]);
        assert_eq!(rigid.stretch_compliance, 0.0);
        assert_eq!(Fabric::layered(&[]), Fabric::default());
    }

    /// Un cuadrado de 10 cm en horizontal, sujeto (como con una pinza) por
    /// sus 3 primeros cm: cuanto más firme es la tela, menos cae el borde
    /// suelto (tras 6 s, ya quieto). Una capa ligera pero firme (como una entretela) hace que caiga
    /// menos; dos capas iguales no, porque doblan la firmeza y también el
    /// peso.
    #[test]
    fn firmer_fabric_droops_less() {
        let droop = |fabric: &Fabric| {
            let mesh = square_mesh(100.0);
            let mut cloth = Cloth::new();
            cloth.add_piece(&mesh, fabric, |p| Vec3::new(p[0] as f32, 500.0, p[1] as f32) * 1e-3, Vec3::Y);
            for (i, p) in mesh.positions.iter().enumerate() {
                if p[0] < 30.0 {
                    cloth.pin(i);
                }
            }
            for _ in 0..360 {
                cloth.step(1.0 / 60.0, &SimParams::default(), &[]);
            }
            assert!(cloth.positions.iter().all(|p| p.is_finite()));
            0.5 - min_y(&cloth)
        };
        let cotton = droop(&Fabric::COTTON);
        let denim = droop(&Fabric::DENIM);
        let interfacing = Fabric { density: 0.05, bend_stiffness: 1e-4, ..Fabric::LINING };
        let interfaced = droop(&Fabric::layered(&[Fabric::DENIM, interfacing]));
        let double_denim = droop(&Fabric::layered(&[Fabric::DENIM, Fabric::DENIM]));
        let leather = droop(&Fabric::LEATHER);
        eprintln!(
            "caída: algodón {cotton:.3}, vaquero {denim:.3}, vaquero con entretela {interfaced:.3}, \
             vaquero doble {double_denim:.3}, cuero {leather:.3}"
        );
        assert!(cotton > 0.06, "el algodón debería caer casi entero: {cotton}");
        assert!(cotton > denim + 0.005 && denim > interfaced + 0.01 && interfaced > leather + 0.01);
        assert!((double_denim - denim).abs() < 0.003, "vaquero doble {double_denim}, sencillo {denim}");
        assert!(leather < 0.03, "el cuero apenas debería caer: {leather}");
    }

    /// Una bisagra plana no tiene curvatura, la mueva como la mueva sin
    /// doblarla; doblada, sí.
    #[test]
    fn flat_hinge_has_no_curvature() {
        let x = [Vec3::ZERO, Vec3::new(0.02, 0.0, 0.0), Vec3::new(0.005, 0.017, 0.0), Vec3::new(0.013, -0.02, 0.0)];
        let c = bend_constraint(x, 1e-5).unwrap();
        assert!(c.k.iter().sum::<f32>().abs() < 1e-5);
        let curvature = |x: [Vec3; 4]| (0..4).map(|i| x[i] * c.k[i]).sum::<Vec3>().length();
        let moved = x.map(|p| glam::Quat::from_rotation_y(0.7) * p * 1.0 + Vec3::new(0.3, -0.1, 2.0));
        assert!(curvature(moved) < 1e-6, "curvatura: {}", curvature(moved));
        let mut folded = x;
        folded[3].z = 0.01;
        assert!(curvature(folded) > 1e-3);
    }

    /// Cuánto se alarga (0,01 = un 1 %) una tira de 50 × 300 mm colgada de
    /// arriba con `load` kg repartidos por abajo.
    fn hanging_strip_stretch(fabric: &Fabric, load: f32) -> f32 {
        let mut p = pattern::Pattern::new("t");
        let id = p.add_rect("tira", pattern::Point::ZERO, 50.0, 300.0);
        let options = mesher::MeshOptions { target_edge_length: 20.0, ..Default::default() };
        let mesh = mesher::mesh_piece(p.piece(id).unwrap(), &options).unwrap();
        let mut cloth = Cloth::from_mesh(&mesh, fabric);
        let bottom: Vec<usize> = (0..mesh.positions.len()).filter(|&i| mesh.positions[i][1] == 300.0).collect();
        for (i, q) in mesh.positions.iter().enumerate() {
            if q[1] == 0.0 {
                cloth.pin(i);
            }
        }
        for &i in &bottom {
            cloth.add_mass(i, load / bottom.len() as f32);
        }
        let params = SimParams { self_collision: false, ..Default::default() };
        for _ in 0..600 {
            cloth.step(1.0 / 60.0, &params, &[]);
        }
        let low = bottom.iter().map(|&i| cloth.positions[i].y).sum::<f32>() / bottom.len() as f32;
        -low / 0.3 - 1.0
    }

    /// Con el mismo peso, el punto cede mucho más que un tejido. Con cuatro
    /// veces más peso, el tejido no se alarga cuatro veces más: los tirantes
    /// lo frenan cerca de su límite.
    #[test]
    fn knit_stretches_and_woven_stops() {
        let woven = hanging_strip_stretch(&Fabric::COTTON, 0.05);
        let knit = hanging_strip_stretch(&Fabric::KNIT, 0.05);
        let heavy = hanging_strip_stretch(&Fabric::COTTON, 0.2);
        eprintln!(
            "algodón {:.2} %, punto {:.2} %, algodón con más peso {:.2} %",
            woven * 100.0,
            knit * 100.0,
            heavy * 100.0
        );
        assert!(woven < 0.02, "el algodón se estira un {} %", woven * 100.0);
        assert!(knit > 2.5 * woven, "el punto ({knit}) cede como el algodón ({woven})");
        assert!(heavy < 2.5 * woven, "con más peso, el algodón se estira un {} %", heavy * 100.0);
    }
}
