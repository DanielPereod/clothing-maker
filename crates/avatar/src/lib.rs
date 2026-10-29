//! Maniquí opcional para vestir las prendas.
//!
//! El maniquí es una malla cerrada en glTF (`assets/maniqui.glb`, sacada de
//! la malla base de MakeHuman, CC0) con sus articulaciones guardadas como
//! nodos `joint-…`. De ella sale:
//! - un [`Collider`] para la tela, con el campo de distancia precalculado;
//! - las **zonas de colocación** ([`ZoneFrame`]): un eje por zona del cuerpo
//!   (torso, brazos y piernas) con su grosor, para colocar las piezas
//!   alrededor antes de coser.
//!
//! Unidades: metros, con los pies en y = 0, mirando hacia +Z y con su
//! izquierda hacia +X.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use glam::Vec3;
use pattern::BodyZone;
use simulator::{Collider, MeshSdf};

/// Separación (m) entre los nodos del campo de distancia.
pub const SDF_CELL: f32 = 0.008;
const SDF_MARGIN: f32 = 0.05;

#[derive(Debug, thiserror::Error)]
pub enum AvatarError {
    #[error("el glTF no se puede leer: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("el glTF no tiene ninguna malla de triángulos")]
    NoMesh,
    #[error("al maniquí le falta la articulación «{0}»")]
    MissingJoint(&'static str),
}

/// Eje de una zona del cuerpo alrededor del que se curvan las piezas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneFrame {
    /// Dónde empieza el eje (el borde de arriba de las piezas).
    pub top: Vec3,
    /// Dirección del eje, hacia abajo por la zona (de hombro a mano, por
    /// ejemplo).
    pub axis: Vec3,
    /// Hacia delante y hacia la derecha de quien mira la pieza desde fuera,
    /// perpendiculares al eje. En los brazos «delante» es la cara de arriba
    /// (la de fuera), para que una manga de una pieza cierre por debajo.
    pub front: Vec3,
    pub right: Vec3,
    /// Largo de la zona (m).
    pub length: f32,
    /// Distancia (m) del eje a la superficie, por arriba (casi todo el
    /// cuerpo de la zona queda dentro).
    pub radius: f32,
}

impl ZoneFrame {
    /// Dirección perpendicular al eje a un ángulo `angle` (0 = delante, π/2
    /// = derecha de quien mira).
    pub fn direction(&self, angle: f32) -> Vec3 {
        self.front * angle.cos() + self.right * angle.sin()
    }
}

/// Un maniquí listo para la simulación.
pub struct Avatar {
    pub positions: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    pub sdf: Arc<MeshSdf>,
    zones: HashMap<BodyZone, ZoneFrame>,
}

impl std::fmt::Debug for Avatar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avatar")
            .field("vertices", &self.positions.len())
            .field("triangles", &self.triangles.len())
            .field("zones", &self.zones)
            .finish()
    }
}

static MANNEQUIN: OnceLock<Arc<Avatar>> = OnceLock::new();

/// El maniquí que viene con la aplicación. La primera llamada calcula el
/// campo de distancia (menos de un segundo); las demás lo reutilizan.
pub fn mannequin() -> Arc<Avatar> {
    MANNEQUIN
        .get_or_init(|| {
            let glb = include_bytes!("../../../assets/maniqui.glb");
            Arc::new(Avatar::from_glb(glb).expect("el maniquí incluido es válido"))
        })
        .clone()
}

