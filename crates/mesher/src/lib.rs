//! Convierte piezas de patrón en mallas de triángulos 2D.
//!
//! El contorno se remuestrea por longitud de arco para que todas las aristas
//! del borde tengan un tamaño parecido (importante para coser luego dos
//! piezas vértice a vértice), y el interior se rellena con una triangulación
//! de Delaunay restringida y refinada (`spade`).

use kurbo::{ParamCurve, ParamCurveArclen, PathSeg, Point};
use pattern::{EdgeId, Piece, polygon_self_intersects};
use spade::{ConstrainedDelaunayTriangulation, Point2, RefinementParameters, Triangulation};

/// Parámetros del mallado.
#[derive(Clone, Copy, Debug)]
pub struct MeshOptions {
    /// Longitud objetivo (mm) de las aristas de la malla.
    pub target_edge_length: f64,
    /// Error máximo (mm) al aproximar curvas con segmentos.
    pub curve_tolerance: f64,
}

impl Default for MeshOptions {
    fn default() -> Self {
        Self { target_edge_length: 20.0, curve_tolerance: 0.5 }
    }
}

/// De qué arista del patrón sale un vértice del borde de la malla.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryVertex {
    /// Índice del vértice en [`Mesh2D::positions`].
    pub vertex: u32,
    pub edge: EdgeId,
    /// Parámetro de longitud de arco normalizado (0..1) dentro de la arista.
    pub s: f64,
}

/// Malla de triángulos en el plano de la pieza (mm).
#[derive(Clone, Debug, Default)]
pub struct Mesh2D {
    pub positions: Vec<[f64; 2]>,
    /// Triángulos en sentido antihorario (en coordenadas Y-arriba).
    pub triangles: Vec<[u32; 3]>,
    /// Vértices del borde, en orden a lo largo del contorno.
    pub boundary: Vec<BoundaryVertex>,
}

impl Mesh2D {
    /// Aristas únicas de la malla (útil para restricciones de distancia).
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut edges: Vec<[u32; 2]> = self
            .triangles
            .iter()
            .flat_map(|t| [[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]])
            .map(|[a, b]| if a < b { [a, b] } else { [b, a] })
            .collect();
        edges.sort_unstable();
        edges.dedup();
        edges
    }

    pub fn area(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.positions[i as usize]);
                0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs()
            })
            .sum()
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MeshError {
    #[error("la pieza necesita al menos 3 vértices")]
    TooFewVertices,
    #[error("el contorno de la pieza se corta a sí mismo")]
    SelfIntersecting,
    #[error("la triangulación ha fallado: {0}")]
    Triangulation(String),
}

/// Malla una pieza.
pub fn mesh_piece(piece: &Piece, options: &MeshOptions) -> Result<Mesh2D, MeshError> {
    if piece.len() < 3 {
        return Err(MeshError::TooFewVertices);
    }
    let target = options.target_edge_length.max(0.1);

    // 1. Remuestreo del contorno.
    let mut outline: Vec<(Point, EdgeId, f64)> = Vec::new();
    for (i, seg) in piece.segments().enumerate() {
        let edge = piece.edges()[i].id;
        let len = seg.arclen(1e-3);
        let n = ((len / target).ceil() as usize).max(curve_min_divisions(&seg, options.curve_tolerance)).max(1);
        for k in 0..n {
            let s = k as f64 / n as f64;
            let t = if k == 0 { 0.0 } else { seg.inv_arclen(s * len, 1e-4) };
            outline.push((seg.eval(t), edge, s));
        }
    }
    outline.dedup_by(|a, b| a.0.distance(b.0) < 1e-6);
    if outline.len() > 1 && outline[0].0.distance(outline[outline.len() - 1].0) < 1e-6 {
        outline.pop();
    }
    let polygon: Vec<Point> = outline.iter().map(|o| o.0).collect();
    if piece.is_self_intersecting(options.curve_tolerance) || polygon_self_intersects(&polygon) {
        return Err(MeshError::SelfIntersecting);
    }

    // 2. Triangulación restringida + refinado.
    let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
    let mut handles = Vec::with_capacity(polygon.len());
    for p in &polygon {
        let h = cdt.insert(Point2::new(p.x, p.y)).map_err(|e| MeshError::Triangulation(format!("{e:?}")))?;
        if handles.contains(&h) {
            // Dos puntos no contiguos del contorno coinciden: se toca a sí mismo.
            return Err(MeshError::SelfIntersecting);
        }
        handles.push(h);
    }
    for i in 0..handles.len() {
        let (a, b) = (handles[i], handles[(i + 1) % handles.len()]);
        if a != b && !cdt.can_add_constraint(a, b) {
            return Err(MeshError::SelfIntersecting);
        }
        cdt.add_constraint(a, b);
    }
    let max_area = 3f64.sqrt() / 4.0 * target * target;
    let result = cdt.refine(
        RefinementParameters::<f64>::new()
            .exclude_outer_faces(true)
            .keep_constraint_edges()
            .with_max_allowed_area(max_area)
            .with_max_additional_vertices(200_000),
    );
    let excluded: std::collections::HashSet<_> = result.excluded_faces.into_iter().collect();

    // 3. Exportación. Los índices de spade son estables y empiezan por los
    // vértices del contorno, en el orden en que se insertaron.
    let positions = cdt.vertices().map(|v| [v.position().x, v.position().y]).collect();
    let triangles = cdt
        .inner_faces()
        .filter(|f| !excluded.contains(&f.fix()))
        .map(|f| f.vertices().map(|v| v.fix().index() as u32))
        .collect();
    let boundary = outline
        .iter()
        .zip(&handles)
        .map(|(&(_, edge, s), h)| BoundaryVertex { vertex: h.index() as u32, edge, s })
        .collect();
    Ok(Mesh2D { positions, triangles, boundary })
}

