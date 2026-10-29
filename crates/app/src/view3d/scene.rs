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
use pattern::{Arrangement, BodyZone, Pattern, Piece, SeamFold, SeamKind, Textile, Topstitch};
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
        // Escalas logarítmicas: de 10⁻⁷ a 10⁻³ N·m y de 10⁻⁹ a 10⁻⁵ m/N.
        bend_stiffness: (1e-7 * 10f64.powf(textile.firmness / 25.0)) as f32,
        stretch_compliance: (1e-9 * 10f64.powf(textile.stretch / 25.0)) as f32,
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
/// tenía el algodón antes de la flexión de Bergou.
fn seam_compliance(fabric: &Fabric, kind: SeamKind) -> f32 {
    const REFERENCE: f32 = 1e-8;
    REFERENCE / (fabric.bend_stiffness.max(1e-9) * kind.stiffness())
}

/// Firmeza (N·m) a partir de la cual la tela manda sobre la costura (del
/// vaquero para arriba): ahí la costura no añade rigidez, peso ni pliegue.
/// Sumarlos a una tela tan firme como el cuero chocaba con su flexión y con
/// la auto-colisión y la tela se disparaba.
const FIRM_FABRIC: f32 = 2e-5;
/// El pliegue preferido es una tendencia, no una bisagra rígida: cede
/// bastante más que la rigidez a lo largo de la costura.
const FOLD_SOFTNESS: f32 = 4.0;

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
    let around = match settings.placement {
        Placement::Around => true,
        Placement::Hang => sewn,
        Placement::Drop => sewn && on_body,
    }
    .then(|| AroundLayout::new(&meshes, settings.obstacle.top(), avatar.as_deref()));
    let flat = (settings.placement == Placement::Drop && sewn && !on_body).then(|| FlatLayout::new(&meshes));
    let hang_z = settings.obstacle.front() + if settings.obstacle == Obstacle::None { 0.0 } else { 0.1 };
    // Al colgar la pieza montada, se sube hasta `HANG_HEIGHT` y se aparta
    // hasta quedar delante del obstáculo.
    let lift = match &around {
        Some(layout) if settings.placement == Placement::Hang => {
            let placed =
                meshes.iter().flat_map(|&(_, piece, mesh)| mesh.positions.iter().map(|&p| layout.place(piece, p)));
            let (top, back) = placed.fold((f32::NEG_INFINITY, f32::INFINITY), |(t, b), p| (t.max(p.y), b.min(p.z)));
            Vec3::new(0.0, HANG_HEIGHT - top, (hang_z - back).max(0.0))
        }
        Some(_) if settings.placement == Placement::Drop => Vec3::Y * BODY_DROP,
        _ => Vec3::ZERO,
    };

    let mut cloth = Cloth::new();
    let mut colors = Vec::new();
    let mut back_colors = Vec::new();
    let mut first_particle = vec![0u32; pattern.pieces.len()];
    let mut pieces = Vec::new();
    for &(index, piece, mesh) in &meshes {
        let fabric = piece_fabric(pattern, piece);
        let range = if let Some(layout) = &around {
            cloth.add_piece(mesh, &fabric, |p| layout.place(piece, p) + lift, layout.front(piece))
        } else if let Some(layout) = &flat {
            cloth.add_piece(mesh, &fabric, |p| layout.place(piece, p, drop_height), layout.front(piece))
        } else if settings.placement == Placement::Hang {
            cloth.add_piece(
                mesh,
                &fabric,
                |p| {
                    Vec3::new(((p[0] - center[0]) * 1e-3) as f32, HANG_HEIGHT - ((p[1] - min[1]) * 1e-3) as f32, hang_z)
                },
                Vec3::Z,
            )
        } else {
            cloth.add_piece(
                mesh,
                &fabric,
                |p| Vec3::new(((p[0] - center[0]) * 1e-3) as f32, drop_height, ((p[1] - center[1]) * 1e-3) as f32),
                Vec3::Y,
            )
        };
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
    if settings.sew {
        let mut topologies: HashMap<usize, Topology> = HashMap::new();
        for seam_mesh in &meshed.seams {
            let Some((index, seam)) = pattern.seams.iter().enumerate().find(|(_, s)| s.id == seam_mesh.seam) else {
                continue;
            };
            seams += 1;
            let turn = if settings.sew_in_order { index as u32 } else { 0 };
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
            // Cada punto se une con el mismo punto de todos los demás lados.
            for (i, a) in seam_mesh.sides.iter().enumerate() {
                for b in &seam_mesh.sides[i + 1..] {
                    for (&va, &vb) in a.vertices.iter().zip(&b.vertices) {
                        seam_pairs.push(([first_particle[a.piece] + va, first_particle[b.piece] + vb], turn));
                    }
                }
            }

            let kind = seam.kind;
            let mut sides = Vec::new();
            for side in &seam_mesh.sides {
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

                // La costura es más rígida a lo largo que la tela sola (la
                // simple es la referencia y se queda como la tela).
                let firm = fabric.bend_stiffness > FIRM_FABRIC;
                let stiffer = if kind.stiffness() > 1.0 && !firm { v.len() } else { 0 };
                for k in 2..stiffer {
                    let rest = (at(k).distance(at(k - 2)) * 1e-3) as f32;
                    cloth.add_constraint(base + v[k - 2], base + v[k], rest, compliance);
                }
                // El margen pesa y abulta: varias capas de tela juntas. La
                // costura simple (dos capas) es la referencia: la tela ya
                // cuenta con ella, así que solo se suma lo que va de más.
                for k in 0..v.len() {
                    let before = if k > 0 { at(k).distance(at(k - 1)) } else { 0.0 };
                    let after = if k + 1 < v.len() { at(k).distance(at(k + 1)) } else { 0.0 };
                    let spacing = (before + after) / 2.0 * 1e-3;
                    let plies = if firm { 0.0 } else { (kind.layers() - 2.0) / 2.0 };
                    let particle = (base + v[k]) as usize;
                    cloth.add_mass(particle, fabric.density * (allowance * spacing) as f32 * plies);
                    cloth.thicken(particle, fabric.thickness * (1.0 + 0.5 * plies));
                }

                let color = egui::Rgba::from(color_of(piece.id));
                let right = [color.r(), color.g(), color.b()];
                let inward: Vec<Option<u32>> = (0..v.len()).map(|k| topology.inward(mesh, v, k)).collect();
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
                    for b in &sides[i + 1..] {
                        for k in 0..a.particles.len().min(b.particles.len()) {
                            if a.firm || b.firm {
                                continue;
                            }
                            let (Some(na), Some(nb)) = (a.inward[k], b.inward[k]) else { continue };
                            let (da, db) = (a.inward_rest[k], b.inward_rest[k]);
                            let rest = match fold {
                                SeamFold::Flat => da + db,
                                _ => (da - db).abs() + a.thickness + b.thickness,
                            };
                            cloth.add_constraint(na, nb, rest, (a.compliance + b.compliance) / 2.0 * FOLD_SOFTNESS);
                        }
                    }
                }
            }
            seam_draws.push(SeamDraw { index, kind, topstitch: seam.topstitch, pressed: seam.pressed_side(), sides });
        }
    }
    for &([a, b], turn) in &seam_pairs {
        cloth.add_seam_in_turn(a, b, turn);
    }

    let mut colliders = vec![Collider::ground(0.0)];
    colliders.extend(settings.obstacle.collider());
    Scene { cloth, colliders, colors, back_colors, seam_draws, failed, crossed, seams, pieces, seam_pairs }
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
        let mut boundary = vec![false; mesh.positions.len()];
        for b in &mesh.boundary {
            boundary[b.vertex as usize] = true;
        }
        Self { neighbors, boundary }
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

