//! Montaje de la escena 3D a partir del patrón y simulación en un hilo
//! aparte, para que la interfaz no se frene aunque la tela sea grande.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use avatar::{Avatar, ZoneFrame};
use eframe::egui;
use glam::Vec3;
use mesher::{Mesh2D, MeshOptions};
use pattern::{Arrangement, BodyZone, Pattern, Piece, SeamFit, SeamFold, SeamKind, Textile, Topstitch};
use simulator::{Cloth, Collider, Fabric, SimParams};

/// Cómo se colocan las piezas antes de soltarlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Alrededor de un eje vertical, cada pieza en su lado (delante, detrás,
    /// a los lados, arriba o abajo) y curvada, para que las costuras cierren
    /// la prenda u objeto. Con el maniquí, el eje es el de la zona del
    /// cuerpo de cada pieza (torso, brazos o piernas).
    Around,
    /// Tumbadas en horizontal sobre el obstáculo, y caen. Si se cosen, van
    /// una encima de otra, como una prenda doblada (sobre el maniquí, como
    /// «alrededor» pero algo más arriba).
    Drop,
    /// Colgadas de su parte más alta: cosidas en el aire, delante del
    /// obstáculo, o cada pieza de su borde superior si no se cosen.
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
                "Cada pieza en su lado (Colocación 3D en sus propiedades), curvada alrededor de un eje \
                 o, con el maniquí, alrededor de su zona del cuerpo"
            }
            Placement::Drop => {
                "Las piezas caen en horizontal sobre el obstáculo, las de delante encima de las de detrás \
                 (sobre el maniquí, la prenda cae sobre los hombros)"
            }
            Placement::Hang => "La pieza se cose en el aire y luego queda colgada de su parte más alta",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Obstacle {
    None,
    Sphere,
    Table,
    /// El maniquí, para vestir prendas.
    Mannequin,
}

/// Propiedades físicas de una tela de la biblioteca.
pub fn physics(textile: &Textile) -> Fabric {
    Fabric {
        density: (textile.weight * 1e-3) as f32,
        // Escalas logarítmicas: de 10⁻⁷ a 10⁻³ N·m de firmeza, y de un
        // módulo de 10⁶ N/m (no cede nada) a 10 N/m (un elástico muy
        // blando). Una lona sale a unos 3·10⁴ N/m, un algodón a 3·10³ y un
        // punto a 180.
        bend_stiffness: (1e-7 * 10f64.powf(textile.firmness / 25.0)) as f32,
        stretch_compliance: (1e-6 * 10f64.powf(textile.stretch / 20.0)) as f32,
        friction: textile.friction as f32,
        thickness: (textile.thickness * 1e-3) as f32,
    }
}

/// La tela de una pieza con todas sus capas juntas.
pub fn piece_fabric(pattern: &Pattern, piece: &Piece) -> Fabric {
    let layers: Vec<Fabric> = pattern.piece_textiles(piece).iter().map(physics).collect();
    Fabric::layered(&layers)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneSettings {
    pub placement: Placement,
    /// Cerrar las costuras del patrón.
    pub sew: bool,
    pub obstacle: Obstacle,
    /// Tamaño de los triángulos de la tela (mm).
    pub mesh_size: f64,
    /// Subpasos por fotograma: más = tela menos elástica, pero más lento.
    pub substeps: u32,
    /// Que la tela no se atraviese a sí misma.
    pub self_collision: bool,
    /// Coser las costuras de una en una, en el orden de montaje.
    pub sew_in_order: bool,
}

impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            placement: Placement::Around,
            sew: true,
            obstacle: Obstacle::Table,
            mesh_size: 20.0,
            substeps: SimParams::default().substeps,
            self_collision: true,
            sew_in_order: false,
        }
    }
}

impl SceneSettings {
    pub fn params(&self) -> SimParams {
        SimParams { substeps: self.substeps, self_collision: self.self_collision, ..Default::default() }
    }

    /// Si se puede pasar de `self` a `other` conservando la forma de la
    /// prenda (solo cambia la resolución) o hay que volver a colocar las
    /// piezas.
    pub fn keeps_shape(&self, other: &SceneSettings) -> bool {
        (self.placement, self.obstacle, self.sew) == (other.placement, other.obstacle, other.sew)
    }
}

/// Lo necesario para simular y dibujar el patrón en 3D.
pub struct Scene {
    pub cloth: Cloth,
    pub colliders: Vec<Collider>,
    /// Color (lineal) de cada partícula por la cara hacia la que apuntan
    /// los triángulos (fuera) y por la otra. El derecho de la tela es el
    /// color de su pieza en el editor; el revés, uno más apagado.
    pub colors: Vec<[f32; 3]>,
    pub back_colors: Vec<[f32; 3]>,
    /// Costuras que se dibujan en 3D (cordón, margen y pespuntes).
    pub seam_draws: Vec<SeamDraw>,
    /// Piezas que no se han podido mallar.
    pub failed: Vec<pattern::PieceId>,
    /// Costuras cuyos lados quedan en sentidos contrarios al colocar las
    /// piezas: al cerrarse, la tela se retuerce.
    pub crossed: Vec<pattern::SeamId>,
    /// Costuras que se van a cerrar.
    pub seams: usize,
    /// Las piezas malladas, en el orden en que están en la tela.
    pub pieces: Vec<ScenePiece>,
    /// Partículas unidas por las costuras y su turno de montaje.
    seam_pairs: Vec<([u32; 2], u32)>,
    /// Etapa de montaje de cada turno (0 = sin etapa; `k` = la etapa `k`
    /// de la línea de montaje, contando desde 1).
    pub turn_stages: Vec<usize>,
    /// Broches y presillas que se dibujan en 3D.
    pub fasteners: Vec<FastenerDraw>,
}

/// Un broche o una presilla en la tela: sus dos partículas.
#[derive(Clone, Copy, Debug)]
pub struct FastenerDraw {
    pub kind: pattern::FastenerKind,
    pub particles: [u32; 2],
    /// Hacia qué cara de los triángulos queda el derecho de cada pieza (ver
    /// [`right_sign`]).
    pub right_sign: [f32; 2],
}

/// Una costura tal y como se dibuja en 3D.
#[derive(Clone, Debug)]
pub struct SeamDraw {
    /// Posición de la costura en el patrón (para su color).
    pub index: usize,
    pub kind: SeamKind,
    pub topstitch: Topstitch,
    /// Lado (en `sides`) hacia el que va planchado el margen.
    pub pressed: Option<usize>,
    pub sides: Vec<SeamSideDraw>,
    /// En una cremallera, el punto (en `sides[k].particles`) donde está el
    /// tirador: el primero cerrado, o uno más allá del final si está toda
    /// abierta.
    pub slider: Option<usize>,
}

/// Un lado de una costura en la tela.
#[derive(Clone, Debug)]
pub struct SeamSideDraw {
    /// Partículas a lo largo de la costura.
    pub particles: Vec<u32>,
    /// Para cada una, la vecina hacia el interior de la pieza (hacia donde
    /// se tumba el margen).
    pub inward: Vec<Option<u32>>,
    /// Distancia en reposo (m) a esa vecina.
    pub inward_rest: Vec<f32>,
    /// Flexibilidad (m/N) de la costura en este lado y grosor (m) de su tela.
    pub compliance: f32,
    /// La tela es tan firme que la costura no cambia cómo se dobla.
    pub firm: bool,
    pub thickness: f32,
    /// Ancho del margen que se ve (m).
    pub allowance: f32,
    /// 1 si el derecho de la pieza mira hacia donde apuntan los triángulos,
    /// −1 si mira al otro lado.
    pub right_sign: f32,
    pub right_color: [f32; 3],
    pub wrong_color: [f32; 3],
}

/// Una pieza ya mallada y metida en la tela.
pub struct ScenePiece {
    pub id: pattern::PieceId,
    /// Vértices del contorno en el patrón, para saber si la pieza se ha
    /// movido en el editor.
    pub outline: Vec<pattern::Point>,
    pub mesh: Mesh2D,
    pub first_particle: u32,
}

/// Flexibilidad (m/N) de las restricciones que endurecen una costura en una
/// pieza de tela `fabric`: cuanto más firme la tela y el tipo de costura,
/// menos cede. Con algodón y una costura simple sale 2·10⁻³, la flexión que
/// tenía el algodón antes de la flexión de Bergou. Nunca cede menos que la
/// tela al estirar (cada restricción abarca dos aristas): una costura en
/// punto se estira con el punto, como un puño cosido estirado.
fn seam_compliance(fabric: &Fabric, kind: SeamKind) -> f32 {
    const REFERENCE: f32 = 1e-8;
    (REFERENCE / (fabric.bend_stiffness.max(1e-9) * kind.stiffness())).max(4.0 * fabric.stretch_compliance)
}

/// Firmeza (N·m) a partir de la cual la tela decide sola cómo se tumba la
/// costura (del vaquero para arriba): ahí no se añade el pliegue preferido.
/// Contra una flexión tan firme, el pliegue y la auto-colisión se peleaban
/// y la tela temblaba sin llegar a pararse.
const FIRM_FABRIC: f32 = 2e-5;
/// El pliegue preferido es una tendencia, no una bisagra rígida: cede
/// bastante más que la rigidez a lo largo de la costura.
const FOLD_SOFTNESS: f32 = 4.0;

/// Flexibilidad de un doblez planchado: se queda doblado, pero cede algo.
const FOLD_COMPLIANCE: f32 = 1e-6;

/// Si un doblez lleva la solapa hacia la cara de atrás de los triángulos de
/// la tela (la contraria a la que mira hacia fuera al colocar la pieza).
fn toward_back(pattern: &Pattern, piece: &Piece, fold: &pattern::Fold) -> bool {
    (right_sign(pattern, piece) > 0.0) != fold.toward_right_side
}

/// Dónde empieza un punto (mm) de una pieza con dobleces que se colocan ya
/// doblados: reflejado en cada doblez cuya solapa lo contiene y, por cada
/// uno, una capa (m) por delante (positivo) o por detrás de la pieza.
fn folded(pattern: &Pattern, piece: &Piece, p: [f64; 2]) -> ([f64; 2], f32) {
    let sep = 2.0 * piece_fabric(pattern, piece).thickness + 0.002;
    let mut q = pattern::Point::new(p[0], p[1]);
    let mut offset = 0.0;
    for line in &piece.lines {
        let Some(fold) = line.fold.filter(|f| f.starts_folded()) else { continue };
        if line.in_flap(q) {
            q = line.reflect(q);
            offset += if toward_back(pattern, piece, &fold) { -sep } else { sep };
        }
    }
    ([q.x, q.y], offset)
}

/// Hasta dónde (mm) se nota el frunce de una costura dentro de la tela.
const GATHER_DEPTH: f64 = 40.0;

/// Longitud en reposo (m) de un tramo `a`–`b` (posiciones en el patrón, m)
/// de una tela fruncida (o estirada) a lo largo de la polilínea `line`: el
/// tramo `line[k]`–`line[k + 1]` pasa a medir `ratio[k]` de su largo, y lo
/// que va en su misma dirección cerca de él cambia menos cuanto más lejos;
/// a `depth` ya no se nota. `None` si el tramo no cambia.
fn gathered_length(line: &[Vec3], ratio: &[f32], depth: f32, a: Vec3, b: Vec3) -> Option<f32> {
    let mid = (a + b) / 2.0;
    let (distance, k) = line
        .windows(2)
        .enumerate()
        .map(|(k, w)| {
            let d = w[1] - w[0];
            let t = ((mid - w[0]).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
            (mid.distance(w[0] + d * t), k)
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))?;
    let fade = 1.0 - distance / depth;
    let shrink = (1.0 - ratio[k]) * fade;
    if fade <= 0.0 || shrink.abs() < 1e-6 {
        return None;
    }
    let tangent = (line[k + 1] - line[k]).normalize_or_zero();
    let e = b - a;
    let along = tangent * e.dot(tangent);
    Some((along * (1.0 - shrink) + (e - along)).length())
}

/// Color del revés de una tela del color `c` (lineal): más claro y apagado.
pub fn wrong_side_color(c: [f32; 3]) -> [f32; 3] {
    c.map(|x| x * 0.45 + 0.3)
}

/// 1 si el derecho de la pieza queda hacia fuera de la prenda, −1 si no.
fn right_sign(pattern: &Pattern, piece: &Piece) -> f32 {
    if piece.right_side_in != pattern.inside_out { -1.0 } else { 1.0 }
}

const SPHERE_RADIUS: f32 = 0.25;
/// Holgura (m) entre el cuerpo y las piezas al colocarlas.
const BODY_CLEARANCE: f32 = 0.03;
const TABLE_HALF: Vec3 = Vec3::new(0.4, 0.375, 0.3);
/// Altura (m) del borde superior de las piezas colgadas.
const HANG_HEIGHT: f32 = 1.6;
/// Por debajo del punto más alto, hasta dónde se fija la tela al colgarla (m).
const HANG_MARGIN: f32 = 0.02;
/// Al soltar piezas cosidas, separación (m) entre las dos capas y entre las
/// piezas de una misma capa.
const LAYER_GAP: f32 = 0.03;
const FLAT_GAP: f64 = 20.0;
/// Al soltar una prenda sobre el maniquí, cuánto más alta (m) que al
/// colocarla alrededor.
const BODY_DROP: f32 = 0.08;

impl Obstacle {
    pub const ALL: [Obstacle; 4] = [Obstacle::Mannequin, Obstacle::None, Obstacle::Sphere, Obstacle::Table];

    pub fn label(self) -> &'static str {
        match self {
            Obstacle::None => "Solo suelo",
            Obstacle::Sphere => "Esfera",
            Obstacle::Table => "Mesa",
            Obstacle::Mannequin => "Maniquí",
        }
    }

    /// El maniquí, si este obstáculo lo es (la primera vez se carga).
    pub fn avatar(self) -> Option<Arc<Avatar>> {
        (self == Obstacle::Mannequin).then(avatar::mannequin)
    }

    pub fn collider(self) -> Option<Collider> {
        match self {
            Obstacle::None => None,
            Obstacle::Mannequin => self.avatar().map(|a| a.collider()),
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
            Obstacle::Mannequin => self.avatar().map_or(0.0, |a| a.height()),
        }
    }

    /// Hasta dónde llega hacia delante (z), para colgar las piezas delante.
    fn front(self) -> f32 {
        match self {
            Obstacle::None => 0.0,
            Obstacle::Sphere => SPHERE_RADIUS,
            Obstacle::Table => TABLE_HALF.z,
            Obstacle::Mannequin => self.avatar().map_or(0.0, |a| a.front_z()),
        }
    }
}

