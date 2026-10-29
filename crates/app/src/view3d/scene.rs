//! Montaje de la escena 3D a partir del patrón y simulación en un hilo
//! aparte, para que la interfaz no se frene aunque la tela sea grande.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;
use glam::Vec3;
use mesher::{Mesh2D, MeshOptions};
use pattern::{Arrangement, Pattern};
use simulator::{Cloth, Collider, Fabric, SimParams};

/// Cómo se colocan las piezas antes de soltarlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Alrededor de un eje vertical, cada pieza en su lado (delante, detrás,
    /// a los lados, arriba o abajo) y curvada, para que las costuras cierren
    /// la prenda u objeto.
    Around,
    /// Tumbadas en horizontal, a medio metro sobre el obstáculo, y caen.
    Drop,
    /// En vertical, colgadas de su borde superior.
    Hang,
}

impl Placement {
    pub const ALL: [Placement; 3] = [Placement::Around, Placement::Drop, Placement::Hang];

    pub fn label(self) -> &'static str {
        match self {
            Placement::Around => "Alrededor",
            Placement::Drop => "Soltar",
            Placement::Hang => "Colgar",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Placement::Around => {
                "Cada pieza en su lado (Colocación 3D en sus propiedades), curvada alrededor de un eje"
            }
            Placement::Drop => "Las piezas caen en horizontal sobre el obstáculo",
            Placement::Hang => "Las piezas cuelgan de su borde superior",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Obstacle {
    None,
    Sphere,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FabricKind {
    Cotton,
    Silk,
    Denim,
}

impl FabricKind {
    pub const ALL: [FabricKind; 3] = [FabricKind::Cotton, FabricKind::Silk, FabricKind::Denim];

    pub fn label(self) -> &'static str {
        match self {
            FabricKind::Cotton => "Algodón",
            FabricKind::Silk => "Seda",
            FabricKind::Denim => "Vaquero",
        }
    }

    pub fn fabric(self) -> Fabric {
        match self {
            FabricKind::Cotton => Fabric::COTTON,
            FabricKind::Silk => Fabric::SILK,
            FabricKind::Denim => Fabric::DENIM,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneSettings {
    pub placement: Placement,
    /// Cerrar las costuras del patrón.
    pub sew: bool,
    pub obstacle: Obstacle,
    pub fabric: FabricKind,
    /// Tamaño de los triángulos de la tela (mm).
    pub mesh_size: f64,
    /// Subpasos por fotograma: más = tela menos elástica, pero más lento.
    pub substeps: u32,
}

impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            placement: Placement::Around,
            sew: true,
            obstacle: Obstacle::Table,
            fabric: FabricKind::Cotton,
            mesh_size: 20.0,
            substeps: SimParams::default().substeps,
        }
    }
}

impl SceneSettings {
    pub fn params(&self) -> SimParams {
        SimParams { substeps: self.substeps, ..Default::default() }
    }
}

/// Lo necesario para simular y dibujar el patrón en 3D.
pub struct Scene {
    pub cloth: Cloth,
    pub colliders: Vec<Collider>,
    /// Color (lineal) de cada partícula: el de su pieza en el editor.
    pub colors: Vec<[f32; 3]>,
    /// Piezas que no se han podido mallar.
    pub failed: Vec<String>,
    /// Costuras que se van a cerrar.
    pub seams: usize,
}

const SPHERE_RADIUS: f32 = 0.25;
const TABLE_HALF: Vec3 = Vec3::new(0.4, 0.375, 0.3);
/// Altura (m) del borde superior de las piezas colgadas.
const HANG_HEIGHT: f32 = 1.6;

impl Obstacle {
    pub fn collider(self) -> Option<Collider> {
        match self {
            Obstacle::None => None,
            Obstacle::Sphere => {
                Some(Collider::Sphere { center: Vec3::new(0.0, SPHERE_RADIUS, 0.0), radius: SPHERE_RADIUS })
            }
            Obstacle::Table => {
                Some(Collider::Cuboid { center: Vec3::new(0.0, TABLE_HALF.y, 0.0), half_extents: TABLE_HALF })
            }
        }
    }

    fn top(self) -> f32 {
        match self {
            Obstacle::None => 0.0,
            Obstacle::Sphere => 2.0 * SPHERE_RADIUS,
            Obstacle::Table => 2.0 * TABLE_HALF.y,
        }
    }
}

pub fn build(
    pattern: &Pattern,
    settings: &SceneSettings,
    color_of: impl Fn(pattern::PieceId) -> egui::Color32,
) -> Scene {
    let options = MeshOptions { target_edge_length: settings.mesh_size, ..Default::default() };
    let fabric = settings.fabric.fabric();
    let meshed = mesher::mesh_pattern(pattern, &options);
    let mut meshes = Vec::new();
    let mut failed = Vec::new();
    for (index, (piece, mesh)) in pattern.pieces.iter().zip(&meshed.pieces).enumerate() {
        match mesh {
            Ok(mesh) => meshes.push((index, piece, mesh)),
            Err(_) => failed.push(piece.name.clone()),
        }
    }

    // Se respeta la disposición de las piezas en el editor, centrada.
    let all: Vec<&Mesh2D> = meshes.iter().map(|m| m.2).collect();
    let (min, max) = bounds(&all);
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let drop_height = settings.obstacle.top() + 0.1;
    let around = settings.placement == Placement::Around;
    let layout = around.then(|| AroundLayout::new(&meshes, settings.obstacle.top()));

    let mut cloth = Cloth::new();
    let mut colors = Vec::new();
    let mut first_particle = vec![0u32; pattern.pieces.len()];
    for &(index, piece, mesh) in &meshes {
        let range = match settings.placement {
            Placement::Around => {
                let layout = layout.as_ref().expect("calculada arriba");
                let a = piece.arrangement;
                cloth.add_piece(mesh, &fabric, |p| layout.place(a, p), layout.front(a))
            }
            Placement::Drop => cloth.add_piece(
                mesh,
                &fabric,
                |p| Vec3::new(((p[0] - center[0]) * 1e-3) as f32, drop_height, ((p[1] - center[1]) * 1e-3) as f32),
                Vec3::Y,
            ),
            Placement::Hang => cloth.add_piece(
                mesh,
                &fabric,
                |p| Vec3::new(((p[0] - center[0]) * 1e-3) as f32, HANG_HEIGHT - ((p[1] - min[1]) * 1e-3) as f32, 0.0),
                Vec3::Z,
            ),
        };
        first_particle[index] = range.first_particle;
        if settings.placement == Placement::Hang {
            // Fija los puntos más altos de cada pieza (con 5 mm de margen).
            let top = mesh.positions.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
            for (i, p) in mesh.positions.iter().enumerate() {
                if p[1] - top < 5.0 {
                    cloth.pin(range.first_particle as usize + i);
                }
            }
        }
        let color = egui::Rgba::from(color_of(piece.id));
        colors.extend(std::iter::repeat_n([color.r(), color.g(), color.b()], mesh.positions.len()));
    }

    let mut seams = 0;
    if settings.sew {
        for seam in &meshed.seams {
            seams += 1;
            // Cada punto se une con el mismo punto de todos los demás lados.
            for (i, a) in seam.sides.iter().enumerate() {
                for b in &seam.sides[i + 1..] {
                    for (&va, &vb) in a.vertices.iter().zip(&b.vertices) {
                        cloth.add_seam(first_particle[a.piece] + va, first_particle[b.piece] + vb);
                    }
                }
            }
        }
    }

    let mut colliders = vec![Collider::ground(0.0)];
    colliders.extend(settings.obstacle.collider());
    Scene { cloth, colliders, colors, failed, seams }
}

fn bounds(meshes: &[&Mesh2D]) -> ([f64; 2], [f64; 2]) {
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for mesh in meshes {
        for p in &mesh.positions {
            for k in 0..2 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
    }
    (min, max)
}

/// Colocación «alrededor»: las piezas de delante, detrás y los lados se
/// curvan sobre un cilindro vertical, cada grupo centrado en su lado y con
/// la disposición del editor; las de arriba y abajo van en horizontal,
/// tapando el cilindro.
struct AroundLayout {
    radius: f64,
    /// Centro horizontal (mm) de cada grupo en el patrón.
    group_center: HashMap<Arrangement, [f64; 2]>,
    /// Borde inferior (mm, Y hacia abajo) de las piezas laterales.
    bottom: f64,
    /// Altura (m) del borde inferior de las piezas laterales.
    base: f32,
    /// Altura (m) de la tapa de arriba y de la de abajo.
    top_y: f32,
    bottom_y: f32,
}

/// Separación (m) entre las tapas y las piezas laterales.
const CAP_GAP: f32 = 0.05;

impl AroundLayout {
    fn new(meshes: &[(usize, &pattern::Piece, &Mesh2D)], floor: f32) -> Self {
        let mut groups: HashMap<Arrangement, Vec<&Mesh2D>> = HashMap::new();
        for (_, piece, mesh) in meshes {
            groups.entry(piece.arrangement).or_default().push(mesh);
        }
        let group_bounds: HashMap<Arrangement, ([f64; 2], [f64; 2])> =
            groups.iter().map(|(a, m)| (*a, bounds(m))).collect();
        let group_center = group_bounds
            .iter()
            .map(|(a, (min, max))| (*a, [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0]))
            .collect();

        let is_wall = |a: &Arrangement| !matches!(a, Arrangement::Top | Arrangement::Bottom);
        let walls: Vec<&Mesh2D> =
            groups.iter().filter(|(a, _)| is_wall(a)).flat_map(|(_, m)| m.iter().copied()).collect();
        let (wall_min, wall_max) = bounds(&walls);
        // Cada grupo ocupa como mucho este ángulo del cilindro, para que
        // quede un hueco entre grupos vecinos.
        let sides = groups.contains_key(&Arrangement::Left) || groups.contains_key(&Arrangement::Right);
        let max_angle = if sides { 0.42 } else { 0.85 } * std::f64::consts::PI;
        let widest =
            group_bounds.iter().filter(|(a, _)| is_wall(a)).map(|(_, (min, max))| max[0] - min[0]).fold(0.0, f64::max);
        let radius = (widest / max_angle).max(100.0);

        let height = if walls.is_empty() { 0.0 } else { ((wall_max[1] - wall_min[1]) * 1e-3) as f32 };
        let has_bottom = groups.contains_key(&Arrangement::Bottom);
        let base = floor + 0.1 + if has_bottom { CAP_GAP } else { 0.0 };
        Self {
            radius,
            group_center,
            bottom: if walls.is_empty() { 0.0 } else { wall_max[1] },
            base,
            top_y: base + height + CAP_GAP,
            bottom_y: base - CAP_GAP,
        }
    }

    /// Ángulo del centro de cada lado alrededor del eje (0 = delante, +Z).
    fn angle(a: Arrangement) -> f64 {
        use std::f64::consts::{FRAC_PI_2, PI};
        match a {
            Arrangement::Right => FRAC_PI_2,
            Arrangement::Back => PI,
            Arrangement::Left => -FRAC_PI_2,
            _ => 0.0,
        }
    }

    fn place(&self, a: Arrangement, p: [f64; 2]) -> Vec3 {
        let c = self.group_center.get(&a).copied().unwrap_or_default();
        let (dx, dy) = ((p[0] - c[0]) * 1e-3, (p[1] - c[1]) * 1e-3);
        match a {
            Arrangement::Top => Vec3::new(dx as f32, self.top_y, dy as f32),
            Arrangement::Bottom => Vec3::new(dx as f32, self.bottom_y, -dy as f32),
            _ => {
                // La x del dibujo avanza hacia la derecha de quien mira la pieza.
                let theta = Self::angle(a) + (p[0] - c[0]) / self.radius;
                let r = (self.radius * 1e-3) as f32;
                let y = self.base + ((self.bottom - p[1]) * 1e-3) as f32;
                Vec3::new(r * theta.sin() as f32, y, r * theta.cos() as f32)
            }
        }
    }

    fn front(&self, a: Arrangement) -> Vec3 {
        match a {
            Arrangement::Top => Vec3::Y,
            Arrangement::Bottom => -Vec3::Y,
            _ => {
                let theta = Self::angle(a);
                Vec3::new(theta.sin() as f32, 0.0, theta.cos() as f32)
            }
        }
    }
}

// ---------------------------------------------------------------- hilo

enum Command {
    Load(u64, Box<Cloth>, Vec<Collider>, SimParams),
    Run(bool),
    Quit,
}

#[derive(Default)]
struct Shared {
    /// Tela cargada ahora mismo; el hilo descarta pasos de una anterior.
    generation: u64,
    positions: Vec<Vec3>,
    /// Cambia cada vez que llegan posiciones nuevas.
    version: u64,
    /// Tiempo simulado (s) desde que se cargó la tela.
    sim_time: f32,
    /// Coste medio de un fotograma de simulación (ms).
    step_ms: f32,
    /// Quedan costuras por cerrar.
    sewing: bool,
}

/// Estado de la simulación para mostrarlo.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub sim_time: f32,
    pub step_ms: f32,
    pub sewing: bool,
}

/// Simulación que corre en su propio hilo a 60 pasos por segundo.
pub struct Runner {
    commands: Sender<Command>,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
    running: bool,
}

pub const FRAME: f32 = 1.0 / 60.0;

impl Runner {
    pub fn new() -> Self {
        let (commands, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("simulación".into())
            .spawn(move || run(rx, thread_shared))
            .expect("no se pudo crear el hilo de simulación");
        Self { commands, shared, thread: Some(thread), running: false }
    }

    pub fn load(&mut self, cloth: Cloth, colliders: Vec<Collider>, params: SimParams) {
        let generation = {
            let mut shared = self.shared.lock().unwrap();
            shared.generation += 1;
            shared.positions.clone_from(&cloth.positions);
            shared.version += 1;
            shared.sim_time = 0.0;
            shared.step_ms = 0.0;
            shared.sewing = cloth.is_sewing();
            shared.generation
        };
        let _ = self.commands.send(Command::Load(generation, Box::new(cloth), colliders, params));
    }

    pub fn set_running(&mut self, running: bool) {
        self.running = running;
        let _ = self.commands.send(Command::Run(running));
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Copia las posiciones más recientes en `out` si han cambiado desde
    /// `version`. Devuelve la versión actual.
    pub fn read(&self, version: u64, out: &mut Vec<Vec3>) -> u64 {
        let shared = self.shared.lock().unwrap();
        if shared.version != version {
            out.clone_from(&shared.positions);
        }
        shared.version
    }

    pub fn stats(&self) -> Stats {
        let shared = self.shared.lock().unwrap();
        Stats { sim_time: shared.sim_time, step_ms: shared.step_ms, sewing: shared.sewing }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(rx: Receiver<Command>, shared: Arc<Mutex<Shared>>) {
    let mut scene: Option<(u64, Box<Cloth>, Vec<Collider>, SimParams)> = None;
    let mut running = false;
    let mut next = Instant::now();
    loop {
        // En pausa se espera sin gastar CPU; en marcha, hasta el siguiente paso.
        let timeout = if running && scene.is_some() {
            next.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        match rx.recv_timeout(timeout) {
            Ok(Command::Load(generation, cloth, colliders, params)) => {
                scene = Some((generation, cloth, colliders, params));
                next = Instant::now();
                continue;
            }
            Ok(Command::Run(run)) => {
                running = run;
                next = Instant::now();
                continue;
            }
            Ok(Command::Quit) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let Some((generation, cloth, colliders, params)) = scene.as_mut() else { continue };
        let start = Instant::now();
        cloth.step(FRAME, params, colliders);
        let ms = start.elapsed().as_secs_f32() * 1000.0;
        {
            let mut shared = shared.lock().unwrap();
            if shared.generation != *generation {
                continue; // ya hay otra tela en camino
            }
            shared.positions.clone_from(&cloth.positions);
            shared.version += 1;
            shared.sim_time += FRAME;
            shared.sewing = cloth.is_sewing();
            shared.step_ms = if shared.step_ms == 0.0 { ms } else { shared.step_ms * 0.9 + ms * 0.1 };
        }
        // Si un paso tarda más que un fotograma, la simulación va más lenta
        // que el tiempo real en vez de acumular retraso.
        next = (next + Duration::from_secs_f32(FRAME)).max(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples;

    fn build_example(make: fn() -> Pattern, settings: &SceneSettings) -> Scene {
        build(&make(), settings, |_| egui::Color32::WHITE)
    }

    /// Colocadas alrededor, los dos lados de cada costura de los ejemplos
    /// corren en el mismo sentido: al revés, los puntos quedarían más lejos.
    #[test]
    fn example_seams_line_up_when_placed_around() {
        let settings = SceneSettings::default();
        for (name, _, make) in examples::ALL {
            let pattern = make();
            let scene = build_example(make, &settings);
            assert!(scene.failed.is_empty());
            assert_eq!(scene.seams, pattern.seams.len(), "{name}");
            let meshed = mesher::mesh_pattern(
                &pattern,
                &MeshOptions { target_edge_length: settings.mesh_size, ..Default::default() },
            );
            for seam in &meshed.seams {
                let points = |side: &mesher::SeamSideMesh| -> Vec<Vec3> {
                    let base = scene.cloth.pieces[side.piece].first_particle;
                    side.vertices.iter().map(|&v| scene.cloth.positions[(base + v) as usize]).collect()
                };
                let (a, b) = (points(&seam.sides[0]), points(&seam.sides[1]));
                let straight: f32 = a.iter().zip(&b).map(|(p, q)| p.distance(*q)).sum();
                let crossed: f32 = a.iter().zip(b.iter().rev()).map(|(p, q)| p.distance(*q)).sum();
                let seam_name = &pattern.seam(seam.seam).unwrap().name;
                assert!(straight < crossed, "{name}, «{seam_name}»: {straight} ≥ {crossed}");
            }
        }
    }

    /// Cosidos y soltados sobre la mesa, los ejemplos se cierran y quedan
    /// quietos sin que nada explote.
    #[test]
    fn examples_sew_and_settle() {
        let settings = SceneSettings { mesh_size: 30.0, ..Default::default() };
        for (name, _, make) in examples::ALL {
            let mut scene = build_example(make, &settings);
            assert!(scene.cloth.is_sewing());
            for _ in 0..240 {
                scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
            }
            assert!(scene.cloth.positions.iter().all(|p| p.is_finite()), "{name}");
            assert!(!scene.cloth.is_sewing(), "{name}: sigue cosiendo");
            let gap = scene.cloth.max_seam_gap();
            assert!(gap < 5e-3, "{name}: costura abierta {gap}");
            let lowest = scene.cloth.positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            assert!(lowest > -1e-3, "{name}: atraviesa el suelo ({lowest})");
        }
    }
}