/// Colocación «alrededor»: las piezas de delante, detrás y los lados se
/// curvan sobre un cilindro, cada grupo centrado en su lado y con la
/// disposición del editor; las de arriba y abajo van perpendiculares al eje,
/// tapando el cilindro.
///
/// Sin maniquí hay un solo cilindro vertical, sobre el obstáculo. Con
/// maniquí, uno por zona del cuerpo ([`BodyZone`]), con el eje de esa zona
/// y un radio que deja algo de holgura sobre el cuerpo.
struct AroundLayout {
    /// Centro (mm) de cada grupo de piezas en el patrón.
    group_center: HashMap<(BodyZone, Arrangement), [f64; 2]>,
    zones: HashMap<BodyZone, ZoneLayout>,
    /// Con maniquí, cada pieza va en su zona; sin él, todas en una.
    by_zone: bool,
}

struct ZoneLayout {
    frame: ZoneFrame,
    /// Radio del cilindro (mm).
    radius: f64,
    /// Borde superior (mm, Y hacia abajo) de las piezas laterales: va en
    /// `frame.top`.
    top: f64,
    /// Alto (m) de las piezas laterales.
    height: f32,
}

/// Separación (m) entre las tapas y las piezas laterales.
const CAP_GAP: f32 = 0.05;

