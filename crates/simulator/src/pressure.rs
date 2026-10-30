//! Presión interior: da volumen a una prenda cerrada, como si tuviera relleno
//! o aire dentro (un cojín, una bolsa llena).
//!
//! Cada triángulo recibe una fuerza `presión × área` hacia su derecho (fuera
//! de la prenda), repartida entre sus tres vértices. Es el gradiente del
//! volumen encerrado por la tela, multiplicado por la presión.
//!
//! Las bocas que quedan abiertas (la de una bolsa, el cuello o las mangas de
//! una camiseta) se cierran con una tapa imaginaria: un abanico de
//! triángulos desde el centro de la boca. La tapa también empuja, así que la
//! boca se abre y se levanta en vez de escaparse el relleno por ella, y la
//! suma de todas las fuerzas es cero: la prenda no sale disparada.
//!
//! Una arista del borde de una pieza está cosida si hay otra arista del
//! borde cuyos extremos están cosidos a los suyos; las que quedan son las
//! bocas.
//!
//! El relleno va en el hueco de dentro de la prenda y empuja hacia fuera
//! la tela que lo rodea. Cada pieza dice hacia dónde: hacia su derecho
//! (la normal de sus triángulos), hacia su revés (un forro, con el derecho
//! hacia dentro: así el relleno lo aprieta contra el exterior en vez de
//! hinchar el hueco entre los dos) o nada (un bolsillo o un asa, que no
//! encierran el relleno). Tampoco empuja a las piezas que casi no van
//! cosidas por su borde (una cinta cosida solo por sus puntas).

use std::collections::HashMap;

use glam::Vec3;

/// Tiempo (s) en que la presión pasa de cero a la pedida: subirla de golpe
/// haría saltar la tela, y con las telas tejidas casi sin estirar, un cojín
/// arrugado que se abre de golpe salta de la mesa.
const RAMP: f32 = 2.0;

/// Una pieza con menos de esta parte de su borde cosida no encierra el
/// relleno (una cinta o un asa cosidas por sus puntas).
const MIN_SEWN: f32 = 0.25;

/// La presión empieza a subir cuando la tela, ya cosida, casi se ha parado
/// (velocidad media en m/s): una bolsa que se hincha mientras cae llega
/// redonda y rueda fuera de la mesa; si se hincha ya apoyada, se queda.
const SETTLED_SPEED: f32 = 0.05;
/// Y no antes de este tiempo (s) desde que se termina de coser: justo al
/// coser la tela está quieta en el aire, y aún tiene que caer.
const SETTLE_TIME: f32 = 0.5;

/// Una boca: un tramo cerrado de borde sin coser.
#[derive(Clone, Debug, Default)]
struct Opening {
    /// Aristas `[a, b]` en el sentido de su triángulo y hacia dónde empuja
    /// el relleno a ese triángulo.
    edges: Vec<([u32; 2], f32)>,
    /// Partículas de la boca, sin repetir.
    particles: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Pressure {
    /// Hay que volver a buscar las bocas (nuevas piezas o costuras).
    pub dirty: bool,
    openings: Vec<Opening>,
    /// Presión (Pa) que se aplica ahora; sube poco a poco hasta la pedida.
    pub current: f32,
    /// La tela ya se ha asentado una vez tras coser.
    started: bool,
    /// Tiempo (s) que lleva cosida, hasta que empieza la presión.
    waited: f32,
    /// Gradiente del volumen (m²) de cada partícula.
    gradient: Vec<Vec3>,
    /// Hacia dónde empuja el relleno a cada triángulo: 1 hacia su normal,
    /// −1 al revés, 0 nada.
    signs: Vec<f32>,
}

impl Pressure {
    /// Busca las bocas de la tela y qué triángulos empuja el relleno.
    /// `fill` es hacia dónde empuja a cada triángulo (ver
    /// [`crate::Cloth::set_fill`]) y `pieces`, el primer triángulo de cada
    /// pieza (en orden).
    pub fn rebuild(
        &mut self,
        particles: usize,
        triangles: &[[u32; 3]],
        seams: &[[u32; 2]],
        fill: &[f32],
        pieces: &[u32],
    ) {
        self.dirty = false;
        let mut parent: Vec<u32> = (0..particles as u32).collect();
        for &[a, b] in seams {
            union(&mut parent, a, b);
        }
        let root: Vec<u32> = (0..particles as u32).map(|i| find(&mut parent, i)).collect();

        // Aristas del borde de las piezas (las que solo tienen un triángulo),
        // con su triángulo.
        let mut uses: HashMap<(u32, u32), ([u32; 2], u32, usize)> = HashMap::with_capacity(triangles.len() * 3);
        for (i, t) in triangles.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                uses.entry((a.min(b), a.max(b))).or_insert(([a, b], 0, i)).1 += 1;
            }
        }
        let mut border: Vec<([u32; 2], usize)> =
            uses.into_values().filter(|&(_, n, _)| n == 1).map(|(e, _, t)| (e, t)).collect();
        border.sort_unstable();