impl Avatar {
    /// Carga un maniquí de un glTF binario (`.glb`): la primera malla de
    /// triángulos, ya en metros, y las articulaciones como nodos con
    /// nombre `joint-…` (las de MakeHuman).
    pub fn from_glb(bytes: &[u8]) -> Result<Self, AvatarError> {
        let gltf = gltf::Gltf::from_slice(bytes)?;
        let blob = gltf.blob.as_deref();
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        for primitive in gltf.meshes().flat_map(|m| m.primitives().collect::<Vec<_>>()) {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| match buffer.source() {
                gltf::buffer::Source::Bin => blob,
                gltf::buffer::Source::Uri(_) => None,
            });
            let Some(points) = reader.read_positions() else { continue };
            let base = positions.len() as u32;
            positions.extend(points.map(Vec3::from));
            let count = positions.len() as u32 - base;
            let indices: Vec<u32> = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect(),
                None => (0..count).collect(),
            };
            triangles.extend(indices.chunks_exact(3).map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        }
        if triangles.is_empty() {
            return Err(AvatarError::NoMesh);
        }
        // Que los triángulos miren hacia fuera (volumen positivo), para
        // dibujarlo bien.
        let volume: f32 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| positions[i as usize]);
                a.dot(b.cross(c))
            })
            .sum();
        if volume < 0.0 {
            triangles.iter_mut().for_each(|t| t.swap(1, 2));
        }
        let joints: HashMap<String, Vec3> = gltf
            .nodes()
            .filter_map(|n| {
                let name = n.name()?;
                name.starts_with("joint-").then(|| (name.to_owned(), Vec3::from(n.transform().decomposed().0)))
            })
            .collect();
        let zones = zones(&positions, &joints)?;
        let sdf = Arc::new(MeshSdf::from_mesh(&positions, &triangles, SDF_CELL, SDF_MARGIN));
        Ok(Self { positions, triangles, sdf, zones })
    }

    pub fn collider(&self) -> Collider {
        Collider::Mesh(self.sdf.clone())
    }

    pub fn zone(&self, zone: BodyZone) -> &ZoneFrame {
        &self.zones[&zone]
    }

    /// Altura (m) de la parte más alta.
    pub fn height(&self) -> f32 {
        self.positions.iter().map(|p| p.y).fold(0.0, f32::max)
    }

    /// Punto más adelantado (m, en z).
    pub fn front_z(&self) -> f32 {
        self.positions.iter().map(|p| p.z).fold(f32::NEG_INFINITY, f32::max)
    }
}

/// Cuánto sube el borde de arriba de las piezas sobre la articulación de
/// arriba de cada zona: en el torso, del cuello a lo alto de los hombros;
/// en las piernas, de la cadera a la cintura.
const TORSO_ABOVE_NECK: f32 = 0.03;
const LEGS_ABOVE_HIP: f32 = 0.12;