fn is_wall(a: Arrangement) -> bool {
    !matches!(a, Arrangement::Top | Arrangement::Bottom)
}

impl AroundLayout {
    fn new(meshes: &[(usize, &Piece, &Mesh2D)], floor: f32, avatar: Option<&Avatar>) -> Self {
        let by_zone = avatar.is_some();
        let zone_of = |piece: &Piece| if by_zone { piece.zone } else { BodyZone::Torso };
        let mut groups: HashMap<(BodyZone, Arrangement), Vec<&Mesh2D>> = HashMap::new();
        for (_, piece, mesh) in meshes {
            groups.entry((zone_of(piece), piece.arrangement)).or_default().push(mesh);
        }
        let group_bounds: HashMap<(BodyZone, Arrangement), ([f64; 2], [f64; 2])> =
            groups.iter().map(|(k, m)| (*k, bounds(m))).collect();
        let group_center = group_bounds
            .iter()
            .map(|(k, (min, max))| (*k, [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0]))
            .collect();

        let mut zones = HashMap::new();
        for zone in BodyZone::ALL {
            let in_zone: Vec<Arrangement> = groups.keys().filter(|(z, _)| *z == zone).map(|(_, a)| *a).collect();
            if in_zone.is_empty() {
                continue;
            }
            let walls: Vec<&Mesh2D> = groups
                .iter()
                .filter(|((z, a), _)| *z == zone && is_wall(*a))
                .flat_map(|(_, m)| m.iter().copied())
                .collect();
            let (wall_min, wall_max) = bounds(&walls);
            let height = if walls.is_empty() { 0.0 } else { ((wall_max[1] - wall_min[1]) * 1e-3) as f32 };
            // Cada grupo ocupa como mucho este ángulo del cilindro, para que
            // quede un hueco entre grupos vecinos. Sobre el cuerpo, una pieza
            // sola (una manga, por ejemplo) casi da la vuelta entera.
            let wall_groups = in_zone.iter().filter(|a| is_wall(**a)).count();
            let sides = in_zone.contains(&Arrangement::Left) || in_zone.contains(&Arrangement::Right);
            let max_angle = if by_zone && wall_groups == 1 {
                1.7
            } else if sides {
                0.42
            } else {
                0.85
            } * std::f64::consts::PI;
            let widest = group_bounds
                .iter()
                .filter(|((z, a), _)| *z == zone && is_wall(*a))
                .map(|(_, (min, max))| max[0] - min[0])
                .fold(0.0, f64::max);
            let fit = widest / max_angle;
            let (frame, radius) = match avatar {
                Some(avatar) => {
                    let frame = *avatar.zone(zone);
                    (frame, fit.max(((frame.radius + BODY_CLEARANCE) * 1e3) as f64))
                }
                None => {
                    let has_bottom = in_zone.contains(&Arrangement::Bottom);
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
            let top = if walls.is_empty() { 0.0 } else { wall_min[1] };
            zones.insert(zone, ZoneLayout { frame, radius, top, height });
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

    fn place(&self, piece: &Piece, p: [f64; 2]) -> Vec3 {
        let (zone, layout) = self.zone(piece);
        let a = piece.arrangement;
        let f = &layout.frame;
        let c = self.group_center.get(&(zone, a)).copied().unwrap_or_default();
        let (dx, dy) = (((p[0] - c[0]) * 1e-3) as f32, ((p[1] - c[1]) * 1e-3) as f32);
        match a {
            Arrangement::Top => f.top - f.axis * CAP_GAP + f.right * dx + f.front * dy,
            Arrangement::Bottom => f.top + f.axis * (layout.height + CAP_GAP) + f.right * dx - f.front * dy,
            _ => {
                // La x del dibujo avanza hacia la derecha de quien mira la pieza.
                let theta = Self::angle(a) + (p[0] - c[0]) / layout.radius;
                let along = ((p[1] - layout.top) * 1e-3) as f32;
                f.top + f.axis * along + f.direction(theta as f32) * (layout.radius * 1e-3) as f32
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
struct FlatLayout {
    /// Centro (mm) de cada grupo de piezas en el patrón.
    center: HashMap<Arrangement, [f64; 2]>,
    bounds: HashMap<Arrangement, ([f64; 2], [f64; 2])>,
    /// Altura media (mm) de las paredes en el patrón: es la de la tela
    /// tumbada (z = 0).
    middle: f64,
    /// Ancho y alto (mm) de las piezas de delante.
    front_size: [f64; 2],
}

impl FlatLayout {
    fn new(meshes: &[(usize, &Piece, &Mesh2D)]) -> Self {
        let mut groups: HashMap<Arrangement, Vec<&Mesh2D>> = HashMap::new();
        for (_, piece, mesh) in meshes {
            groups.entry(piece.arrangement).or_default().push(mesh);
        }
        let bounds: HashMap<Arrangement, ([f64; 2], [f64; 2])> =
            groups.iter().map(|(a, m)| (*a, self::bounds(m))).collect();
        let center =
            bounds.iter().map(|(a, (min, max))| (*a, [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0])).collect();
        let walls: Vec<&Mesh2D> =
            groups.iter().filter(|(a, _)| is_wall(**a)).flat_map(|(_, m)| m.iter().copied()).collect();
        let (wall_min, wall_max) = self::bounds(&walls);
        let middle = if walls.is_empty() { 0.0 } else { (wall_min[1] + wall_max[1]) / 2.0 };
        let front_size = bounds
            .get(&Arrangement::Front)
            .or_else(|| bounds.get(&Arrangement::Back))
            .map_or([0.0, 0.0], |(min, max)| [max[0] - min[0], max[1] - min[1]]);
        Self { center, bounds, middle, front_size }
    }

    /// Posición (m) de un punto del patrón; la capa de abajo está a `floor`.
    fn place(&self, piece: &Piece, p: [f64; 2], floor: f32) -> Vec3 {
        let a = piece.arrangement;
        let c = self.center[&a];
        let (min, max) = self.bounds[&a];
        let [w, h] = self.front_size;
        let (x, z) = match a {
            Arrangement::Front => (p[0] - c[0], p[1] - self.middle),
            // Vista desde detrás: la x del dibujo va hacia la izquierda.
            Arrangement::Back => (c[0] - p[0], p[1] - self.middle),
            Arrangement::Left => (-w / 2.0 - FLAT_GAP - (max[0] - p[0]), p[1] - self.middle),
            Arrangement::Right => (w / 2.0 + FLAT_GAP + (p[0] - min[0]), p[1] - self.middle),
            Arrangement::Top => (p[0] - c[0], -h / 2.0 - FLAT_GAP - (max[1] - p[1])),
            Arrangement::Bottom => (p[0] - c[0], h / 2.0 + FLAT_GAP + (p[1] - min[1])),
        };
        let y = if a == Arrangement::Back { floor } else { floor + LAYER_GAP };
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
            let speed = cloth.velocities.iter().map(|v| v.length()).fold(0.0, f32::max);
            assert!(speed < 0.1, "{name}: sigue moviéndose a {speed} m/s");
            let crossings = cloth.count_crossings();
            assert!(crossings < 5, "{name}: {crossings} aristas cruzadas");
        }
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
            // El cuero, tan firme, se pliega mal justo al lado de las
            // costuras y deja unas pocas aristas cruzadas, todas a menos de un
            // triángulo de una costura.
            let crossings = cloth.count_crossings();
            let tolerated = if pattern.name == "Bolso de cuero" { 15 } else { 5 };
            assert!(crossings < tolerated, "{name}: {crossings} aristas cruzadas");
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
}