        // Cosidas: dos o más aristas del borde entre los mismos puntos.
        let key = |[a, b]: [u32; 2]| {
            let (ra, rb) = (root[a as usize], root[b as usize]);
            (ra.min(rb), ra.max(rb))
        };
        let count = |edges: &mut dyn Iterator<Item = [u32; 2]>| {
            let mut sides: HashMap<(u32, u32), u32> = HashMap::new();
            for e in edges {
                *sides.entry(key(e)).or_default() += 1;
            }
            sides
        };
        let piece_of = |t: usize| pieces.partition_point(|&first| first as usize <= t).saturating_sub(1);
        let sides = count(&mut border.iter().map(|&(e, _)| e));
        let sewn = |sides: &HashMap<(u32, u32), u32>, e: [u32; 2]| {
            let (ra, rb) = key(e);
            ra == rb || sides[&(ra, rb)] > 1
        };
        let mut edges_of = vec![(0u32, 0u32); pieces.len().max(1)];
        for &(e, t) in &border {
            let counts = &mut edges_of[piece_of(t)];
            counts.0 += 1;
            counts.1 += sewn(&sides, e) as u32;
        }
        let encloses: Vec<bool> =
            edges_of.iter().map(|&(all, sewn)| all > 0 && sewn as f32 >= MIN_SEWN * all as f32).collect();
        self.signs = (0..triangles.len())
            .map(|t| {
                let piece = piece_of(t);
                if pieces.is_empty() || encloses[piece] { fill.get(t).copied().unwrap_or(1.0) } else { 0.0 }
            })
            .collect();

        // Las bocas, solo entre los triángulos que empuja el relleno: lo
        // cosido a un asa sigue siendo boca.
        border.retain(|&(_, t)| self.signs[t] != 0.0);
        let sides = count(&mut border.iter().map(|&(e, _)| e));
        let open: Vec<([u32; 2], f32)> =
            border.into_iter().filter(|&(e, _)| !sewn(&sides, e)).map(|(e, t)| (e, self.signs[t])).collect();