pub fn build(
    pattern: &Pattern,
    settings: &SceneSettings,
    color_of: impl Fn(pattern::PieceId) -> egui::Color32,
) -> Scene {
    let options = MeshOptions { target_edge_length: settings.mesh_size, ..Default::default() };
    let meshed = mesher::mesh_pattern(pattern, &options);
    let mut meshes = Vec::new();
    let mut failed = Vec::new();
    for (index, (piece, mesh)) in pattern.pieces.iter().zip(&meshed.pieces).enumerate() {
        match mesh {
            Ok(mesh) => meshes.push((index, piece, mesh)),
            // Los agujeros se recortan de su pieza: no son tela.
            Err(mesher::MeshError::Hole) => {}
            Err(_) => failed.push(piece.id),
        }
    }

    // Sin costuras, las piezas se colocan sueltas, con la disposición del
    // editor centrada. Con costuras, ya colocadas para que cierren sin
    // cruzarse: al soltarlas, tumbadas una encima de otra (sobre el
    // maniquí, como «alrededor» pero algo más arriba); al colgarlas, como
    // «alrededor» pero en el aire, delante del obstáculo.
    let sewn = settings.sew && !meshed.seams.is_empty();
    let on_body = settings.obstacle == Obstacle::Mannequin;
    let all: Vec<&Mesh2D> = meshes.iter().map(|m| m.2).collect();
    let (min, max) = bounds(&all);
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let drop_height = settings.obstacle.top() + 0.1;
    let avatar = settings.obstacle.avatar();
    // Las piezas que van encima de otra se colocan con ella; las demás, por
    // grupos.
    let mounts = mount_places(pattern, &meshed, &meshes);
    let free: Vec<(usize, &Piece, &Mesh2D)> = meshes.iter().copied().filter(|m| !mounts.contains_key(&m.0)).collect();
    let around = match settings.placement {
        Placement::Around => true,
        Placement::Hang => sewn,
        Placement::Drop => sewn && on_body,
    }
    .then(|| AroundLayout::new(&free, &pattern.seams, settings.obstacle.top(), avatar.as_deref()));
    let flat = (settings.placement == Placement::Drop && sewn && !on_body).then(|| FlatLayout::new(&free));
    // Los dobleces que empiezan doblados: la solapa va reflejada sobre el
    // resto de la pieza, una capa por delante o por detrás.
    let fold = |index: usize, p: [f64; 2]| folded(pattern, &pattern.pieces[index], p);
    let place_around = |layout: &AroundLayout, index: usize, p: [f64; 2]| {
        let (p, extra) = fold(index, p);
        let (root, p, offset) = resolve_mount(&mounts, index, p);
        layout.place(&pattern.pieces[root], p, offset + extra)
    };
    let root = |index: usize| &pattern.pieces[resolve_mount(&mounts, index, [0.0, 0.0]).0];
    let hang_z = settings.obstacle.front() + if settings.obstacle == Obstacle::None { 0.0 } else { 0.1 };
    // Al colgar la pieza montada, se sube hasta `HANG_HEIGHT` y se aparta
    // hasta quedar delante del obstáculo.
    let lift = match &around {
        Some(layout) if settings.placement == Placement::Hang => {
            let placed = meshes
                .iter()
                .flat_map(|&(index, _, mesh)| mesh.positions.iter().map(move |&p| place_around(layout, index, p)));
            let (top, back) = placed.fold((f32::NEG_INFINITY, f32::INFINITY), |(t, b), p| (t.max(p.y), b.min(p.z)));
            Vec3::new(0.0, HANG_HEIGHT - top, (hang_z - back).max(0.0))
        }
        Some(_) if settings.placement == Placement::Drop => Vec3::Y * BODY_DROP,
        _ => Vec3::ZERO,
    };

    // Al soltarla sobre la mesa, lo que cuelga por debajo de las piezas de
    // detrás (un asa en arco) también empieza por encima de la mesa.
    let flat_lift = match &flat {
        Some(layout) => {
            let mounts = &mounts;
            let lowest = meshes
                .iter()
                .flat_map(|&(index, _, mesh)| {
                    mesh.positions.iter().map(move |&p| {
                        let (p, extra) = fold(index, p);
                        let (root, p, offset) = resolve_mount(mounts, index, p);
                        layout.place(&pattern.pieces[root], p, drop_height, offset + extra).y
                    })
                })
                .fold(f32::INFINITY, f32::min);
            Vec3::Y * (drop_height - lowest).max(0.0)
        }
        None => Vec3::ZERO,
    };

    let mut cloth = Cloth::new();
    let mut colors = Vec::new();
    let mut back_colors = Vec::new();
    let mut first_particle = vec![0u32; pattern.pieces.len()];
    let mut pieces = Vec::new();
    for &(index, piece, mesh) in &meshes {
        let fabric = piece_fabric(pattern, piece);
        let range = if let Some(layout) = &around {
            cloth.add_piece(mesh, &fabric, |p| place_around(layout, index, p) + lift, layout.front(root(index)))
        } else if let Some(layout) = &flat {
            let place = |p| {
                let (p, extra) = fold(index, p);
                let (root, p, offset) = resolve_mount(&mounts, index, p);
                layout.place(&pattern.pieces[root], p, drop_height, offset + extra) + flat_lift
            };
            cloth.add_piece(mesh, &fabric, place, layout.front(root(index)))
        } else if settings.placement == Placement::Hang {
            cloth.add_piece(
                mesh,
                &fabric,
                |p| {
                    let (p, extra) = fold(index, p);
                    Vec3::new(
                        ((p[0] - center[0]) * 1e-3) as f32,
                        HANG_HEIGHT - ((p[1] - min[1]) * 1e-3) as f32,
                        hang_z + extra,
                    )
                },
                Vec3::Z,
            )
        } else {
            cloth.add_piece(
                mesh,
                &fabric,
                |p| {
                    let (p, extra) = fold(index, p);
                    Vec3::new(
                        ((p[0] - center[0]) * 1e-3) as f32,
                        drop_height + extra,
                        ((p[1] - center[1]) * 1e-3) as f32,
                    )
                },
                Vec3::Y,
            )
        };
        cloth.set_layer(&range, root(index).layer);
        // El relleno empuja hacia fuera de la prenda: hacia el derecho de las
        // piezas que lo tienen fuera y hacia el revés de los forros. Lo que va
        // encima de otra pieza (un bolsillo, un asa) no encierra el relleno.
        let fill = if pattern.mount(piece.id).is_some() { 0.0 } else { right_sign(pattern, piece) };
        cloth.set_fill(&range, fill);
        first_particle[index] = range.first_particle;
        pieces.push(ScenePiece {
            id: piece.id,
            outline: piece.vertices().to_vec(),
            mesh: mesh.clone(),
            first_particle: range.first_particle,
        });
        let color = egui::Rgba::from(color_of(piece.id));
        let right = [color.r(), color.g(), color.b()];
        let (front, back) = if right_sign(pattern, piece) > 0.0 {
            (right, wrong_side_color(right))
        } else {
            (wrong_side_color(right), right)
        };
        colors.extend(std::iter::repeat_n(front, mesh.positions.len()));
        back_colors.extend(std::iter::repeat_n(back, mesh.positions.len()));
    }

    // Dobleces: bisagras libres o planchados a su ángulo y hacia su lado.
    for &(index, piece, mesh) in &meshes {
        for meshed_line in &mesh.lines {
            let Some(line) = piece.lines.iter().find(|l| l.id == meshed_line.line) else { continue };
            let Some(fold) = line.fold else { continue };
            let crease = simulator::Crease {
                angle: fold.pressed.then_some(fold.angle.to_radians() as f32),
                toward_back: toward_back(pattern, piece, &fold),
                compliance: FOLD_COMPLIANCE,
            };
            let base = first_particle[index];
            for w in meshed_line.vertices.windows(2) {
                cloth.crease(base + w[0], base + w[1], crease);
            }
        }
    }

    if settings.placement == Placement::Hang {
        // Cuelga de sus puntos más altos (con 2 cm de margen, para que una
        // prenda quede cogida por los hombros y no solo por el cuello): los
        // de toda la pieza montada o, sin coser, los de cada pieza. Se fijan
        // al terminar de coser: antes, las costuras tienen que poder
        // moverlos para cerrar.
        let ranges: Vec<(usize, usize)> = if sewn {
            vec![(0, cloth.positions.len())]
        } else {
            cloth
                .pieces
                .iter()
                .map(|r| (r.first_particle as usize, (r.first_particle + r.particle_count) as usize))
                .collect()
        };
        for (start, end) in ranges {
            let top = cloth.positions[start..end].iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            for i in start..end {
                if top - cloth.positions[i].y < HANG_MARGIN {
                    cloth.pin_after_sewing(i);
                }
            }
        }
    }

    let mut seams = 0;
    let mut seam_pairs = Vec::new();
    let mut seam_draws = Vec::new();
    let mut crossed = Vec::new();
    let mut turn_stages = Vec::new();
    if settings.sew {
        let mut topologies: HashMap<usize, Topology> = HashMap::new();
        let turns = pattern.sewing_turns(settings.sew_in_order);
        for (seam, &turn) in pattern.seams.iter().zip(&turns) {
            let turn = turn as usize;
            if turn_stages.len() <= turn {
                turn_stages.resize(turn + 1, 0);
            }
            turn_stages[turn] = pattern.seam_stage(seam);
        }
        for seam_mesh in &meshed.seams {
            let Some((index, seam)) = pattern.seams.iter().enumerate().find(|(_, s)| s.id == seam_mesh.seam) else {
                continue;
            };
            seams += 1;
            let turn = turns.get(index).copied().unwrap_or(0);
            let side_points = |side: &mesher::SeamSideMesh| -> Vec<Vec3> {
                let base = first_particle[side.piece];
                side.vertices.iter().map(|&v| cloth.positions[(base + v) as usize]).collect()
            };
            if let Some((first, rest)) = seam_mesh.sides.split_first() {
                let a = side_points(first);
                if rest.iter().any(|b| runs_backwards(&a, &side_points(b))) {
                    crossed.push(seam.id);
                }
            }
            // Separación en el patrón (mm) entre cada punto de un lado y el
            // siguiente. En una costura fruncida o embebida, el lado largo
            // se encoge en la línea de costura hasta lo que mide el corto, y
            // menos según se aleja de ella: la tela se abre en ondas. En una
            // estirada, cada tela cede según lo elástica que sea.
            let gaps: Vec<Vec<f64>> = seam_mesh
                .sides
                .iter()
                .map(|side| {
                    let Ok(mesh) = &meshed.pieces[side.piece] else { return Vec::new() };
                    let at = |v: u32| glam::DVec2::from(mesh.positions[v as usize]);
                    side.vertices.windows(2).map(|w| at(w[0]).distance(at(w[1]))).collect()
                })
                .collect();
            // Qué puntos quedan cerrados: todos, salvo en una cremallera
            // abierta lo que va del principio al tirador.
            let closed: Vec<bool> = {
                let first = gaps.first().map_or(&[][..], |g| g.as_slice());
                let total: f64 = first.iter().sum();
                let mut along = 0.0;
                let count = seam_mesh.sides.iter().map(|s| s.vertices.len()).min().unwrap_or(0);
                (0..count)
                    .map(|k| {
                        if k > 0 {
                            along += first.get(k - 1).copied().unwrap_or(0.0);
                        }
                        !seam.is_open_at(if total > 0.0 { along / total } else { 0.0 })
                    })
                    .collect()
            };
            // Cada punto se une con el mismo punto de todos los demás lados.
            for (i, a) in seam_mesh.sides.iter().enumerate() {
                for b in &seam_mesh.sides[i + 1..] {
                    for (k, (&va, &vb)) in a.vertices.iter().zip(&b.vertices).enumerate() {
                        if closed[k] {
                            seam_pairs.push(([first_particle[a.piece] + va, first_particle[b.piece] + vb], turn));
                        }
                    }
                }
            }
            // El largo al que queda cada tramo de la línea de costura. En una
            // fruncida o embebida, el del lado corto. En una estirada, donde
            // se equilibran las telas: cada una cede según su módulo, el lado
            // corto estirándose y el largo encogiéndose, y una tela tejida
            // apenas cede ante un punto. Se calcula aquí, en vez de dejarlo
            // al solver, porque con los subpasos de siempre la tela firme
            // cedería mucho más de lo que dice su módulo.
            let target: Option<Vec<f64>> = match seam.fit {
                SeamFit::Gathered | SeamFit::Eased => gaps
                    .iter()
                    .filter(|g| !g.is_empty())
                    .min_by(|a, b| a.iter().sum::<f64>().total_cmp(&b.iter().sum::<f64>()))
                    .cloned(),
                SeamFit::Stretched => {
                    let modulus: Vec<f64> = seam_mesh
                        .sides
                        .iter()
                        .map(|side| {
                            let c = piece_fabric(pattern, &pattern.pieces[side.piece]).stretch_compliance as f64;
                            if c > 0.0 { 1.0 / c } else { 1e9 }
                        })
                        .collect();
                    let count = gaps.iter().map(Vec::len).min().unwrap_or(0);
                    (count > 0).then(|| {
                        (0..count)
                            .map(|k| {
                                let stiffness: f64 = modulus.iter().sum();
                                let per_length: f64 = gaps.iter().zip(&modulus).map(|(g, e)| e / g[k].max(1e-9)).sum();
                                stiffness / per_length
                            })
                            .collect()
                    })
                }
                SeamFit::Exact => None,
            };

            let kind = seam.kind;
            let mut sides = Vec::new();
            for (side, own) in seam_mesh.sides.iter().zip(&gaps) {
                let Ok(mesh) = &meshed.pieces[side.piece] else { continue };
                let piece = &pattern.pieces[side.piece];
                let fabric = piece_fabric(pattern, piece);
                let compliance = seam_compliance(&fabric, kind);
                let topology = topologies.entry(side.piece).or_insert_with(|| Topology::new(mesh));
                let base = first_particle[side.piece];
                let v = &side.vertices;
                let at = |k: usize| glam::DVec2::from(mesh.positions[v[k] as usize]);
                let edge = seam.sides[side.side];
                let allowance = pattern.allowance(edge.piece, edge.edge) * 1e-3;
                let gap = match &target {
                    Some(target) if target.len() == own.len() => target,
                    _ => own,
                };
                if gap.iter().zip(own).any(|(g, o)| (g - o).abs() > 1e-6)
                    && let Some(range) = cloth.pieces.iter().find(|r| r.first_particle == base).copied()
                {
                    let line: Vec<Vec3> =
                        (0..v.len()).map(|k| Vec3::new(at(k).x as f32, at(k).y as f32, 0.0) * 1e-3).collect();
                    let ratio: Vec<f32> = gap.iter().zip(own).map(|(g, o)| (g / o.max(1e-9)) as f32).collect();
                    let depth = (GATHER_DEPTH.max(2.0 * settings.mesh_size) * 1e-3) as f32;
                    cloth.reshape(&range, |a, b| gathered_length(&line, &ratio, depth, a, b));
                }

                // La costura es más rígida a lo largo que la tela sola (la
                // simple es la referencia y se queda como la tela).
                let firm = fabric.bend_stiffness > FIRM_FABRIC;
                let stiffer = if kind.stiffness() > 1.0 { v.len() } else { 0 };
                for k in 2..stiffer {
                    let shrink = (gap[k - 2] + gap[k - 1]) / (own[k - 2] + own[k - 1]).max(1e-9);
                    let rest = (at(k).distance(at(k - 2)) * shrink * 1e-3) as f32;
                    cloth.add_constraint(base + v[k - 2], base + v[k], rest, compliance);
                }
                // El margen pesa y abulta: varias capas de tela juntas. La
                // costura simple (dos capas) es la referencia: la tela ya
                // cuenta con ella, así que solo se suma lo que va de más.
                for k in 0..v.len() {
                    let before = if k > 0 { at(k).distance(at(k - 1)) } else { 0.0 };
                    let after = if k + 1 < v.len() { at(k).distance(at(k + 1)) } else { 0.0 };
                    let spacing = (before + after) / 2.0 * 1e-3;
                    let plies = (kind.layers() - 2.0) / 2.0;
                    let particle = (base + v[k]) as usize;
                    cloth.add_mass(particle, fabric.density * (allowance * spacing) as f32 * plies);
                    cloth.thicken(particle, fabric.thickness * (1.0 + 0.5 * plies));
                }

                let color = egui::Rgba::from(color_of(piece.id));
                let right = [color.r(), color.g(), color.b()];
                // Sobre una línea interior hay tela a los dos lados: no hay
                // un «hacia dentro» ni margen que doblar.
                let on_line = pattern.piece(edge.piece).is_some_and(|p| p.line_index(edge.edge).is_some());
                let inward: Vec<Option<u32>> =
                    (0..v.len()).map(|k| if on_line { None } else { topology.inward(mesh, v, k) }).collect();
                let inward_rest = (0..v.len())
                    .map(|k| {
                        inward[k].map_or(0.0, |n| {
                            (glam::DVec2::from(mesh.positions[n as usize]).distance(at(k)) * 1e-3) as f32
                        })
                    })
                    .collect();
                sides.push(SeamSideDraw {
                    particles: v.iter().map(|&i| base + i).collect(),
                    inward: inward.iter().map(|n| n.map(|i| base + i)).collect(),
                    inward_rest,
                    compliance,
                    firm,
                    thickness: fabric.thickness,
                    allowance: (allowance * kind.visible_allowance()) as f32,
                    right_sign: right_sign(pattern, piece),
                    right_color: right,
                    wrong_color: wrong_side_color(right),
                });
            }

            // Pliegue preferido: las piezas siguen planas (los pespuntes
            // también aplastan la costura) o se doblan una sobre otra.
            let fold = match kind.fold() {
                SeamFold::Free if seam.topstitch != Topstitch::None => SeamFold::Flat,
                fold => fold,
            };
            if fold != SeamFold::Free {
                for (i, a) in sides.iter().enumerate() {
                    for b in sides[i + 1..].iter().filter(|b| !a.firm && !b.firm) {
                        let inward = a.inward.iter().zip(&b.inward);
                        let rests = a.inward_rest.iter().zip(&b.inward_rest);
                        for (((&na, &nb), (&da, &db)), &closed) in inward.zip(rests).zip(&closed) {
                            let (true, Some(na), Some(nb)) = (closed, na, nb) else { continue };
                            let rest = match fold {
                                SeamFold::Flat => da + db,
                                // Una sobre otra, a la distancia que guarda el
                                // choque de la tela consigo misma.
                                _ => (da - db).abs() + 2.0 * a.thickness.max(b.thickness),
                            };
                            cloth.add_constraint(na, nb, rest, (a.compliance + b.compliance) / 2.0 * FOLD_SOFTNESS);
                        }
                    }
                }
            }
            let slider = (kind == SeamKind::Zipper).then(|| closed.iter().position(|&c| c).unwrap_or(closed.len()));
            seam_draws.push(SeamDraw {
                index,
                kind,
                topstitch: seam.topstitch,
                pressed: seam.pressed_side(),
                sides,
                slider,
            });
        }
    }
    // Broches y presillas: unen su par de puntos con la última etapa, como
    // al terminar la prenda. Las fijaciones sujetan el suyo.
    let mut fasteners = Vec::new();
    let last_turn = pattern.sewing_turns(settings.sew_in_order).into_iter().max().unwrap_or(0);
    for fastener in &pattern.fasteners {
        let particle = |end: pattern::MarkRef| {
            let index = pattern.pieces.iter().position(|p| p.id == end.piece)?;
            let mesh = meshed.pieces[index].as_ref().ok()?;
            let &(_, v) = mesh.marks.iter().find(|(id, _)| *id == end.mark)?;
            Some((first_particle[index] + v, right_sign(pattern, &pattern.pieces[index])))
        };
        let (Some((a, sa)), Some((b, sb))) = (particle(fastener.ends[0]), particle(fastener.ends[1])) else {
            continue;
        };
        if settings.sew && fastener.joins() {
            seam_pairs.push(([a, b], last_turn));
        }
        // Una fijación se queda donde esté al terminar de coser.
        if fastener.is_pin() {
            cloth.pin_after_sewing(a as usize);
        }
        fasteners.push(FastenerDraw { kind: fastener.kind, particles: [a, b], right_sign: [sa, sb] });
    }
    for &([a, b], turn) in &seam_pairs {
        cloth.add_seam_in_turn(a, b, turn);
    }

    let mut colliders = vec![Collider::ground(0.0)];
    colliders.extend(settings.obstacle.collider());
    Scene {
        cloth,
        colliders,
        colors,
        back_colors,
        seam_draws,
        failed,
        crossed,
        seams,
        pieces,
        seam_pairs,
        turn_stages,
        fasteners,
    }
}

