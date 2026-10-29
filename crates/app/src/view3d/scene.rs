//! Montaje de la escena 3D a partir del patrón y simulación en un hilo
//! aparte, para que la interfaz no se frene aunque la tela sea grande.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;
use glam::Vec3;
use mesher::MeshOptions;
use pattern::Pattern;
use simulator::{Cloth, Collider, Fabric, SimParams};

/// Cómo se colocan las piezas antes de soltarlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Tumbadas en horizontal, a medio metro sobre el obstáculo, y caen.
    Drop,
    /// En vertical, colgadas de su borde superior.
    Hang,
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
            placement: Placement::Drop,
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
    let mut meshes = Vec::new();
    let mut failed = Vec::new();
    for piece in &pattern.pieces {
        match mesher::mesh_piece(piece, &options) {
            Ok(mesh) => meshes.push((piece, mesh)),
            Err(_) => failed.push(piece.name.clone()),
        }
    }

    // Se respeta la disposición de las piezas en el editor, centrada.
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (_, mesh) in &meshes {
        for p in &mesh.positions {
            for k in 0..2 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
    }
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let drop_height = settings.obstacle.top() + 0.1;

    let mut cloth = Cloth::new();
    let mut colors = Vec::new();
    for (piece, mesh) in &meshes {
        let range = match settings.placement {
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

    let mut colliders = vec![Collider::ground(0.0)];
    colliders.extend(settings.obstacle.collider());
    Scene { cloth, colliders, colors, failed }
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

    pub fn stats(&self) -> (f32, f32) {
        let shared = self.shared.lock().unwrap();
        (shared.sim_time, shared.step_ms)
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
            shared.step_ms = if shared.step_ms == 0.0 { ms } else { shared.step_ms * 0.9 + ms * 0.1 };
        }
        // Si un paso tarda más que un fotograma, la simulación va más lenta
        // que el tiempo real en vez de acumular retraso.
        next = (next + Duration::from_secs_f32(FRAME)).max(Instant::now());
    }
}
