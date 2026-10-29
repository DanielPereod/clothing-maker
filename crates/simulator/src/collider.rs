//! Objetos de colisión genéricos: el suelo, primitivas y, más adelante, el
//! avatar (como una malla convertida a campo de distancia). La tela no sabe
//! si hay un cuerpo o no; solo ve una lista de colisionadores.

use glam::Vec3;

/// Un objeto rígido y quieto contra el que choca la tela.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    /// Semiespacio: todo lo que queda por debajo del plano es sólido.
    Plane {
        point: Vec3,
        normal: Vec3,
    },
    Sphere {
        center: Vec3,
        radius: f32,
    },
    /// Caja alineada con los ejes.
    Cuboid {
        center: Vec3,
        half_extents: Vec3,
    },
}

impl Collider {
    /// Suelo horizontal a la altura `y`.
    pub fn ground(y: f32) -> Self {
        Collider::Plane { point: Vec3::new(0.0, y, 0.0), normal: Vec3::Y }
    }

    /// Distancia con signo desde `p` a la superficie (negativa dentro) y la
    /// dirección hacia fuera.
    pub fn signed_distance(&self, p: Vec3) -> (f32, Vec3) {
        match *self {
            Collider::Plane { point, normal } => {
                let n = normal.normalize_or(Vec3::Y);
                ((p - point).dot(n), n)
            }
            Collider::Sphere { center, radius } => {
                let d = p - center;
                let len = d.length();
                let n = if len > 1e-9 { d / len } else { Vec3::Y };
                (len - radius, n)
            }
            Collider::Cuboid { center, half_extents } => {
                let local = p - center;
                let q = local.abs() - half_extents;
                if q.max_element() > 0.0 {
                    // Fuera: distancia al punto más cercano de la caja.
                    let outside = q.max(Vec3::ZERO);
                    let len = outside.length();
                    let n = (outside * local.signum()) / len.max(1e-9);
                    (len, n)
                } else {
                    // Dentro: sale por la cara más cercana.
                    let axis = if q.x > q.y && q.x > q.z {
                        Vec3::X
                    } else if q.y > q.z {
                        Vec3::Y
                    } else {
                        Vec3::Z
                    };
                    (q.max_element(), axis * local.signum())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        let (d, n) = Collider::ground(0.0).signed_distance(Vec3::new(3.0, -0.5, 1.0));
        assert_eq!((d, n), (-0.5, Vec3::Y));

        let s = Collider::Sphere { center: Vec3::ZERO, radius: 1.0 };
        let (d, n) = s.signed_distance(Vec3::new(0.0, 0.0, 3.0));
        assert!((d - 2.0).abs() < 1e-6 && n == Vec3::Z);

        let b = Collider::Cuboid { center: Vec3::ZERO, half_extents: Vec3::new(1.0, 2.0, 3.0) };
        let (d, n) = b.signed_distance(Vec3::new(0.0, 2.5, 0.0));
        assert!((d - 0.5).abs() < 1e-6 && n == Vec3::Y);
        let (d, n) = b.signed_distance(Vec3::new(0.9, 0.0, 0.0));
        assert!((d + 0.1).abs() < 1e-6 && n == Vec3::X);
        let (d, _) = b.signed_distance(Vec3::new(2.0, 3.0, 0.0));
        assert!((d - 2f32.sqrt()).abs() < 1e-6);
    }
}