/// Si dos lados de una costura, tal como quedan colocados, corren en
/// sentidos contrarios: al cerrarse, la tela se retuerce. En los lados casi
/// rectos se mira el sentido de cada uno; en los muy curvos (una sisa, una
/// boca redonda), si cada punto queda mucho más cerca del punto del otro
/// extremo que del suyo.
fn runs_backwards(a: &[Vec3], b: &[Vec3]) -> bool {
    let chord = |s: &[Vec3]| s[s.len() - 1] - s[0];
    let length = |s: &[Vec3]| s.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>();
    let (ca, cb) = (chord(a), chord(b));
    if ca.length() > 0.5 * length(a) && cb.length() > 0.5 * length(b) {
        return ca.dot(cb) < -0.5 * ca.length() * cb.length();
    }
    let straight: f32 = a.iter().zip(b).map(|(p, q)| p.distance(*q)).sum();
    let reversed: f32 = a.iter().zip(b.iter().rev()).map(|(p, q)| p.distance(*q)).sum();
    reversed < straight * 0.7
}

/// Vecinos de cada vértice de una malla y cuáles están en el borde.
struct Topology {
    neighbors: Vec<Vec<u32>>,
    boundary: Vec<bool>,
}

impl Topology {
    fn new(mesh: &Mesh2D) -> Self {
        let mut neighbors = vec![Vec::new(); mesh.positions.len()];
        for [a, b] in mesh.edges() {
            neighbors[a as usize].push(b);
            neighbors[b as usize].push(a);
        }
        Self { neighbors, boundary: mesh.border_flags() }
    }

    /// El vecino del punto `k` del borde `side` que más se mete hacia el
    /// interior de la pieza (en perpendicular al borde).
    fn inward(&self, mesh: &Mesh2D, side: &[u32], k: usize) -> Option<u32> {
        let at = |i: u32| glam::DVec2::from(mesh.positions[i as usize]);
        let tangent = at(side[(k + 1).min(side.len() - 1)]) - at(side[k.saturating_sub(1)]);
        let v = side[k];
        self.neighbors[v as usize]
            .iter()
            .copied()
            .filter(|&n| !self.boundary[n as usize])
            .map(|n| (n, tangent.perp_dot(at(n) - at(v)).abs()))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(n, _)| n)
    }
}

