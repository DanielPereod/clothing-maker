//! Simulación de tela con XPBD (Extended Position Based Dynamics).
//!
//! De momento es el esqueleto: partículas, restricciones de distancia sobre
//! las aristas de la malla y gravedad, con subpasos. Las costuras, la
//! flexión, las colisiones y la paralelización con `rayon` llegarán en las
//! siguientes fases.
//!
//! Unidades: metros, segundos y kilogramos. Las mallas vienen en mm y se
//! convierten al crear la tela.

use glam::Vec3;
use mesher::Mesh2D;

/// Restricción de distancia entre dos partículas.
#[derive(Clone, Copy, Debug)]
pub struct DistanceConstraint {
    pub a: u32,
    pub b: u32,
    pub rest_length: f32,
    /// Inversa de la rigidez (m/N). 0 = inextensible.
    pub compliance: f32,
}

/// Una tela formada por partículas.
#[derive(Clone, Debug, Default)]
pub struct Cloth {
    pub positions: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    /// Inversa de la masa; 0 = partícula fija.
    pub inv_mass: Vec<f32>,
    pub constraints: Vec<DistanceConstraint>,
    prev_positions: Vec<Vec3>,
}

/// Parámetros de la simulación.
#[derive(Clone, Copy, Debug)]
pub struct SimParams {
    pub gravity: Vec3,
    pub substeps: u32,
    /// Amortiguación de la velocidad por segundo (0..1).
    pub damping: f32,
}

impl Default for SimParams {
    fn default() -> Self {
        Self { gravity: Vec3::new(0.0, -9.81, 0.0), substeps: 10, damping: 0.1 }
    }
}

impl Cloth {
    /// Crea una tela a partir de una malla 2D (mm), colocada en el plano
    /// XY (el eje Y de la pieza apunta hacia abajo, así que se invierte).
    ///
    /// `density` en kg/m², `compliance` en m/N.
    pub fn from_mesh(mesh: &Mesh2D, density: f32, compliance: f32) -> Self {
        let positions: Vec<Vec3> =
            mesh.positions.iter().map(|p| Vec3::new(p[0] as f32 * 1e-3, -p[1] as f32 * 1e-3, 0.0)).collect();

        // Reparte la masa de cada triángulo entre sus vértices.
        let mut mass = vec![0.0f32; positions.len()];
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|i| positions[i as usize]);
            let m = 0.5 * (b - a).cross(c - a).length() * density;
            for i in t {
                mass[*i as usize] += m / 3.0;
            }
        }
        let inv_mass = mass.iter().map(|&m| if m > 0.0 { 1.0 / m } else { 0.0 }).collect();

        let constraints = mesh
            .edges()
            .into_iter()
            .map(|[a, b]| DistanceConstraint {
                a,
                b,
                rest_length: positions[a as usize].distance(positions[b as usize]),
                compliance,
            })
            .collect();

        Self {
            velocities: vec![Vec3::ZERO; positions.len()],
            prev_positions: positions.clone(),
            positions,
            inv_mass,
            constraints,
        }
    }

    /// Fija una partícula en su sitio.
    pub fn pin(&mut self, i: usize) {
        self.inv_mass[i] = 0.0;
    }

    /// Avanza la simulación `dt` segundos.
    pub fn step(&mut self, dt: f32, params: &SimParams) {
        let substeps = params.substeps.max(1);
        let h = dt / substeps as f32;
        let damping = (1.0 - params.damping).powf(h);
        for _ in 0..substeps {
            for i in 0..self.positions.len() {
                self.prev_positions[i] = self.positions[i];
                if self.inv_mass[i] > 0.0 {
                    self.velocities[i] += params.gravity * h;
                    self.positions[i] += self.velocities[i] * h;
                }
            }
            self.solve_distances(h);
            for i in 0..self.positions.len() {
                self.velocities[i] = (self.positions[i] - self.prev_positions[i]) / h * damping;
            }
        }
    }

    fn solve_distances(&mut self, h: f32) {
        for c in &self.constraints {
            let (a, b) = (c.a as usize, c.b as usize);
            let w = self.inv_mass[a] + self.inv_mass[b];
            if w == 0.0 {
                continue;
            }
            let d = self.positions[a] - self.positions[b];
            let len = d.length();
            if len < 1e-9 {
                continue;
            }
            let alpha = c.compliance / (h * h);
            let lambda = -(len - c.rest_length) / (w + alpha);
            let corr = d / len * lambda;
            self.positions[a] += corr * self.inv_mass[a];
            self.positions[b] -= corr * self.inv_mass[b];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pattern::{Pattern, Point};

    #[test]
    fn hanging_cloth_stays_stable() {
        let mut p = Pattern::new("t");
        let id = p.add_rect("r", Point::ZERO, 200.0, 200.0);
        let mesh = mesher::mesh_piece(p.piece(id).unwrap(), &Default::default()).unwrap();
        let mut cloth = Cloth::from_mesh(&mesh, 0.2, 0.0);
        // Cuelga de las dos esquinas superiores.
        for (i, pos) in mesh.positions.iter().enumerate() {
            if pos[1] == 0.0 && (pos[0] == 0.0 || pos[0] == 200.0) {
                cloth.pin(i);
            }
        }
        for _ in 0..120 {
            cloth.step(1.0 / 60.0, &SimParams::default());
        }
        assert!(cloth.positions.iter().all(|p| p.is_finite()));
        // La tela cae pero no se estira de forma absurda (< 10 %).
        let lowest = cloth.positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        assert!(lowest < -0.15 && lowest > -0.31, "punto más bajo: {lowest}");
    }
}