/// Número mínimo de divisiones para que una curva no se desvíe más de
/// `tolerance` de su aproximación poligonal.
fn curve_min_divisions(seg: &PathSeg, tolerance: f64) -> usize {
    match seg {
        PathSeg::Line(_) => 1,
        _ => {
            let mut count = 0usize;
            let mut path = kurbo::BezPath::new();
            path.move_to(seg.start());
            match seg {
                PathSeg::Cubic(c) => path.curve_to(c.p1, c.p2, c.p3),
                PathSeg::Quad(q) => path.quad_to(q.p1, q.p2),
                PathSeg::Line(l) => path.line_to(l.p1),
            }
            kurbo::flatten(&path, tolerance.max(1e-3), |_| count += 1);
            count.saturating_sub(1).max(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pattern::Pattern;

    #[test]
    fn meshes_a_rectangle() {
        let mut p = Pattern::new("t");
        let id = p.add_rect("r", Point::ZERO, 200.0, 100.0);
        let mesh = mesh_piece(p.piece(id).unwrap(), &MeshOptions::default()).unwrap();
        assert!((mesh.area() - 20_000.0).abs() < 1e-6);
        // 600 mm de perímetro / 20 mm = 30 vértices de borde.
        assert_eq!(mesh.boundary.len(), 30);
        assert!(mesh.triangles.len() > 50);
        for e in mesh.edges() {
            let [a, b] = e.map(|i| mesh.positions[i as usize]);
            let len = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
            assert!(len < 40.0, "arista demasiado larga: {len}");
        }
    }

    #[test]
    fn meshes_a_concave_curved_piece() {
        let mut p = Pattern::new("t");
        let id = p.add_polygon(
            "L",
            vec![
                Point::new(0.0, 0.0),
                Point::new(200.0, 0.0),
                Point::new(200.0, 50.0),
                Point::new(50.0, 50.0),
                Point::new(50.0, 200.0),
                Point::new(0.0, 200.0),
            ],
        );
        let piece = p.piece_mut(id).unwrap();
        piece.make_curve(1);
        piece.set_control_point(1, 0, Point::new(260.0, 10.0));
        let piece = p.piece(id).unwrap();
        let mesh = mesh_piece(piece, &MeshOptions::default()).unwrap();
        let rel = (mesh.area() - piece.area().abs()).abs() / piece.area().abs();
        assert!(rel < 0.01, "área de la malla desviada un {:.2}%", rel * 100.0);
    }

    #[test]
    fn rejects_self_intersecting() {
        let mut p = Pattern::new("t");
        let id = p.add_polygon(
            "x",
            vec![Point::new(0.0, 0.0), Point::new(100.0, 100.0), Point::new(100.0, 0.0), Point::new(0.0, 100.0)],
        );
        assert_eq!(mesh_piece(p.piece(id).unwrap(), &MeshOptions::default()).unwrap_err(), MeshError::SelfIntersecting);
    }
}