/// Pasa la forma 3D de una escena anterior a una nueva del mismo patrón ya
/// editado: cada pieza que ya existía toma la posición que tenía (aunque su
/// malla haya cambiado) y las piezas nuevas se quedan en su sitio de
/// salida. Las costuras se vuelven a crear a la distancia a la que quedan,
/// así que las que ya estaban cosidas siguen cerradas.
///
/// `old_positions` son las posiciones actuales de la tela vieja.
pub fn carry_over(old: &[ScenePiece], old_positions: &[Vec3], new: &mut Scene) {
    let cloth = &mut new.cloth;
    for piece in &new.pieces {
        let Some(before) = old.iter().find(|p| p.id == piece.id) else { continue };
        let start = before.first_particle as usize;
        let Some(positions) = old_positions.get(start..start + before.mesh.positions.len()) else { continue };
        let remap = simulator::Remap::new(&before.mesh, positions, outline_offset(&before.outline, &piece.outline));
        for (i, &p) in piece.mesh.positions.iter().enumerate() {
            cloth.positions[piece.first_particle as usize + i] = remap.at(p);
        }
    }
    // Las costuras se crean con la distancia que hay al añadirlas: hay que
    // rehacerlas con las posiciones nuevas.
    cloth.clear_seams();
    for &([a, b], turn) in &new.seam_pairs {
        cloth.add_seam_in_turn(a, b, turn);
    }
}