fn zones(positions: &[Vec3], joints: &HashMap<String, Vec3>) -> Result<HashMap<BodyZone, ZoneFrame>, AvatarError> {
    let joint = |name: &'static str| joints.get(name).copied().ok_or(AvatarError::MissingJoint(name));
    let neck = joint("joint-neck")?;
    let pelvis = joint("joint-pelvis")?;
    // El eje del torso pasa por el centro del pecho, no por la columna.
    let chest: Vec<&Vec3> =
        positions.iter().filter(|p| p.x.abs() < 0.1 && p.y > pelvis.y + 0.1 && p.y < neck.y - 0.1).collect();
    let (front, back) = chest.iter().fold((f32::NEG_INFINITY, f32::INFINITY), |(f, b), p| (f.max(p.z), b.min(p.z)));
    let torso_z = if chest.is_empty() { neck.z } else { (front + back) / 2.0 };
    let segments = [
        (BodyZone::Torso, Vec3::new(0.0, neck.y + TORSO_ABOVE_NECK, torso_z), Vec3::new(0.0, pelvis.y, torso_z)),
        // Los brazos del maniquí se doblan un poco por el codo: el eje sigue
        // el brazo (de hombro a codo), que es donde van las mangas.
        (BodyZone::RightArm, joint("joint-r-shoulder")?, joint("joint-r-elbow")?),
        (BodyZone::LeftArm, joint("joint-l-shoulder")?, joint("joint-l-elbow")?),
        (BodyZone::RightLeg, joint("joint-r-upper-leg")?, joint("joint-r-ankle")?),
        (BodyZone::LeftLeg, joint("joint-l-upper-leg")?, joint("joint-l-ankle")?),
    ];

    // Cada vértice cuenta para el grosor de la zona cuyo eje tiene más cerca.
    let mut radial: HashMap<BodyZone, Vec<f32>> = HashMap::new();
    for &p in positions {
        let nearest = segments
            .iter()
            .map(|&(zone, a, b)| {
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                (zone, t, p.distance(a + ab * t))
            })
            .min_by(|x, y| x.2.total_cmp(&y.2));
        if let Some((zone, t, d)) = nearest
            && (0.15..=0.85).contains(&t)
        {
            radial.entry(zone).or_default().push(d);
        }
    }

    let mut frames = HashMap::new();
    for (zone, a, b) in segments {
        let axis = (b - a).normalize();
        let forward = match zone {
            BodyZone::RightArm | BodyZone::LeftArm => Vec3::Y,
            _ => Vec3::Z,
        };
        let front = (forward - axis * axis.dot(forward)).normalize();
        let right = front.cross(axis);
        let lift = match zone {
            BodyZone::RightLeg | BodyZone::LeftLeg => LEGS_ABOVE_HIP,
            _ => 0.0,
        };
        // Las piernas empiezan en la cintura: se sube el principio del eje.
        let top = a - axis * (lift / axis.y.abs().max(0.5));
        let mut r = radial.remove(&zone).unwrap_or_default();
        r.sort_by(f32::total_cmp);
        // En brazos y piernas se cuelan vértices del pecho o de la cadera,
        // así que se toma un percentil más bajo.
        let percentile = if zone == BodyZone::Torso { 95 } else { 75 };
        let radius = r.get(r.len() * percentile / 100).copied().unwrap_or(0.1);
        frames.insert(zone, ZoneFrame { top, axis, front, right, length: top.distance(b), radius });
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mannequin_loads_with_sensible_zones() {
        let start = std::time::Instant::now();
        let avatar = mannequin();
        eprintln!("maniquí en {:?}: {avatar:?}", start.elapsed());
        assert!((1.5..1.9).contains(&avatar.height()), "altura {}", avatar.height());

        let torso = avatar.zone(BodyZone::Torso);
        assert!(torso.axis.y < -0.99 && torso.front.z > 0.99 && torso.right.x > 0.99, "{torso:?}");
        assert!((0.1..0.25).contains(&torso.radius), "torso {}", torso.radius);
        for zone in [BodyZone::RightArm, BodyZone::LeftArm, BodyZone::RightLeg, BodyZone::LeftLeg] {
            let f = avatar.zone(zone);
            assert!((0.03..0.1).contains(&f.radius), "{zone:?}: {}", f.radius);
            assert!(f.axis.y < 0.0 && f.front.dot(f.axis).abs() < 1e-5, "{zone:?}: {f:?}");
        }
        // La derecha del maniquí queda a la izquierda de quien lo mira (−X).
        assert!(avatar.zone(BodyZone::RightArm).top.x < -0.1);
        // Los brazos miran hacia arriba y hacia fuera.
        let arm = avatar.zone(BodyZone::LeftArm);
        assert!(arm.front.y > 0.5 && arm.front.x > 0.5, "{arm:?}");
        assert!(avatar.zone(BodyZone::LeftLeg).top.x > 0.0);
    }

    #[test]
    fn collider_matches_the_body() {
        let avatar = mannequin();
        let collider = avatar.collider();
        let torso = *avatar.zone(BodyZone::Torso);
        // Dentro del pecho, fuera delante del pecho y lejos por encima.
        let chest = torso.top + torso.axis * 0.25;
        let (d, _) = collider.signed_distance(chest);
        assert!(d < -0.05, "centro del pecho: {d}");
        let (d, n) = collider.signed_distance(chest + Vec3::Z * 0.4);
        assert!(d > 0.2 && n.z > 0.8, "delante del pecho: {d} {n}");
        let (d, _) = collider.signed_distance(Vec3::new(0.0, 2.5, 0.0));
        assert!(d > 0.7, "por encima de la cabeza: {d}");
        // Todos los vértices de la malla están en la superficie.
        let worst = avatar.positions.iter().map(|&p| collider.signed_distance(p).0.abs()).fold(0.0, f32::max);
        assert!(worst < SDF_CELL, "vértice a {worst} de la superficie");
    }
}
