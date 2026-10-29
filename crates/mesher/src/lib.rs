//! Convierte piezas de patrón en mallas de triángulos 2D.
//!
//! El contorno se remuestrea por longitud de arco para que todas las aristas
//! del borde tengan un tamaño parecido (importante para coser luego dos
//! piezas vértice a vértice), y el interior se rellena con una triangulación
//! de Delaunay restringida y refinada (`spade`).
//!
//! [`mesh_pattern`] malla todas las piezas de un patrón a la vez y reparte
//! los puntos de modo que todos los lados de una costura tengan los mismos,
//! para poder unirlos punto a punto aunque midan algo distinto.

use std::collections::HashMap;

use kurbo::{ParamCurve, ParamCurveArclen, PathSeg, Point};
use pattern::{EdgeId, Pattern, Piece, PieceId, SeamId, polygon_self_intersects};
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

    /// Vértices del borde a lo largo de una arista del patrón, del principio
    /// al final, con los dos extremos.
    pub fn edge_vertices(&self, edge: EdgeId) -> Option<Vec<u32>> {
        let first = self.boundary.iter().position(|b| b.edge == edge)?;
        let mut out: Vec<u32> =
            self.boundary[first..].iter().take_while(|b| b.edge == edge).map(|b| b.vertex).collect();
        let end = (first + out.len()) % self.boundary.len();
        out.push(self.boundary[end].vertex);
        Some(out)
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

#[derive(Clone, Debug, thiserror::Error, PartialEq)]
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
    mesh_piece_with(piece, options, |_| None)
}

/// Lo que mide de verdad cada lado de una costura en la malla.
#[derive(Clone, Debug, PartialEq)]
pub struct SeamSideMesh {
    /// Índice de la pieza en [`PatternMesh::pieces`] (el mismo que en el patrón).
    pub piece: usize,
    /// Índice del lado en [`pattern::Seam::sides`].
    pub side: usize,
    /// Vértices del borde de la pieza a lo largo de la arista, en el sentido
    /// de la costura y con los dos extremos incluidos.
    pub vertices: Vec<u32>,
}

/// Una costura ya mallada: todos sus lados tienen el mismo número de
/// vértices, y el vértice `k` de cada lado se une con el `k` de los demás.
#[derive(Clone, Debug, PartialEq)]
pub struct SeamMesh {
    pub seam: SeamId,
    pub sides: Vec<SeamSideMesh>,
}

/// Mallas de todas las piezas de un patrón y sus costuras.
#[derive(Clone, Debug)]
pub struct PatternMesh {
    /// Una entrada por pieza, en el orden del patrón.
    pub pieces: Vec<Result<Mesh2D, MeshError>>,
    /// Solo las costuras con al menos dos lados en piezas bien malladas.
    pub seams: Vec<SeamMesh>,
}