/// Cuánto se ha movido una pieza en el editor: la mediana del movimiento de
/// sus vértices (si solo se ha arrastrado un vértice, la pieza no se ha
/// movido). Si ha cambiado el número de vértices, se supone que nada.
fn outline_offset(before: &[pattern::Point], after: &[pattern::Point]) -> [f64; 2] {
    if before.len() != after.len() || before.is_empty() {
        return [0.0, 0.0];
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    [
        median(before.iter().zip(after).map(|(a, b)| b.x - a.x).collect()),
        median(before.iter().zip(after).map(|(a, b)| b.y - a.y).collect()),
    ]
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

/// Separación (m) entre una capa y la siguiente hacia dentro (el exterior y
/// el forro), al colocarlas alrededor.
const LAYER_SEP: f32 = 0.02;
/// Separación (m) entre una pieza y la pieza sobre la que va (un bolsillo de
/// parche y su panel).
const MOUNT_GAP: f32 = 0.01;

/// Una pieza que va encima de otra ([`Pattern::mount`]): se coloca como su
/// pieza de debajo, con sus puntos llevados encima de donde van cosidos y
/// algo apartada de ella.
struct MountPlace {
    /// Índice de la pieza de debajo en el patrón.
    host: usize,
    /// Giro (coseno, seno) y traslación que llevan un punto de la pieza a
    /// donde cae sobre la de debajo, en el patrón (mm).
    rotation: [f64; 2],
    from: [f64; 2],
    to: [f64; 2],
    /// 1 si va hacia fuera de la de debajo, −1 si hacia dentro.
    outward: f32,
    /// Si la pieza es más larga que lo que separa sus costuras (un asa),
    /// el arco en el que empieza entre ellas.
    arch: Option<Arch>,
}

/// Un asa (o lo que se cose por sus dos extremos y mide más que lo que los
/// separa) empieza en un arco de circunferencia que sale de la pieza de
/// debajo: de largo, el de la pieza, y de cuerda, lo que separa sus
/// costuras.
#[derive(Clone, Copy, Debug)]
struct Arch {
    /// Eje de la pieza (unitario, en su patrón) y dónde empieza y acaba lo
    /// cosido a lo largo de él, desde `MountPlace::from`.
    axis: [f64; 2],
    start: f64,
    /// Radio del arco (mm) y ángulo que abarca (rad).
    radius: f64,
    angle: f64,
}

impl Arch {
    /// Busca el arco de largo `length` cuya cuerda mide `chord`: el ángulo
    /// θ con 2·sen(θ/2)/θ = chord/length.
    fn new(axis: [f64; 2], start: f64, length: f64, chord: f64) -> Option<Self> {
        let ratio = chord / length;
        if !(0.0..0.95).contains(&ratio) {
            return None;
        }
        let (mut lo, mut hi) = (1e-6, 2.0 * std::f64::consts::PI - 1e-6);
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if 2.0 * (mid / 2.0).sin() / mid > ratio { lo = mid } else { hi = mid }
        }
        let angle = (lo + hi) / 2.0;
        Some(Self { axis, start, radius: length / angle, angle })
    }

    /// Para un punto a `u` mm del principio de lo cosido, a lo largo del
    /// eje: dónde cae a lo largo de la cuerda (desde su centro) y a qué
    /// altura sobre la pieza de debajo.
    fn at(&self, u: f64) -> (f64, f64) {
        let half = self.angle / 2.0;
        let phi = (-half + u / self.radius).clamp(-half, half);
        (self.radius * phi.sin(), self.radius * (phi.cos() - half.cos()))
    }
}

impl MountPlace {
    /// Dónde cae un punto de la pieza sobre la de debajo (mm) y a qué
    /// altura de más (m) por el arco.
    fn apply(&self, p: [f64; 2]) -> ([f64; 2], f32) {
        let [c, s] = self.rotation;
        let (mut x, mut y) = (p[0] - self.from[0], p[1] - self.from[1]);
        let mut lift = 0.0;
        if let Some(arch) = &self.arch {
            // A lo largo del eje, el arco; de través, igual.
            let [ax, ay] = arch.axis;
            let (along, across) = (x * ax + y * ay, -x * ay + y * ax);
            let u = along - arch.start;
            let length = arch.radius * arch.angle;
            let (on_chord, height) = arch.at(u.clamp(0.0, length));
            // Lo que pase de los extremos cosidos sigue recto por la cuerda.
            let along = on_chord + u.min(0.0) + (u - length).max(0.0);
            (x, y) = (along * ax - across * ay, along * ay + across * ax);
            lift = (height * 1e-3) as f32;
        }
        ([self.to[0] + c * x - s * y, self.to[1] + s * x + c * y], lift)
    }
}

/// Las piezas que van encima de otra, por su índice en el patrón. La pieza
/// de debajo tiene que estar mallada, y una cadena que vuelve sobre sí misma
/// no cuenta.
fn mount_places(
    pattern: &Pattern,
    meshed: &mesher::PatternMesh,
    meshes: &[(usize, &Piece, &Mesh2D)],
) -> HashMap<usize, MountPlace> {
    let mesh_of = |i: usize| meshes.iter().find(|m| m.0 == i).map(|m| m.2);
    let mut out = HashMap::new();
    for &(index, piece, mesh) in meshes {
        let Some(mount) = pattern.mount(piece.id) else { continue };
        let Some(host) = pattern.pieces.iter().position(|p| p.id == mount.piece) else { continue };
        let Some(host_mesh) = mesh_of(host) else { continue };
        // Los puntos cosidos entre las dos piezas, en cada una.
        let mut pairs = Vec::new();
        for seam in &meshed.seams {
            for a in seam.sides.iter().filter(|s| s.piece == index) {
                for b in seam.sides.iter().filter(|s| s.piece == host) {
                    for (&va, &vb) in a.vertices.iter().zip(&b.vertices) {
                        pairs.push((mesh.positions[va as usize], host_mesh.positions[vb as usize]));
                    }
                }
            }
        }
        let (rotation, from, to) = rigid_fit(&pairs);
        let outward = if pattern.pieces[host].right_side_in == mount.wrong_side { 1.0 } else { -1.0 };
        let arch = arch_of(&pairs, rotation, from, to);
        out.insert(index, MountPlace { host, rotation, from, to, outward, arch });
    }
    // Fuera las cadenas que vuelven sobre sí mismas.
    let looping: Vec<usize> = out
        .keys()
        .copied()
        .filter(|&start| {
            let mut i = start;
            for _ in 0..=out.len() {
                match out.get(&i) {
                    Some(m) if m.host == start => return true,
                    Some(m) => i = m.host,
                    None => return false,
                }
            }
            true
        })
        .collect();
    for i in looping {
        out.remove(&i);
    }
    out
}

/// El arco de una pieza cosida por dos sitios que mide bastante más que lo
/// que los separa (ver [`Arch`]): a lo largo de la dirección en la que más
/// se reparten sus puntos cosidos.
fn arch_of(pairs: &[([f64; 2], [f64; 2])], rotation: [f64; 2], from: [f64; 2], to: [f64; 2]) -> Option<Arch> {
    let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
    for (a, _) in pairs {
        let (x, y) = (a[0] - from[0], a[1] - from[1]);
        (xx, yy, xy) = (xx + x * x, yy + y * y, xy + x * y);
    }
    let theta = 0.5 * (2.0 * xy).atan2(xx - yy);
    let axis = [theta.cos(), theta.sin()];
    let [c, s] = rotation;
    let host_axis = [c * axis[0] - s * axis[1], s * axis[0] + c * axis[1]];
    let span = |points: &mut dyn Iterator<Item = f64>| {
        points.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
    };
    let (start, end) = span(&mut pairs.iter().map(|(a, _)| (a[0] - from[0]) * axis[0] + (a[1] - from[1]) * axis[1]));
    let (h0, h1) = span(&mut pairs.iter().map(|(_, b)| (b[0] - to[0]) * host_axis[0] + (b[1] - to[1]) * host_axis[1]));
    let (length, chord) = (end - start, h1 - h0);
    // Solo si sobra bastante: un bolsillo algo embebido sigue plano.
    if chord < 10.0 || length < 1.2 * chord {
        return None;
    }
    Arch::new(axis, start, length, chord)
}

/// Giro y traslación que mejor llevan los primeros puntos de cada par sobre
/// los segundos (sin espejo ni escala). Sin pares, nada.
fn rigid_fit(pairs: &[([f64; 2], [f64; 2])]) -> ([f64; 2], [f64; 2], [f64; 2]) {
    if pairs.is_empty() {
        return ([1.0, 0.0], [0.0, 0.0], [0.0, 0.0]);
    }
    let n = pairs.len() as f64;
    let mean = |points: &mut dyn Iterator<Item = [f64; 2]>| {
        let s = points.fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
        [s[0] / n, s[1] / n]
    };
    let (from, to) = (mean(&mut pairs.iter().map(|p| p.0)), mean(&mut pairs.iter().map(|p| p.1)));
    let (mut dot, mut cross) = (0.0, 0.0);
    for (a, b) in pairs {
        let (ax, ay, bx, by) = (a[0] - from[0], a[1] - from[1], b[0] - to[0], b[1] - to[1]);
        dot += ax * bx + ay * by;
        cross += ax * by - ay * bx;
    }
    let angle = if dot.hypot(cross) > 1e-9 { cross.atan2(dot) } else { 0.0 };
    ([angle.cos(), angle.sin()], from, to)
}

/// Sigue una pieza que va encima de otra hasta la de debajo del todo: su
/// índice, dónde cae el punto `p` sobre ella y a qué distancia (m, hacia
/// fuera).
fn resolve_mount(mounts: &HashMap<usize, MountPlace>, index: usize, p: [f64; 2]) -> (usize, [f64; 2], f32) {
    let (mut index, mut p, mut offset) = (index, p, 0.0);
    while let Some(m) = mounts.get(&index) {
        let (q, lift) = m.apply(p);
        p = q;
        offset += m.outward * (MOUNT_GAP + lift);
        index = m.host;
    }
    (index, p, offset)
}

/// Grupo de piezas que se colocan juntas: zona del cuerpo, capa y lado.
type Group = (BodyZone, u8, Arrangement);

/// Colocación «alrededor»: las piezas de delante, detrás y los lados se
/// curvan sobre un cilindro, cada grupo centrado en su lado y con la
/// disposición del editor; las de arriba y abajo van perpendiculares al eje,
/// tapando el cilindro.
///
/// Sin maniquí hay un solo cilindro vertical, sobre el obstáculo. Con
/// maniquí, uno por zona del cuerpo ([`BodyZone`]), con el eje de esa zona
/// y un radio que deja algo de holgura sobre el cuerpo.
///
/// Cada capa ([`Piece::layer`]) va en su propio cilindro, uno dentro de otro
/// y con el borde de arriba de sus paredes a la misma altura: el forro
/// queda dentro del exterior, con la boca a la par.
struct AroundLayout {
    /// Centro (mm) de cada grupo de piezas en el patrón.
    group_center: HashMap<Group, [f64; 2]>,
    zones: HashMap<BodyZone, ZoneLayout>,
    /// Con maniquí, cada pieza va en su zona; sin él, todas en una.
    by_zone: bool,
}

struct ZoneLayout {
    frame: ZoneFrame,
    /// Radio (mm) del cilindro de la capa de más dentro.
    radius: f64,
    /// La capa de más dentro.
    inner: u8,
    /// Borde superior (mm, Y hacia abajo) de las piezas laterales de cada
    /// capa, que va en `frame.top`, y su alto (m).
    layers: HashMap<u8, (f64, f32)>,
    /// Alto (m) de las piezas laterales más altas.
    height: f32,
}

impl ZoneLayout {
    /// Radio (mm) del cilindro de una capa.
    fn radius(&self, layer: u8) -> f64 {
        self.radius + f64::from(self.inner.saturating_sub(layer)) * f64::from(LAYER_SEP) * 1e3
    }
}

/// Separación (m) entre las tapas y las piezas laterales.
const CAP_GAP: f32 = 0.05;

fn is_wall(a: Arrangement) -> bool {
    !matches!(a, Arrangement::Top | Arrangement::Bottom)
}

impl AroundLayout {
    fn new(meshes: &[(usize, &Piece, &Mesh2D)], seams: &[pattern::Seam], floor: f32, avatar: Option<&Avatar>) -> Self {
        let by_zone = avatar.is_some();
        let zone_of = |piece: &Piece| if by_zone { piece.zone } else { BodyZone::Torso };
        let mut groups: HashMap<Group, Vec<&Mesh2D>> = HashMap::new();
        for (_, piece, mesh) in meshes {
            groups.entry((zone_of(piece), piece.layer, piece.arrangement)).or_default().push(mesh);
        }
        // Grupos cosidos a sí mismos: un tubo (un cuerpo de una pieza con
        // su costado), que se cierra dando la vuelta entera.
        let group_of: HashMap<pattern::PieceId, Group> =
            meshes.iter().map(|(_, p, _)| (p.id, (zone_of(p), p.layer, p.arrangement))).collect();
        let closed: std::collections::HashSet<Group> = seams
            .iter()
            .filter_map(|seam| {
                let mut sides = seam.sides.iter().map(|s| group_of.get(&s.piece).copied());
                let first = sides.next()??;
                sides.all(|g| g == Some(first)).then_some(first)
            })
            .collect();
        let group_bounds: HashMap<Group, ([f64; 2], [f64; 2])> = groups.iter().map(|(k, m)| (*k, bounds(m))).collect();
        let group_center = group_bounds
            .iter()
            .map(|(k, (min, max))| (*k, [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0]))
            .collect();

        let mut zones = HashMap::new();
        for zone in BodyZone::ALL {
            let in_zone: Vec<(u8, Arrangement)> =
                groups.keys().filter(|(z, ..)| *z == zone).map(|&(_, l, a)| (l, a)).collect();
            if in_zone.is_empty() {
                continue;
            }
            let inner = in_zone.iter().map(|&(l, _)| l).max().unwrap_or(0);
            let mut layers = HashMap::new();
            for &(layer, _) in &in_zone {
                let walls: Vec<&Mesh2D> = groups
                    .iter()
                    .filter(|((z, l, a), _)| *z == zone && *l == layer && is_wall(*a))
                    .flat_map(|(_, m)| m.iter().copied())
                    .collect();
                let (min, max) = bounds(&walls);
                let entry = if walls.is_empty() { (0.0, 0.0) } else { (min[1], ((max[1] - min[1]) * 1e-3) as f32) };
                layers.insert(layer, entry);
            }
            let height = layers.values().map(|&(_, h)| h).fold(0.0, f32::max);
            // Cada grupo ocupa como mucho este ángulo del cilindro, para que
            // quede un hueco entre grupos vecinos. Sobre el cuerpo, una pieza
            // sola (una manga, por ejemplo) casi da la vuelta entera, y
            // también un tubo cosido a sí mismo.
            let wall_groups = |layer: u8| in_zone.iter().filter(|&&(l, a)| l == layer && is_wall(a)).count();
            let lone_walls = layers.keys().all(|&l| wall_groups(l) <= 1);
            let tubes = in_zone.iter().filter(|&&(_, a)| is_wall(a)).all(|&(l, a)| closed.contains(&(zone, l, a)));
            let sides = in_zone.iter().any(|&(_, a)| matches!(a, Arrangement::Left | Arrangement::Right));
            let max_angle = if lone_walls && (by_zone || tubes) {
                1.7
            } else if sides {
                0.42
            } else {
                0.85
            } * std::f64::consts::PI;
            let widest = group_bounds
                .iter()
                .filter(|((z, _, a), _)| *z == zone && is_wall(*a))
                .map(|(_, (min, max))| max[0] - min[0])
                .fold(0.0, f64::max);
            let fit = widest / max_angle;
            let (frame, radius) = match avatar {
                Some(avatar) => {
                    let frame = *avatar.zone(zone);
                    (frame, fit.max(((frame.radius + BODY_CLEARANCE) * 1e3) as f64))
                }
                None => {
                    let has_bottom = in_zone.iter().any(|&(_, a)| a == Arrangement::Bottom);
                    let base = floor + 0.1 + if has_bottom { CAP_GAP } else { 0.0 };
                    let frame = ZoneFrame {
                        top: Vec3::new(0.0, base + height, 0.0),
                        axis: -Vec3::Y,
                        front: Vec3::Z,
                        right: Vec3::X,
                        length: height,
                        radius: 0.0,
                    };
                    (frame, fit.max(100.0))
                }
            };
            zones.insert(zone, ZoneLayout { frame, radius, inner, layers, height });
        }
        Self { group_center, zones, by_zone }
    }

    fn zone(&self, piece: &Piece) -> (BodyZone, &ZoneLayout) {
        let zone = if self.by_zone { piece.zone } else { BodyZone::Torso };
        (zone, &self.zones[&zone])
    }

    /// Ángulo del centro de cada lado alrededor del eje (0 = delante).
    fn angle(a: Arrangement) -> f64 {
        use std::f64::consts::{FRAC_PI_2, PI};
        match a {
            Arrangement::Right => FRAC_PI_2,
            Arrangement::Back => PI,
            Arrangement::Left => -FRAC_PI_2,
            _ => 0.0,
        }
    }

    /// Posición (m) de un punto del patrón de la pieza, `offset` metros
    /// hacia fuera de donde iría.
    fn place(&self, piece: &Piece, p: [f64; 2], offset: f32) -> Vec3 {
        let (zone, layout) = self.zone(piece);
        let a = piece.arrangement;
        let f = &layout.frame;
        let c = self.group_center.get(&(zone, piece.layer, a)).copied().unwrap_or_default();
        let (dx, dy) = (((p[0] - c[0]) * 1e-3) as f32, ((p[1] - c[1]) * 1e-3) as f32);
        let (top, height) = layout.layers.get(&piece.layer).copied().unwrap_or((0.0, layout.height));
        // Las tapas de las capas de dentro, también algo más dentro.
        let inset = f32::from(piece.layer) * LAYER_SEP;
        match a {
            Arrangement::Top => f.top - f.axis * (CAP_GAP - inset + offset) + f.right * dx + f.front * dy,
            Arrangement::Bottom => f.top + f.axis * (height + CAP_GAP - inset + offset) + f.right * dx - f.front * dy,
            _ => {
                // La x del dibujo avanza hacia la derecha de quien mira la pieza.
                let radius = layout.radius(piece.layer);
                let theta = Self::angle(a) + (p[0] - c[0]) / radius;
                let along = ((p[1] - top) * 1e-3) as f32;
                f.top + f.axis * along + f.direction(theta as f32) * ((radius * 1e-3) as f32 + offset)
            }
        }
    }

    fn front(&self, piece: &Piece) -> Vec3 {
        let (_, layout) = self.zone(piece);
        match piece.arrangement {
            Arrangement::Top => -layout.frame.axis,
            Arrangement::Bottom => layout.frame.axis,
            a => layout.frame.direction(Self::angle(a) as f32),
        }
    }
}

/// Colocación para soltar piezas cosidas: como una prenda doblada sobre la
/// mesa, con las de delante arriba (el derecho hacia arriba) y las de
/// detrás debajo, en espejo, justo encima. Así cada costura une puntos que
/// ya están cerca y la tela no tiene que darse la vuelta en el aire para
/// cerrarse (lo que la dejaba arrugada y temblando).
///
/// Las piezas de los lados quedan a izquierda y derecha de las de delante,
/// y las de arriba y abajo, más allá de su borde superior e inferior, todas
/// en la capa de arriba: al coserse se doblan hacia abajo.
///
/// Las capas de dentro (el forro) van entre las de delante y las de detrás
/// del exterior, como dentro de la prenda doblada.
struct FlatLayout {
    /// Centro (mm) de cada grupo de piezas en el patrón.
    center: HashMap<(u8, Arrangement), [f64; 2]>,
    bounds: HashMap<(u8, Arrangement), ([f64; 2], [f64; 2])>,
    /// Borde superior (mm) de las paredes de cada capa en el patrón.
    top: HashMap<u8, f64>,
    /// Alto (mm) de las paredes más altas: su mitad queda en z = 0.
    height: f64,
    /// Ancho y alto (mm) de las piezas de delante de cada capa.
    front_size: HashMap<u8, [f64; 2]>,
    /// La capa de más dentro.
    inner: u8,
}

impl FlatLayout {
    fn new(meshes: &[(usize, &Piece, &Mesh2D)]) -> Self {
        let mut groups: HashMap<(u8, Arrangement), Vec<&Mesh2D>> = HashMap::new();
        for (_, piece, mesh) in meshes {
            groups.entry((piece.layer, piece.arrangement)).or_default().push(mesh);
        }
        let bounds: HashMap<(u8, Arrangement), ([f64; 2], [f64; 2])> =
            groups.iter().map(|(k, m)| (*k, self::bounds(m))).collect();
        let center =
            bounds.iter().map(|(k, (min, max))| (*k, [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0])).collect();
        let layers: Vec<u8> = groups.keys().map(|&(l, _)| l).collect();
        let (mut top, mut front_size, mut height) = (HashMap::new(), HashMap::new(), 0.0f64);
        for &layer in &layers {
            let walls: Vec<&Mesh2D> = groups
                .iter()
                .filter(|((l, a), _)| *l == layer && is_wall(*a))
                .flat_map(|(_, m)| m.iter().copied())
                .collect();
            let (min, max) = self::bounds(&walls);
            if !walls.is_empty() {
                top.insert(layer, min[1]);
                height = height.max(max[1] - min[1]);
            }
            let size = bounds
                .get(&(layer, Arrangement::Front))
                .or_else(|| bounds.get(&(layer, Arrangement::Back)))
                .map_or([0.0, 0.0], |(min, max)| [max[0] - min[0], max[1] - min[1]]);
            front_size.insert(layer, size);
        }
        let inner = layers.iter().copied().max().unwrap_or(0);
        Self { center, bounds, top, height, front_size, inner }
    }

    /// Posición (m) de un punto del patrón, `offset` metros hacia fuera de
    /// donde iría; la capa de abajo está a `floor`.
    fn place(&self, piece: &Piece, p: [f64; 2], floor: f32, offset: f32) -> Vec3 {
        let (a, layer) = (piece.arrangement, piece.layer);
        let c = self.center[&(layer, a)];
        let (min, max) = self.bounds[&(layer, a)];
        let [w, h] = self.front_size[&layer];
        let middle = self.top.get(&layer).copied().unwrap_or(min[1]) + self.height / 2.0;
        let (x, z) = match a {
            Arrangement::Front => (p[0] - c[0], p[1] - middle),
            // Vista desde detrás: la x del dibujo va hacia la izquierda.
            Arrangement::Back => (c[0] - p[0], p[1] - middle),
            Arrangement::Left => (-w / 2.0 - FLAT_GAP - (max[0] - p[0]), p[1] - middle),
            Arrangement::Right => (w / 2.0 + FLAT_GAP + (p[0] - min[0]), p[1] - middle),
            Arrangement::Top => (p[0] - c[0], -h / 2.0 - FLAT_GAP - (max[1] - p[1])),
            Arrangement::Bottom => (p[0] - c[0], h / 2.0 + FLAT_GAP + (p[1] - min[1])),
        };
        // De abajo arriba: las de detrás de fuera hacia dentro y luego las
        // demás de dentro hacia fuera.
        let level = if a == Arrangement::Back {
            f32::from(layer)
        } else {
            f32::from(2 * self.inner + 1 - layer.min(self.inner))
        };
        let y = floor + level * LAYER_GAP + self.front(piece).y * offset;
        Vec3::new((x * 1e-3) as f32, y, (z * 1e-3) as f32)
    }

    fn front(&self, piece: &Piece) -> Vec3 {
        if piece.arrangement == Arrangement::Back { -Vec3::Y } else { Vec3::Y }
    }
}

// ---------------------------------------------------------------- hilo

enum Command {
    /// El último campo dice si es la misma prenda, editada.
    Load(u64, Box<Cloth>, Vec<Collider>, SimParams, bool),
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
    /// Turno de montaje que se está cosiendo.
    turn: Option<u32>,
    /// Presión aplicada (Pa) y volumen encerrado (m³).
    pressure: f32,
    volume: f32,
}

/// Estado de la simulación para mostrarlo.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub sim_time: f32,
    pub step_ms: f32,
    pub sewing: bool,
    /// Turno de montaje que se está cosiendo.
    pub turn: Option<u32>,
    pub pressure: f32,
    pub volume: f32,
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

    /// Carga una tela nueva. Con `keep_time` el reloj sigue contando: es la
    /// misma prenda, editada.
    pub fn load(&mut self, cloth: Cloth, colliders: Vec<Collider>, params: SimParams, keep_time: bool) {
        let generation = {
            let mut shared = self.shared.lock().unwrap();
            shared.generation += 1;
            shared.positions.clone_from(&cloth.positions);
            shared.version += 1;
            if !keep_time {
                shared.sim_time = 0.0;
                shared.step_ms = 0.0;
            }
            shared.sewing = cloth.is_sewing();
            shared.turn = cloth.sewing_turn();
            shared.generation
        };
        let _ = self.commands.send(Command::Load(generation, Box::new(cloth), colliders, params, keep_time));
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
        Stats {
            sim_time: shared.sim_time,
            step_ms: shared.step_ms,
            sewing: shared.sewing,
            turn: shared.turn,
            pressure: shared.pressure,
            volume: shared.volume,
        }
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
            Ok(Command::Load(generation, mut cloth, colliders, params, same)) => {
                // La prenda editada sigue igual de hinchada.
                if same && let Some((_, old, _, _)) = &scene {
                    cloth.set_applied_pressure(old.applied_pressure());
                }
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
        let volume = if cloth.applied_pressure() != 0.0 { cloth.volume() } else { 0.0 };
        {
            let mut shared = shared.lock().unwrap();
            if shared.generation != *generation {
                continue; // ya hay otra tela en camino
            }
            shared.positions.clone_from(&cloth.positions);
            shared.version += 1;
            shared.sim_time += FRAME;
            shared.sewing = cloth.is_sewing();
            shared.turn = cloth.sewing_turn();
            shared.pressure = cloth.applied_pressure();
            shared.volume = volume;
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
    use pattern::{FastenerKind, PieceId, Point, SeamSide};

    /// Dos paneles tumbados y cosidos por un borde con una costura de tipo
    /// `kind`: el de la izquierda (20 cm) se fija y el de la derecha (`width`
    /// mm) queda en el aire. Devuelve la tela tras 3 s y dónde empieza el
    /// panel suelto.
    fn hinge(kind: SeamKind, width: f64) -> (Scene, std::ops::Range<usize>) {
        let mut pattern = Pattern::new("bisagra");
        let a = pattern.add_rect("fijo", pattern::Point::ZERO, 200.0, 200.0);
        let b = pattern.add_rect("suelto", pattern::Point::new(200.0, 0.0), width, 200.0);
        // Al soltar piezas cosidas, la de la derecha se tumba a la derecha de la de delante.
        pattern.piece_mut(b).unwrap().arrangement = Arrangement::Right;
        let ea = pattern.piece(a).unwrap().edges()[1].id;
        let eb = pattern.piece(b).unwrap().edges()[3].id;
        let sides = vec![pattern::SeamSide::new(a, ea, false), pattern::SeamSide::new(b, eb, true)];
        let id = pattern.add_seam("", sides).unwrap();
        pattern.seam_mut(id).unwrap().set_kind(kind);
        let settings = SceneSettings { placement: Placement::Drop, obstacle: Obstacle::Table, ..Default::default() };
        let mut scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
        let (fixed, free) = (&scene.pieces[0], &scene.pieces[1]);
        for i in 0..fixed.mesh.positions.len() {
            scene.cloth.pin(fixed.first_particle as usize + i);
        }
        let free = free.first_particle as usize..free.first_particle as usize + free.mesh.positions.len();
        for _ in 0..180 {
            scene.cloth.step(FRAME, &settings.params(), &[]);
        }
        assert!(scene.cloth.positions.iter().all(|p| p.is_finite()));
        (scene, free)
    }

    /// Cuánto baja (m) la franja del panel suelto junto a la costura (sus
    /// primeros 25 mm) respecto a la altura a la que está la costura.
    fn droop_near_seam(kind: SeamKind) -> f32 {
        let (scene, free) = hinge(kind, 80.0);
        let piece = &scene.pieces[1];
        let near: Vec<f32> = free
            .clone()
            .zip(&piece.mesh.positions)
            .filter(|(_, p)| p[0] - 200.0 < 25.0 && p[0] > 200.0)
            .map(|(i, _)| scene.cloth.positions[i].y)
            .collect();
        // El panel fijo está en horizontal, a la altura de la costura.
        let top = scene.cloth.positions[scene.pieces[0].first_particle as usize].y;
        top - near.iter().sum::<f32>() / near.len() as f32
    }

    /// Un volante de 40 cm cosido a una cinturilla de 20 cm: fruncido, se
    /// encoge hasta lo que mide la cinturilla y ella apenas se estira;
    /// estirado, la cinturilla cede bastante más (las dos telas son iguales).
    #[test]
    fn gathered_seams_keep_the_short_side() {
        let band_length = |fit: SeamFit| {
            let mut pattern = Pattern::new("volante");
            let band = pattern.add_rect("cinturilla", Point::ZERO, 200.0, 60.0);
            let ruffle = pattern.add_rect("volante", Point::new(0.0, 100.0), 400.0, 100.0);
            let bottom = pattern.piece(band).unwrap().edges()[2].id;
            let top = pattern.piece(ruffle).unwrap().edges()[0].id;
            let sides = vec![SeamSide::new(band, bottom, false), SeamSide::new(ruffle, top, true)];
            let id = pattern.add_seam("", sides).unwrap();
            pattern.seam_mut(id).unwrap().fit = fit;
            let settings = SceneSettings { placement: Placement::Drop, ..Default::default() };
            let mut scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
            let colliders = scene.colliders.clone();
            for _ in 0..600 {
                if !scene.cloth.is_sewing() {
                    break;
                }
                scene.cloth.step(FRAME, &settings.params(), &colliders);
            }
            for _ in 0..60 {
                scene.cloth.step(FRAME, &settings.params(), &colliders);
            }
            assert!(!scene.cloth.is_sewing(), "{fit:?}: sigue cosiendo");
            assert!(scene.cloth.positions.iter().all(|p| p.is_finite()), "{fit:?}");
            let gap = scene.cloth.max_seam_gap();
            assert!(gap < 3e-3, "{fit:?}: costura abierta {gap}");
            let line = &scene.seam_draws[0].sides[0].particles;
            line.windows(2)
                .map(|w| scene.cloth.positions[w[0] as usize].distance(scene.cloth.positions[w[1] as usize]))
                .sum::<f32>()
        };
        let (gathered, stretched) = (band_length(SeamFit::Gathered), band_length(SeamFit::Stretched));
        eprintln!("cinturilla: {gathered:.3} m fruncida, {stretched:.3} m estirada");
        assert!(gathered < 0.22, "fruncida: la cinturilla mide {gathered} m");
        assert!(stretched > gathered + 0.02, "estirada: {stretched} m, fruncida {gathered} m");
    }

    /// Un puño con un doblez planchado a 180° empieza doblado por la mitad y
    /// así se queda al caer en la mesa, sin atravesarse; con una bisagra
    /// libre, cae desdoblándose.
    #[test]
    fn pressed_folds_start_folded_and_stay() {
        let depth = |pressed: bool| {
            let mut pattern = Pattern::new("puño");
            let cuff = pattern.add_rect("puño", Point::ZERO, 300.0, 120.0);
            let fold = pattern.add_fold(cuff, Point::new(0.0, 60.0), Point::new(300.0, 60.0)).unwrap();
            let piece = pattern.piece_mut(cuff).unwrap();
            let k = piece.line_index(fold).unwrap();
            piece.lines[k].fold.as_mut().unwrap().pressed = pressed;
            let settings = SceneSettings { placement: Placement::Drop, ..Default::default() };
            let mut scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
            let colliders = scene.colliders.clone();
            let span = |scene: &Scene| {
                let z = scene.cloth.positions.iter().map(|p| p.z);
                z.clone().fold(f32::NEG_INFINITY, f32::max) - z.fold(f32::INFINITY, f32::min)
            };
            let start = span(&scene);
            for _ in 0..240 {
                scene.cloth.step(FRAME, &settings.params(), &colliders);
            }
            assert!(scene.cloth.positions.iter().all(|p| p.is_finite()));
            let crossings = scene.cloth.count_crossings();
            (start, span(&scene), crossings)
        };
        let (start, end, crossings) = depth(true);
        eprintln!("planchado: de {start:.3} m a {end:.3} m de fondo, {crossings} cruces");
        assert!(start < 0.07, "empieza doblado: {start}");
        assert!(end < 0.075, "sigue doblado: {end}");
        assert_eq!(crossings, 0);
    }

    /// Una costura simple hace de bisagra y el panel suelto cuelga junto a
    /// ella; una inglesa (plana y rígida) lo sostiene mucho más.
    #[test]
    fn flat_felled_seams_stay_flat() {
        let (plain, flat) = (droop_near_seam(SeamKind::Plain), droop_near_seam(SeamKind::FlatFelled));
        eprintln!("junto a la costura baja {plain:.4} m con la simple y {flat:.4} m con la inglesa");
        assert!(plain > 0.01, "{plain}");
        assert!(flat < plain * 0.5, "{flat} ≥ {plain} / 2");
    }

    /// Una costura volteada dobla una pieza sobre la otra: el panel suelto
    /// se mete hacia el fijo en vez de colgar recto.
    #[test]
    fn bagged_seams_fold_back() {
        let reach = |kind| {
            let (scene, free) = hinge(kind, 80.0);
            // Hasta dónde llega el panel suelto hacia el fijo, desde la
            // costura (el borde derecho del panel fijo).
            let fixed = &scene.pieces[0];
            let start = fixed.first_particle as usize;
            let seam = scene.cloth.positions[start..start + fixed.mesh.positions.len()]
                .iter()
                .map(|p| p.x)
                .fold(f32::NEG_INFINITY, f32::max);
            scene.cloth.positions[free].iter().map(|p| p.x).fold(f32::INFINITY, f32::min) - seam
        };
        let (plain, bagged) = (reach(SeamKind::Plain), reach(SeamKind::Bagged));
        eprintln!("el panel suelto llega a {plain:.3} m de la costura con la simple y a {bagged:.3} m con la volteada");
        assert!(bagged < plain - 0.02, "{bagged} ≥ {plain} - 0,02");
    }

    fn build_example(make: fn() -> Pattern, settings: &SceneSettings) -> Scene {
        build(&make(), settings, |_| egui::Color32::WHITE)
    }

    /// Como en la aplicación: las prendas, sobre el maniquí.
    fn settings_for(make: fn() -> Pattern, settings: SceneSettings) -> SceneSettings {
        if make().on_avatar { SceneSettings { obstacle: Obstacle::Mannequin, ..settings } } else { settings }
    }

    /// La tote con forro aparte: cada capa en su cilindro, el forro dentro
    /// del exterior y el bolsillo dentro del forro. Cosida y asentada no se
    /// cruza (con el forro en la misma capa que el exterior, como antes de
    /// las capas, sí).
    #[test]
    fn lining_starts_and_stays_inside() {
        let lined = examples::lined_tote_bag();
        let settings = SceneSettings { mesh_size: 30.0, ..Default::default() };
        let radius = |scene: &Scene, name: &str| {
            let index = lined.pieces.iter().position(|p| p.name == name).unwrap();
            let range = &scene.cloth.pieces[index];
            let start = range.first_particle as usize;
            scene.cloth.positions[start..start + range.particle_count as usize]
                .iter()
                .map(|p| p.x.hypot(p.z))
                .fold((f32::INFINITY, 0.0f32), |(lo, hi), r| (lo.min(r), hi.max(r)))
        };
        let scene = build(&lined, &settings, |_| egui::Color32::WHITE);
        assert!(scene.failed.is_empty() && scene.crossed.is_empty(), "{:?} {:?}", scene.failed, scene.crossed);
        let (outer, lining, pocket) =
            (radius(&scene, "Delantero"), radius(&scene, "Forro delantero"), radius(&scene, "Bolsillo"));
        assert!(lining.1 + 0.015 < outer.0, "forro {lining:?}, exterior {outer:?}");
        assert!(pocket.1 + 0.005 < lining.0, "bolsillo {pocket:?}, forro {lining:?}");

        let settle = |pattern: &Pattern| {
            let mut scene = build(pattern, &settings, |_| egui::Color32::WHITE);
            for _ in 0..300 {
                scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
            }
            assert!(scene.cloth.positions.iter().all(|p| p.is_finite()));
            (scene.cloth.count_crossings(), scene.cloth.max_seam_gap())
        };
        let (crossings, gap) = settle(&lined);
        let mut flat = lined.clone();
        for piece in &mut flat.pieces {
            piece.layer = 0;
        }
        let (before, _) = settle(&flat);
        eprintln!("tote con forro: {crossings} aristas cruzadas ({before} con todo en la misma capa), costura {gap}");
        assert!(gap < 5e-3, "costura abierta {gap}");
        assert!(crossings <= 5, "{crossings} aristas cruzadas ({before} en una capa)");
    }

    /// Un bolsillo de parche cosido sobre tres líneas interiores del
    /// delantero de la tote (los laterales y el fondo; la boca queda
    /// abierta), y una ventana recortada en el trasero, tapada por dentro
    /// con otra pieza: la escena se monta y las costuras cierran.
    #[test]
    fn pockets_on_lines_and_holes_sew() {
        let mut p = examples::tote_bag();
        let (front, back) = (p.pieces[0].id, p.pieces[1].id);
        let corner = |x: f64, y: f64| Point::new(x, y);
        let left = p.add_line(front, corner(110.0, 120.0), corner(110.0, 300.0)).unwrap();
        let bottom = p.add_line(front, corner(110.0, 300.0), corner(270.0, 300.0)).unwrap();
        let right = p.add_line(front, corner(270.0, 300.0), corner(270.0, 120.0)).unwrap();
        // El bolsillo, dibujado justo encima de donde va.
        let pocket = p.add_rect("Bolsillo", corner(110.0, 120.0), 160.0, 180.0);
        let e = |p: &Pattern, id: PieceId, i: usize| p.piece(id).unwrap().edges()[i].id;
        let (pl, pb, pr) = (e(&p, pocket, 3), e(&p, pocket, 2), e(&p, pocket, 1));
        p.add_seam("Bolsillo izdo.", vec![SeamSide::new(pocket, pl, true), SeamSide::new(front, left, false)]);
        p.add_seam("Bolsillo fondo", vec![SeamSide::new(pocket, pb, true), SeamSide::new(front, bottom, false)]);
        p.add_seam("Bolsillo dcho.", vec![SeamSide::new(pocket, pr, true), SeamSide::new(front, right, false)]);
        let window = p.add_rect("Ventana", corner(550.0, 100.0), 180.0, 30.0);
        p.piece_mut(window).unwrap().hole_in = Some(back);
        p.piece_mut(window).unwrap().arrangement = Arrangement::Back;
        let facing = p.add_rect("Vista", corner(900.0, 0.0), 180.0, 30.0);
        p.piece_mut(facing).unwrap().arrangement = Arrangement::Back;
        let (wt, ft) = (e(&p, window, 0), e(&p, facing, 0));
        p.add_seam("Ventana", vec![SeamSide::new(window, wt, false), SeamSide::new(facing, ft, true)]);
        assert!(p.validate().is_empty(), "{:?}", p.validate());

        let settings = SceneSettings { mesh_size: 30.0, ..Default::default() };
        let mut scene = build(&p, &settings, |_| egui::Color32::WHITE);
        assert!(scene.failed.is_empty(), "{:?}", scene.failed);
        assert_eq!(scene.seams, p.seams.len());
        assert_eq!(scene.pieces.len(), 4, "el agujero no es tela");
        for _ in 0..240 {
            scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
        }
        assert!(scene.cloth.positions.iter().all(|q| q.is_finite()));
        assert!(!scene.cloth.is_sewing());
        let gap = scene.cloth.max_seam_gap();
        assert!(gap < 5e-3, "costura abierta {gap}");
    }

    /// Ninguna costura de los ejemplos se retuerce, se coloquen como se
    /// coloquen; con un lado al revés, sí, y se avisa.
    #[test]
    fn crossed_seams_are_reported() {
        for (name, _, make) in examples::ALL {
            for placement in Placement::ALL {
                let settings = settings_for(make, SceneSettings { placement, mesh_size: 30.0, ..Default::default() });
                let scene = build_example(make, &settings);
                assert!(scene.crossed.is_empty(), "{name}, {}: {:?}", placement.label(), scene.crossed);
            }
        }
        // En las bolsas, cualquier costura con un lado al revés se ve.
        for make in
            [examples::tote_bag, examples::box_bag, examples::leather_bag, examples::cushion, examples::chalk_bag]
        {
            let base = make();
            for k in 0..base.seams.len() {
                let mut pattern = base.clone();
                let seam = &mut pattern.seams[k];
                seam.sides[1].reversed = !seam.sides[1].reversed;
                let id = seam.id;
                let settings = SceneSettings { mesh_size: 30.0, ..Default::default() };
                let scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
                assert_eq!(scene.crossed, [id], "{}, «{}»", base.name, base.seams[k].name);
            }
        }
    }

    /// Colocadas alrededor, los dos lados de cada costura de los ejemplos
    /// corren en el mismo sentido: al revés, los puntos quedarían más lejos.
    #[test]
    fn example_seams_line_up_when_placed_around() {
        for (name, _, make) in examples::ALL {
            let settings = settings_for(make, SceneSettings::default());
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

    /// La camiseta, cosida sobre el maniquí: se cierra, se apoya en los
    /// hombros sin caerse al suelo y ninguna partícula acaba dentro del
    /// cuerpo.
    #[test]
    fn t_shirt_dresses_the_mannequin() {
        let settings = SceneSettings { obstacle: Obstacle::Mannequin, mesh_size: 25.0, ..Default::default() };
        let mut scene = build_example(examples::t_shirt, &settings);
        let body = settings.obstacle.collider().unwrap();
        let start = Instant::now();
        for _ in 0..300 {
            scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
        }
        eprintln!("{} partículas, {:?} por paso", scene.cloth.positions.len(), start.elapsed() / 300);
        assert!(scene.cloth.positions.iter().all(|p| p.is_finite()));
        assert!(!scene.cloth.is_sewing());
        let gap = scene.cloth.max_seam_gap();
        assert!(gap < 5e-3, "costura abierta {gap}");
        let deepest = scene.cloth.positions.iter().map(|p| body.signed_distance(*p).0).fold(f32::INFINITY, f32::min);
        assert!(deepest > -0.005, "partícula dentro del maniquí: {deepest}");
        let (lowest, highest) = scene
            .cloth
            .positions
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
        eprintln!("de {lowest} a {highest} m");
        assert!(highest > 1.35 && lowest > 0.5, "se ha caído: de {lowest} a {highest}");
        let speed = scene.cloth.velocities.iter().map(|v| v.length()).fold(0.0, f32::max);
        assert!(speed < 0.1, "sigue moviéndose: {speed}");
    }

    /// Cosidos y soltados sobre la mesa, los ejemplos se cierran y quedan
    /// quietos sin que nada explote.
    #[test]
    fn examples_sew_and_settle() {
        for (name, _, make) in examples::ALL {
            let settings = settings_for(make, SceneSettings { mesh_size: 30.0, ..Default::default() });
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

    /// Los ejemplos, cosidos y asentados (las bolsas, caídas sobre la mesa):
    /// la tela no se atraviesa a sí misma. Sin auto-colisión, las dos caras
    /// de las bolsas blandas se cruzan.
    #[test]
    fn examples_do_not_pass_through_themselves() {
        let crossings = |make: fn() -> Pattern, self_collision: bool| {
            let settings = settings_for(make, SceneSettings { mesh_size: 30.0, self_collision, ..Default::default() });
            let mut scene = build_example(make, &settings);
            for _ in 0..300 {
                scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
            }
            assert!(scene.cloth.positions.iter().all(|p| p.is_finite()));
            scene.cloth.count_crossings()
        };
        for (name, _, make) in examples::ALL {
            let (with, without) = (crossings(make, true), crossings(make, false));
            eprintln!("{name}: {with} aristas cruzadas ({without} sin auto-colisión)");
            // Las bolsas firmes (la de fondo, la de cuero) no se aplastan, así
            // que sin auto-colisión tampoco se cruzan; se toleran unos pocos
            // cruces en las esquinas donde se juntan tres piezas.
            assert!(with <= (without / 5).max(10), "{name}: {with} aristas cruzadas ({without} sin auto-colisión)");
        }
    }

    /// Con relleno, la bolsa tote y el cojín se hinchan sobre la mesa: mucho
    /// más volumen que vacíos, sin atravesar la mesa ni a sí mismos, y se
    /// quedan encima. (Con mucha más presión la bolsa se pone redonda como
    /// un globo y puede rodar.)
    #[test]
    fn pressure_gives_volume_on_the_table() {
        let run = |make: fn() -> Pattern, pressure: f64| {
            let mut pattern = make();
            pattern.pressure = pressure;
            let settings = SceneSettings { mesh_size: 30.0, ..Default::default() };
            let mut scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
            let params = SimParams { pressure: pressure as f32, ..settings.params() };
            for _ in 0..600 {
                scene.cloth.step(FRAME, &params, &scene.colliders);
            }
            assert!(scene.cloth.positions.iter().all(|p| p.is_finite()));
            let table = settings.obstacle.collider().unwrap();
            let deepest =
                scene.cloth.positions.iter().map(|p| table.signed_distance(*p).0).fold(f32::INFINITY, f32::min);
            assert!(deepest > -1e-3, "{}: atraviesa la mesa ({deepest})", pattern.name);
            // Vacío, el cojín es dos láminas cosidas que caen como una tabla
            // y pueden asomar por el borde; lleno, se queda encima.
            let lowest = scene.cloth.positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            assert!(
                pressure == 0.0 || lowest > 2.0 * TABLE_HALF.y - 0.01,
                "{}: se ha caído de la mesa ({lowest})",
                pattern.name
            );
            let speed = scene.cloth.velocities.iter().map(|v| v.length()).fold(0.0, f32::max);
            (scene.cloth.volume(), scene.cloth.count_crossings(), speed)
        };
        for (make, pressure) in [(examples::tote_bag as fn() -> Pattern, 10.0), (examples::cushion, 30.0)] {
            let (empty, _, _) = run(make, 0.0);
            let (full, crossings, speed) = run(make, pressure);
            let name = make().name;
            eprintln!(
                "{name}: {:.1} l vacía, {:.1} l llena, {crossings} cruces, {speed:.3} m/s",
                empty * 1e3,
                full * 1e3
            );
            assert!(full > 3.0 * empty.max(1e-3) && full > 0.008, "{name}: {empty} → {full}");
            assert!(crossings < 20, "{name}: {crossings} aristas cruzadas");
            assert!(speed < 0.1, "{name}: sigue moviéndose a {speed} m/s");
        }
    }

    /// Colgados, los ejemplos se cosen en el aire y luego se quedan
    /// colgando de su parte de arriba, quietos y sin caerse ni cruzarse.
    /// (Antes se fijaban los bordes de arriba de cada pieza antes de coser y
    /// las costuras tiraban de puntos que no se podían mover.)
    #[test]
    fn examples_hang_from_their_top() {
        for (name, _, make) in examples::ALL {
            let settings =
                settings_for(make, SceneSettings { placement: Placement::Hang, mesh_size: 30.0, ..Default::default() });
            let mut scene = build_example(make, &settings);
            for _ in 0..360 {
                scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
            }
            let cloth = &mut scene.cloth;
            assert!(cloth.positions.iter().all(|p| p.is_finite()), "{name}");
            assert!(!cloth.is_sewing(), "{name}: sigue cosiendo");
            let gap = cloth.max_seam_gap();
            assert!(gap < 5e-3, "{name}: costura abierta {gap}");
            let pinned = cloth.inv_mass.iter().filter(|&&w| w == 0.0).count();
            assert!(pinned >= 4, "{name}: cuelga de {pinned} puntos");
            let (lowest, highest) = cloth
                .positions
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
            assert!((highest - HANG_HEIGHT).abs() < 0.03, "{name}: arriba a {highest} m");
            assert!(lowest > 0.5, "{name}: llega hasta {lowest} m");
            // Como al soltarlos: donde un asa firme se apoya en el panel junto
            // a su costura quedan unos pocos puntos vibrando en cada subpaso,
            // sin moverse de un fotograma a otro.
            let mut speeds: Vec<f32> = cloth.velocities.iter().map(|v| v.length()).collect();
            speeds.sort_by(f32::total_cmp);
            let (most, max) = (speeds[speeds.len() * 99 / 100], speeds[speeds.len() - 1]);
            assert!(most < 0.1 && max < 0.6, "{name}: sigue moviéndose ({most} m/s, máx. {max})");
            let crossings = cloth.count_crossings();
            assert!(crossings < 5, "{name}: {crossings} aristas cruzadas");
        }
    }

    /// Abierta a medias, la cremallera del bolso se queda abierta del
    /// tirador al principio y cerrada del tirador al final (con un poco de
    /// relleno, para que la parte abierta se separe); las asas, que
    /// empiezan en arco, se cosen a sus líneas.
    #[test]
    fn zippers_open_and_close() {
        let mut pattern = examples::zipper_bag();
        let teeth = pattern.seams.iter().position(|s| s.kind == SeamKind::Zipper).unwrap();
        pattern.seams[teeth].open = 0.5;
        let settings = SceneSettings { mesh_size: 30.0, ..Default::default() };
        let mut scene = build(&pattern, &settings, |_| egui::Color32::GRAY);
        let params = SimParams { pressure: 10.0, ..settings.params() };
        for _ in 0..360 {
            scene.cloth.step(FRAME, &params, &scene.colliders);
        }
        let draw = scene.seam_draws.iter().find(|d| d.index == teeth).unwrap();
        let (a, b) = (&draw.sides[0].particles, &draw.sides[1].particles);
        let at = |v: u32| scene.cloth.positions[v as usize];
        let gaps: Vec<f32> = a.iter().zip(b).map(|(&p, &q)| at(p).distance(at(q))).collect();
        let slider = draw.slider.unwrap();
        assert!(slider > 0 && slider < gaps.len(), "tirador en {slider} de {}", gaps.len());
        let open = gaps[..slider - 1].iter().copied().fold(0.0, f32::max);
        let closed = gaps[slider..].iter().copied().fold(0.0, f32::max);
        assert!(closed < 5e-3, "la parte cerrada se separa {closed} m");
        assert!(open > 0.01, "la parte abierta no se abre ({open} m)");
        assert!(!scene.cloth.is_sewing(), "sigue cosiendo");
        assert!(scene.cloth.max_seam_gap() < 5e-3, "costura abierta {}", scene.cloth.max_seam_gap());
    }

    /// Un broche cerrado junta sus dos puntos (aquí, una solapa que cae
    /// sobre el panel); abierto, los deja sueltos.
    #[test]
    fn closed_snaps_join_their_points() {
        let mut pattern = Pattern::new("Solapa con broche");
        let panel = pattern.add_rect("Panel", Point::ZERO, 200.0, 200.0);
        let flap = pattern.add_rect("Solapa", Point::new(250.0, 0.0), 200.0, 100.0);
        let edge = |p: &Pattern, id: PieceId, i: usize| p.piece(id).unwrap().edges()[i].id;
        let sides = vec![
            SeamSide::new(panel, edge(&pattern, panel, 0), false),
            SeamSide::new(flap, edge(&pattern, flap, 0), false),
        ];
        pattern.add_seam("Arriba", sides).unwrap();
        let snap = pattern
            .add_fastener(FastenerKind::Snap, (panel, Point::new(100.0, 160.0)), (flap, Point::new(350.0, 80.0)))
            .unwrap();
        let settings = SceneSettings { mesh_size: 20.0, ..Default::default() };
        let gap = |pattern: &Pattern| {
            let mut scene = build(pattern, &settings, |_| egui::Color32::GRAY);
            assert_eq!(scene.fasteners.len(), 1);
            for _ in 0..240 {
                scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
            }
            let [a, b] = scene.fasteners[0].particles;
            scene.cloth.positions[a as usize].distance(scene.cloth.positions[b as usize])
        };
        let closed = gap(&pattern);
        assert!(closed < 5e-3, "el broche cerrado deja {closed} m");
        pattern.fastener_mut(snap).unwrap().open = true;
        let open = gap(&pattern);
        assert!(open > 0.03, "el broche abierto se ha cerrado ({open} m)");
    }

    /// Sin coser, cada pieza cuelga por separado de su borde de arriba.
    #[test]
    fn loose_pieces_hang_each_from_its_top() {
        let settings = SceneSettings { placement: Placement::Hang, sew: false, mesh_size: 30.0, ..Default::default() };
        let mut scene = build_example(examples::t_shirt_with_sleeves, &settings);
        for _ in 0..120 {
            scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
        }
        for range in &scene.cloth.pieces {
            let piece = range.first_particle as usize..(range.first_particle + range.particle_count) as usize;
            assert!(scene.cloth.inv_mass[piece.clone()].contains(&0.0), "pieza sin colgar");
            let lowest = scene.cloth.positions[piece].iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            assert!(lowest > 0.3, "una pieza se ha caído hasta {lowest} m");
        }
    }

    /// Soltados, los objetos caen sobre la mesa ya cosidos y se quedan
    /// encima, quietos (antes se cosían en plano, dándose la vuelta en el
    /// aire, y quedaban arrugados y temblando); las prendas caen sobre los
    /// hombros del maniquí.
    #[test]
    fn dropped_examples_settle() {
        for (name, _, make) in examples::ALL {
            let settings =
                settings_for(make, SceneSettings { placement: Placement::Drop, mesh_size: 30.0, ..Default::default() });
            let pattern = make();
            let mut scene = build_example(make, &settings);
            let params = SimParams { pressure: pattern.pressure as f32, ..settings.params() };
            for _ in 0..480 {
                scene.cloth.step(FRAME, &params, &scene.colliders);
            }
            let cloth = &mut scene.cloth;
            assert!(cloth.positions.iter().all(|p| p.is_finite()), "{name}");
            let gap = cloth.max_seam_gap();
            assert!(gap < 5e-3, "{name}: costura abierta {gap}");
            let (lowest, highest) = cloth
                .positions
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
            if pattern.on_avatar {
                assert!(highest > 1.35 && lowest > 0.5, "{name}: se ha caído del maniquí ({lowest}..{highest})");
            } else {
                assert!(lowest > 2.0 * TABLE_HALF.y - 0.01, "{name}: se ha caído de la mesa ({lowest})");
            }
            // Donde la tela se dobla contra la mesa (en las esquinas de la
            // bolsa con fondo) quedan algunos puntos vibrando un poco.
            let mut speeds: Vec<f32> = cloth.velocities.iter().map(|v| v.length()).collect();
            speeds.sort_by(f32::total_cmp);
            let (most, max) = (speeds[speeds.len() * 99 / 100], speeds[speeds.len() - 1]);
            assert!(most < 0.1 && max < 0.6, "{name}: sigue moviéndose ({most} m/s, máx. {max})");
            let crossings = cloth.count_crossings();
            assert!(crossings < 5, "{name}: {crossings} aristas cruzadas");
        }
    }

    /// Con la camiseta ya puesta en el maniquí, se edita el patrón: la
    /// prenda nueva empieza donde estaba la vieja y se reajusta sin caerse.
    #[test]
    fn edits_carry_over_the_draped_garment() {
        let settings = SceneSettings { obstacle: Obstacle::Mannequin, mesh_size: 25.0, ..Default::default() };
        let mut pattern = examples::t_shirt();
        let mut scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
        for _ in 0..300 {
            scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
        }
        let extent = |positions: &[Vec3]| {
            positions.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)))
        };
        let (lo, hi) = extent(&scene.cloth.positions);
        let front = pattern.pieces[0].id;

        // Escote más profundo, el delantero movido en el editor y una malla
        // más fina: la prenda queda igual, con las costuras cerradas.
        let piece = pattern.piece_mut(front).unwrap();
        piece.set_edge_kind(
            0,
            pattern::EdgeKind::Cubic { h1: pattern::Vec2::new(5.0, 160.0), h2: pattern::Vec2::new(-5.0, 160.0) },
        );
        piece.translate(pattern::Vec2::new(-200.0, 50.0));
        let settings = SceneSettings { mesh_size: 20.0, ..settings };
        let mut edited = build(&pattern, &settings, |_| egui::Color32::WHITE);
        carry_over(&scene.pieces, &scene.cloth.positions, &mut edited);
        assert!(edited.cloth.positions.len() > scene.cloth.positions.len());
        let gap = edited.cloth.max_seam_gap();
        assert!(gap < 0.01, "las costuras se han abierto al pasar la prenda: {gap}");
        let (new_lo, new_hi) = extent(&edited.cloth.positions);
        assert!(
            (new_lo - lo).abs() < 0.01 && (new_hi - hi).abs() < 0.01,
            "se ha movido: {lo}..{hi} → {new_lo}..{new_hi}"
        );

        // Bajo del delantero 6 cm más largo: los costados ya no miden lo
        // mismo y se vuelven a coser; la prenda sigue puesta y baja.
        let piece = pattern.piece_mut(front).unwrap();
        for i in [4, 5] {
            let v = piece.vertices()[i];
            piece.set_vertex(i, v + pattern::Vec2::new(0.0, 60.0));
        }
        let mut longer = build(&pattern, &settings, |_| egui::Color32::WHITE);
        carry_over(&edited.pieces, &edited.cloth.positions, &mut longer);
        for _ in 0..180 {
            longer.cloth.step(FRAME, &settings.params(), &longer.colliders);
        }
        assert!(longer.cloth.positions.iter().all(|p| p.is_finite()));
        assert!(longer.cloth.max_seam_gap() < 5e-3, "costura abierta: {}", longer.cloth.max_seam_gap());
        let (new_lo, new_hi) = extent(&longer.cloth.positions);
        assert!(new_hi > 1.35, "se ha caído: {new_hi}");
        assert!(new_lo < lo - 0.02, "el bajo no ha bajado: {lo} → {new_lo}");
    }

    /// Largo en 3D (m) de la línea de partículas de una pieza que están a
    /// la altura `y` (mm) en el patrón, ordenadas por x y cerrada.
    fn ring_length(scene: &Scene, piece: usize, y: f64) -> f32 {
        let sp = &scene.pieces[piece];
        let mut row: Vec<(f64, u32)> = sp
            .mesh
            .positions
            .iter()
            .enumerate()
            .filter(|(_, p)| (p[1] - y).abs() < 0.5)
            .map(|(i, p)| (p[0], sp.first_particle + i as u32))
            .collect();
        row.sort_by(|a, b| a.0.total_cmp(&b.0));
        let x = |i: u32| scene.cloth.positions[i as usize];
        let open: f32 = row.windows(2).map(|w| x(w[0].1).distance(x(w[1].1))).sum();
        open + x(row[0].1).distance(x(row[row.len() - 1].1))
    }

    /// El puño de punto del chalk bag (341 mm) se cose estirado a la boca
    /// del cuerpo de lona (381 mm): el punto cede hasta el largo de la boca
    /// y la lona no se frunce para salirle al encuentro.
    #[test]
    fn knit_cuff_stretches_onto_the_mouth() {
        let pattern = examples::chalk_bag();
        let settings = SceneSettings { mesh_size: 20.0, ..Default::default() };
        let mut scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
        for _ in 0..360 {
            scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
        }
        assert!(!scene.cloth.is_sewing(), "sigue cosiendo");
        let rows = |i: usize| {
            let ys = scene.pieces[i].mesh.positions.iter().map(|p| p[1]);
            ys.clone().fold(f64::INFINITY, f64::min)..=ys.fold(f64::NEG_INFINITY, f64::max)
        };
        let (body, cuff) = (rows(0), rows(1));
        let mouth = ring_length(&scene, 0, *body.start()) * 1e3;
        let sewn = ring_length(&scene, 1, *cuff.end()) * 1e3;
        // El fondo va cosido a la base redonda (π × 121 mm, unos 380).
        let bottom = ring_length(&scene, 0, *body.end()) * 1e3;
        assert!(mouth > 370.0, "la boca se encoge hasta {mouth} mm");
        assert!(sewn > 370.0, "el puño se queda en {sewn} mm (341 sin estirar)");
        assert!((mouth - bottom).abs() < 15.0, "boca {mouth} mm y fondo {bottom} mm");
        assert_eq!(scene.cloth.count_crossings(), 0);
    }

    /// Una fijación sujeta su punto donde quede al terminar de coser, y la
    /// bolsa se queda colgando de él.
    #[test]
    fn pins_hold_their_point() {
        let mut pattern = examples::tote_bag();
        let front = pattern.pieces[0].id;
        pattern.add_pin(front, Point::new(190.0, 30.0)).unwrap();
        let settings = SceneSettings { obstacle: Obstacle::None, mesh_size: 30.0, ..Default::default() };
        let mut scene = build(&pattern, &settings, |_| egui::Color32::WHITE);
        assert!(scene.cloth.inv_mass.iter().all(|&w| w > 0.0), "nada fijo mientras se cose");
        let mut held = None;
        for _ in 0..240 {
            scene.cloth.step(FRAME, &settings.params(), &scene.colliders);
            if held.is_none() && !scene.cloth.is_sewing() {
                let i = scene.cloth.inv_mass.iter().position(|&w| w == 0.0).expect("la fijación sujeta un punto");
                held = Some((i, scene.cloth.positions[i]));
            }
        }
        let (i, at) = held.expect("termina de coser");
        assert_eq!(scene.cloth.positions[i], at, "el punto fijo se ha movido");
        assert_eq!(scene.cloth.inv_mass.iter().filter(|&&w| w == 0.0).count(), 1);
        let lowest = scene.cloth.positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        assert!(at.y - lowest > 0.3, "la bolsa no cuelga: {} m por debajo", at.y - lowest);
    }
}