        // Cada grupo de aristas abiertas que se tocan es una boca.
        let mut groups = root.clone();
        for &([a, b], _) in &open {
            union(&mut groups, root[a as usize], root[b as usize]);
        }
        let mut index: HashMap<u32, usize> = HashMap::new();
        self.openings.clear();
        for e in open {
            let g = find(&mut groups, root[e.0[0] as usize]);
            let i = *index.entry(g).or_insert_with(|| {
                self.openings.push(Opening::default());
                self.openings.len() - 1
            });
            self.openings[i].edges.push(e);
        }
        for opening in &mut self.openings {
            opening.particles = opening.edges.iter().flat_map(|(e, _)| *e).collect();
            opening.particles.sort_unstable();
            opening.particles.dedup();
            // Ordenadas para que el resultado no dependa del `HashMap`.
            opening.edges.sort_unstable_by_key(|(e, _)| *e);
        }
        self.openings.sort_by_key(|o| o.edges[0].0);
    }

    pub fn opening_count(&self) -> usize {
        self.openings.len()
    }

    /// Volumen encerrado (m³), con las bocas tapadas.
    pub fn volume(&self, positions: &[Vec3], triangles: &[[u32; 3]]) -> f32 {
        // Respecto al centro de la tela, para no perder precisión lejos del
        // origen.
        let origin = positions.iter().copied().sum::<Vec3>() / positions.len().max(1) as f32;
        let p = |i: u32| positions[i as usize] - origin;
        let mut volume: f32 =
            triangles.iter().zip(&self.signs).map(|(&[a, b, c], s)| s * p(a).dot(p(b).cross(p(c)))).sum();
        for opening in &self.openings {
            let center = opening.center(positions) - origin;
            // La tapa recorre cada arista al revés que su triángulo.
            volume += opening.edges.iter().map(|&([a, b], s)| s * p(b).dot(p(a).cross(center))).sum::<f32>();
        }
        volume / 6.0
    }

    /// Acelera las partículas con la presión de este subpaso. La presión
    /// aplicada se acerca a `target` en [`RAMP`] segundos, desde que la tela
    /// se para por primera vez. Mientras se cose no se llama: la prenda aún
    /// no está cerrada.
    pub fn apply(
        &mut self,
        target: f32,
        h: f32,
        positions: &[Vec3],
        velocities: &mut [Vec3],
        inv_mass: &[f32],
        triangles: &[[u32; 3]],
    ) {
        if !self.started {
            self.waited += h;
            let speed = velocities.iter().map(|v| v.length()).sum::<f32>() / velocities.len().max(1) as f32;
            if self.current == 0.0 && (speed > SETTLED_SPEED || self.waited < SETTLE_TIME) {
                return;
            }
            self.started = true;
        }
        let rate = target.abs().max(self.current.abs()) / RAMP * h;
        self.current += (target - self.current).clamp(-rate, rate);
        if self.current == 0.0 {
            return;
        }
        self.gradient.clear();
        self.gradient.resize(positions.len(), Vec3::ZERO);
        let p = |i: u32| positions[i as usize];
        // Un tercio de la normal (con su área) de cada triángulo, para cada
        // vértice: (b - a) × (c - a) es el doble del área, de ahí el 1/6.
        for (&[a, b, c], &sign) in triangles.iter().zip(&self.signs) {
            if sign == 0.0 {
                continue;
            }
            let n = (p(b) - p(a)).cross(p(c) - p(a)) * (sign / 6.0);
            for i in [a, b, c] {
                self.gradient[i as usize] += n;
            }
        }
        for opening in &self.openings {
            let center = opening.center(positions);
            let mut to_center = Vec3::ZERO;
            for &([a, b], sign) in &opening.edges {
                let n = (p(a) - p(b)).cross(center - p(b)) * (sign / 6.0);
                self.gradient[a as usize] += n;
                self.gradient[b as usize] += n;
                to_center += n;
            }
            // El centro se mueve con todos los puntos de la boca.
            let share = to_center / opening.particles.len() as f32;
            for &i in &opening.particles {
                self.gradient[i as usize] += share;
            }
        }
        let scale = self.current * h;
        for ((v, g), &w) in velocities.iter_mut().zip(&self.gradient).zip(inv_mass) {
            *v += *g * (w * scale);
        }
    }
}

impl Opening {
    fn center(&self, positions: &[Vec3]) -> Vec3 {
        self.particles.iter().map(|&i| positions[i as usize]).sum::<Vec3>() / self.particles.len() as f32
    }
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        let up = parent[parent[i as usize] as usize];
        parent[i as usize] = up;
        i = up;
    }
    i
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra.max(rb) as usize] = ra.min(rb);
    }
}