/// Malla todas las piezas de un patrón. Los lados de cada costura (y de las
/// costuras que comparten aristas con ella) reciben el mismo número de
/// divisiones: el mayor de los que les tocarían por separado.
pub fn mesh_pattern(pattern: &Pattern, options: &MeshOptions) -> PatternMesh {
    let target = options.target_edge_length.max(0.1);
    // Divisiones que pediría cada arista cosida por su cuenta.
    let mut divisions: HashMap<(PieceId, EdgeId), usize> = HashMap::new();
    for seam in &pattern.seams {
        for side in &seam.sides {
            let Some(piece) = pattern.piece(side.piece) else { continue };
            let Some(i) = piece.edge_index(side.edge) else { continue };
            divisions.insert((side.piece, side.edge), edge_divisions(&piece.segment(i), target, options));
        }
    }
    // Aristas unidas por costuras, directa o indirectamente, comparten el
    // número de divisiones. Se propaga el máximo hasta que no cambia nada.
    loop {
        let mut changed = false;
        for seam in &pattern.seams {
            let keys = seam.sides.iter().map(|s| (s.piece, s.edge));
            let max = keys.clone().filter_map(|k| divisions.get(&k).copied()).max().unwrap_or(1);
            for k in keys {
                if let Some(n) = divisions.get_mut(&k)
                    && *n < max
                {
                    *n = max;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    let pieces: Vec<Result<Mesh2D, MeshError>> = pattern
        .pieces
        .iter()
        .map(|piece| mesh_piece_with(piece, options, |edge| divisions.get(&(piece.id, edge)).copied()))
        .collect();

    let index: HashMap<PieceId, usize> = pattern.pieces.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let mut seams = Vec::new();
    for seam in &pattern.seams {
        let mut sides: Vec<SeamSideMesh> = Vec::new();
        for (k, side) in seam.sides.iter().enumerate() {
            let Some(&piece) = index.get(&side.piece) else { continue };
            let Ok(mesh) = &pieces[piece] else { continue };
            let Some(mut vertices) = mesh.edge_vertices(side.edge) else { continue };
            if side.reversed {
                vertices.reverse();
            }
            sides.push(SeamSideMesh { piece, side: k, vertices });
        }
        // Por si el remuestreo ha fundido algún punto (aristas casi nulas):
        // solo se cosen los lados con el número de puntos esperado.
        let expected = seam.sides.first().and_then(|s| divisions.get(&(s.piece, s.edge))).map_or(0, |n| n + 1);
        sides.retain(|s| s.vertices.len() == expected);
        if sides.len() >= 2 {
            seams.push(SeamMesh { seam: seam.id, sides });
        }
    }
    PatternMesh { pieces, seams }
}

fn edge_divisions(seg: &PathSeg, target: f64, options: &MeshOptions) -> usize {
    let len = seg.arclen(1e-3);
    ((len / target).ceil() as usize).max(curve_min_divisions(seg, options.curve_tolerance)).max(1)
}

/// Malla una pieza. `divisions` puede fijar en cuántos tramos se divide una
/// arista (si devuelve `None`, se decide por la longitud objetivo).
pub fn mesh_piece_with(
    piece: &Piece,
    options: &MeshOptions,
    divisions: impl Fn(EdgeId) -> Option<usize>,
) -> Result<Mesh2D, MeshError> {
    if piece.len() < 3 {
        return Err(MeshError::TooFewVertices);
    }
    let target = options.target_edge_length.max(0.1);

    // 1. Remuestreo del contorno.
    let mut outline: Vec<(Point, EdgeId, f64)> = Vec::new();
    for (i, seg) in piece.segments().enumerate() {
        let edge = piece.edges()[i].id;
        let len = seg.arclen(1e-3);
        let n = divisions(edge).unwrap_or_else(|| edge_divisions(&seg, target, options)).max(1);
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
    fn seam_sides_get_the_same_points() {
        use pattern::SeamSide;
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 200.0, 100.0);
        let b = p.add_rect("b", Point::new(300.0, 0.0), 230.0, 100.0);
        let c = p.add_rect("c", Point::new(600.0, 0.0), 100.0, 100.0);
        // La arista de abajo de «b» es curva y más larga.
        p.piece_mut(b).unwrap().make_curve(2);
        p.piece_mut(b).unwrap().set_control_point(2, 0, Point::new(480.0, 160.0));
        let e = |p: &Pattern, id, i: usize| p.piece(id).unwrap().edges()[i].id;
        let (ea, eb, ec) = (e(&p, a, 0), e(&p, b, 2), e(&p, c, 1));
        p.add_seam("abc", vec![SeamSide::new(a, ea, false), SeamSide::new(b, eb, true), SeamSide::new(c, ec, false)]);
        let mesh = mesh_pattern(&p, &MeshOptions::default());
        assert_eq!(mesh.seams.len(), 1);
        let seam = &mesh.seams[0];
        assert_eq!(seam.sides.len(), 3);
        let n = seam.sides[0].vertices.len();
        assert!(n > 11, "la curva larga debería pedir más puntos: {n}");
        assert!(seam.sides.iter().all(|s| s.vertices.len() == n));

        // Extremos: el lado de «a» va de (0,0) a (200,0); el de «b», al revés.
        let pos = |side: &SeamSideMesh, k: usize| {
            mesh.pieces[side.piece].as_ref().unwrap().positions[side.vertices[k] as usize]
        };
        assert_eq!(pos(&seam.sides[0], 0), [0.0, 0.0]);
        assert_eq!(pos(&seam.sides[0], n - 1), [200.0, 0.0]);
        assert_eq!(pos(&seam.sides[1], 0), [300.0, 100.0]);
        assert_eq!(pos(&seam.sides[1], n - 1), [530.0, 100.0]);
        // Los puntos de cada lado son vértices distintos del borde.
        for side in &seam.sides {
            let mut v = side.vertices.clone();
            v.dedup();
            assert_eq!(v.len(), n);
        }
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
