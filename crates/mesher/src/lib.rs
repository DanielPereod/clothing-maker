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
//!
//! Los agujeros de una pieza (otras piezas con `hole_in`) se recortan de su
//! malla y sus líneas interiores quedan como aristas de la malla, con sus
//! vértices, para poder coser encima.

use std::collections::HashMap;

use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveNearest, PathSeg, Point};
use pattern::{EdgeId, Pattern, Piece, PieceId, SeamId, polygon_self_intersects};
use spade::{ConstrainedDelaunayTriangulation, Point2, RefinementParameters, Triangulation};

/// Distancia mínima (mm) de una línea interior al borde de la tela para
/// mallarla.
const LINE_GAP: f64 = 0.5;

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
    /// Borde de cada agujero, en orden a lo largo de su contorno.
    pub holes: Vec<Vec<BoundaryVertex>>,
    /// Vértices de cada línea interior, del principio al final.
    pub lines: Vec<LineVertices>,
}

/// Los vértices de la malla que siguen una línea interior.
#[derive(Clone, Debug, PartialEq)]
pub struct LineVertices {
    pub line: EdgeId,
    pub vertices: Vec<u32>,
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

    /// Vértices a lo largo de una arista del patrón (del contorno o de un
    /// agujero) o de una línea interior, del principio al final, con los dos
    /// extremos.
    pub fn edge_vertices(&self, edge: EdgeId) -> Option<Vec<u32>> {
        let in_loop = |contour: &[BoundaryVertex]| {
            let first = contour.iter().position(|b| b.edge == edge)?;
            let mut out: Vec<u32> = contour[first..].iter().take_while(|b| b.edge == edge).map(|b| b.vertex).collect();
            let end = (first + out.len()) % contour.len();
            out.push(contour[end].vertex);
            Some(out)
        };
        in_loop(&self.boundary)
            .or_else(|| self.holes.iter().find_map(|h| in_loop(h)))
            .or_else(|| self.lines.iter().find(|l| l.line == edge).map(|l| l.vertices.clone()))
    }

    /// Si un vértice está en el borde de la tela (el contorno o un agujero).
    pub fn border_flags(&self) -> Vec<bool> {
        let mut border = vec![false; self.positions.len()];
        for b in self.boundary.iter().chain(self.holes.iter().flatten()) {
            border[b.vertex as usize] = true;
        }
        border
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
    /// No es tela: es un agujero de otra pieza, y se malla con ella.
    #[error("es un agujero de otra pieza")]
    Hole,
}

/// Malla una pieza sola, sin agujeros.
pub fn mesh_piece(piece: &Piece, options: &MeshOptions) -> Result<Mesh2D, MeshError> {
    mesh_piece_with(piece, &[], options, |_| None)
}

/// Malla una pieza de un patrón, con sus agujeros.
pub fn mesh_piece_in(pattern: &Pattern, piece: &Piece, options: &MeshOptions) -> Result<Mesh2D, MeshError> {
    if piece.is_hole() {
        return Err(MeshError::Hole);
    }
    let holes: Vec<&Piece> = pattern.holes_of(piece.id).collect();
    mesh_piece_with(piece, &holes, options, |_| None)
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
    // Las aristas de un agujero se mallan con la pieza en la que está.
    let owner = |id: PieceId| pattern.piece(id).and_then(|p| p.hole_in).unwrap_or(id);
    // Divisiones que pediría cada arista cosida por su cuenta.
    let mut divisions: HashMap<(PieceId, EdgeId), usize> = HashMap::new();
    for seam in &pattern.seams {
        for side in &seam.sides {
            let Some(seg) = pattern.piece(side.piece).and_then(|p| p.edge_segment(side.edge)) else { continue };
            divisions.insert((owner(side.piece), side.edge), edge_divisions(&seg, target, options));
        }
    }
    // Aristas unidas por costuras, directa o indirectamente, comparten el
    // número de divisiones. Se propaga el máximo hasta que no cambia nada.
    loop {
        let mut changed = false;
        for seam in &pattern.seams {
            let keys = seam.sides.iter().map(|s| (owner(s.piece), s.edge));
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
        .map(|piece| {
            if piece.is_hole() {
                return Err(MeshError::Hole);
            }
            let holes: Vec<&Piece> = pattern.holes_of(piece.id).collect();
            mesh_piece_with(piece, &holes, options, |edge| divisions.get(&(piece.id, edge)).copied())
        })
        .collect();

    let index: HashMap<PieceId, usize> = pattern.pieces.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let mut seams = Vec::new();
    for seam in &pattern.seams {
        let mut sides: Vec<SeamSideMesh> = Vec::new();
        for (k, side) in seam.sides.iter().enumerate() {
            let Some(&piece) = index.get(&owner(side.piece)) else { continue };
            let Ok(mesh) = &pieces[piece] else { continue };
            let Some(mut vertices) = mesh.edge_vertices(side.edge) else { continue };
            if side.reversed {
                vertices.reverse();
            }
            sides.push(SeamSideMesh { piece, side: k, vertices });
        }
        // Por si el remuestreo ha fundido algún punto (aristas casi nulas):
        // solo se cosen los lados con el número de puntos esperado.
        let expected = seam.sides.first().and_then(|s| divisions.get(&(owner(s.piece), s.edge))).map_or(0, |n| n + 1);
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

/// Malla una pieza con sus agujeros (`holes`, piezas con su contorno).
/// `divisions` puede fijar en cuántos tramos se divide una arista o una
/// línea interior (si devuelve `None`, se decide por la longitud objetivo).
///
/// Un agujero que no cabe en la pieza o una línea que se sale de la tela o
/// cruza otra se dejan fuera (el patrón avisa al validarlo) en vez de
/// estropear la malla entera.
pub fn mesh_piece_with(
    piece: &Piece,
    holes: &[&Piece],
    options: &MeshOptions,
    divisions: impl Fn(EdgeId) -> Option<usize>,
) -> Result<Mesh2D, MeshError> {
    if piece.len() < 3 {
        return Err(MeshError::TooFewVertices);
    }
    let target = options.target_edge_length.max(0.1);
    let count =
        |edge: EdgeId, seg: &PathSeg| divisions(edge).unwrap_or_else(|| edge_divisions(seg, target, options)).max(1);

    // 1. Remuestreo del contorno y de los agujeros.
    let outline = resample(piece, &count);
    let polygon: Vec<Point> = outline.iter().map(|o| o.0).collect();
    if piece.is_self_intersecting(options.curve_tolerance) || polygon_self_intersects(&polygon) {
        return Err(MeshError::SelfIntersecting);
    }
    let mut hole_loops: Vec<Vec<(Point, EdgeId, f64)>> = Vec::new();
    for hole in holes {
        if hole.len() < 3 {
            continue;
        }
        let contour = resample(hole, &count);
        let ring: Vec<Point> = contour.iter().map(|o| o.0).collect();
        let fits = contour.len() >= 3
            && !polygon_self_intersects(&ring)
            && ring.iter().all(|&p| point_in_polygon(p, &polygon))
            && !rings_cross(&ring, &polygon)
            && hole_loops.iter().all(|h| {
                let other: Vec<Point> = h.iter().map(|o| o.0).collect();
                !rings_cross(&ring, &other)
                    && !ring.iter().any(|&p| point_in_polygon(p, &other))
                    && !other.iter().any(|&p| point_in_polygon(p, &ring))
            });
        if fits {
            hole_loops.push(contour);
        }
    }
    let rings: Vec<Vec<Point>> = hole_loops.iter().map(|h| h.iter().map(|o| o.0).collect()).collect();
    let in_fabric = |p: Point| point_in_polygon(p, &polygon) && !rings.iter().any(|r| point_in_polygon(p, r));

    // 2. Líneas interiores: puntos a lo largo de cada una, si caben.
    let mut lines: Vec<(EdgeId, Vec<Point>)> = Vec::new();
    for line in &piece.lines {
        let seg = line.segment();
        let len = seg.arclen(1e-3);
        if len < 1e-6 {
            continue;
        }
        let n = count(line.id, &seg);
        let points: Vec<Point> = (0..=n).map(|k| seg.eval(seg.inv_arclen(k as f64 / n as f64 * len, 1e-4))).collect();
        let clear = |p: Point| {
            in_fabric(p) && std::iter::once(&polygon).chain(&rings).all(|ring| ring_distance(p, ring) > LINE_GAP)
        };
        if points.iter().all(|&p| clear(p)) && !lines.iter().any(|(_, other)| polylines_cross(&points, other)) {
            lines.push((line.id, points));
        }
    }

    // 3. Triangulación restringida + refinado.
    let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
    let insert = |cdt: &mut ConstrainedDelaunayTriangulation<Point2<f64>>, p: Point| {
        cdt.insert(Point2::new(p.x, p.y)).map_err(|e| MeshError::Triangulation(format!("{e:?}")))
    };
    let mut handles = Vec::with_capacity(polygon.len());
    for &p in &polygon {
        let h = insert(&mut cdt, p)?;
        if handles.contains(&h) {
            // Dos puntos no contiguos del contorno coinciden: se toca a sí mismo.
            return Err(MeshError::SelfIntersecting);
        }
        handles.push(h);
    }
    close_ring(&mut cdt, &handles).then_some(()).ok_or(MeshError::SelfIntersecting)?;
    let mut hole_handles = Vec::new();
    for ring in &rings {
        let hs: Vec<_> = ring.iter().map(|&p| insert(&mut cdt, p)).collect::<Result<_, _>>()?;
        if !close_ring(&mut cdt, &hs) {
            return Err(MeshError::SelfIntersecting);
        }
        hole_handles.push(hs);
    }
    let mut line_handles = Vec::new();
    for (id, points) in &lines {
        let hs: Vec<_> = points.iter().map(|&p| insert(&mut cdt, p)).collect::<Result<_, _>>()?;
        let distinct = hs.windows(2).all(|w| w[0] != w[1]);
        if distinct && hs.windows(2).all(|w| cdt.can_add_constraint(w[0], w[1])) {
            for w in hs.windows(2) {
                cdt.add_constraint(w[0], w[1]);
            }
            line_handles.push((*id, hs));
        }
    }
    // Sin líneas interiores, spade sabe qué caras quedan fuera (el exterior
    // y los agujeros, por paridad). Una línea abierta le confunde, así que
    // entonces se refina todo y se mira después qué triángulos son tela.
    let simple = line_handles.is_empty();
    let max_area = 3f64.sqrt() / 4.0 * target * target;
    let result = cdt.refine(
        RefinementParameters::<f64>::new()
            .exclude_outer_faces(simple)
            .keep_constraint_edges()
            .with_max_allowed_area(max_area)
            .with_max_additional_vertices(200_000),
    );
    let excluded: std::collections::HashSet<_> = result.excluded_faces.into_iter().collect();

    // 4. Exportación, solo con los vértices que usan los triángulos de tela.
    let triangles: Vec<[u32; 3]> = cdt
        .inner_faces()
        .filter(|f| !excluded.contains(&f.fix()))
        .filter(|f| {
            simple || {
                let [a, b, c] = f.positions();
                in_fabric(Point::new((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0))
            }
        })
        .map(|f| f.vertices().map(|v| v.fix().index() as u32))
        .collect();
    let mut remap = vec![u32::MAX; cdt.num_vertices()];
    for &i in triangles.iter().flatten() {
        remap[i as usize] = 0;
    }
    let mut positions = Vec::new();
    for (i, v) in cdt.vertices().enumerate() {
        if remap[i] == 0 {
            remap[i] = positions.len() as u32;
            positions.push([v.position().x, v.position().y]);
        }
    }
    let at = |h: spade::handles::FixedVertexHandle| remap[h.index()];
    let triangles = triangles.into_iter().map(|t| t.map(|i| remap[i as usize])).collect();
    let contour = |samples: &[(Point, EdgeId, f64)], hs: &[spade::handles::FixedVertexHandle]| {
        samples.iter().zip(hs).map(|(&(_, edge, s), &h)| BoundaryVertex { vertex: at(h), edge, s }).collect()
    };
    let boundary = contour(&outline, &handles);
    let holes = hole_loops.iter().zip(&hole_handles).map(|(samples, hs)| contour(samples, hs)).collect();
    let lines = line_handles
        .iter()
        .map(|(line, hs)| LineVertices { line: *line, vertices: hs.iter().map(|&h| at(h)).collect() })
        .collect();
    Ok(Mesh2D { positions, triangles, boundary, holes, lines })
}

/// Puntos del contorno de una pieza repartidos por longitud de arco: cada
/// arista en los tramos que diga `count`. Cada punto lleva su arista y su
/// posición en ella (0..1).
fn resample(piece: &Piece, count: &impl Fn(EdgeId, &PathSeg) -> usize) -> Vec<(Point, EdgeId, f64)> {
    let mut out: Vec<(Point, EdgeId, f64)> = Vec::new();
    for (i, seg) in piece.segments().enumerate() {
        let edge = piece.edges()[i].id;
        let len = seg.arclen(1e-3);
        let n = count(edge, &seg);
        for k in 0..n {
            let s = k as f64 / n as f64;
            let t = if k == 0 { 0.0 } else { seg.inv_arclen(s * len, 1e-4) };
            out.push((seg.eval(t), edge, s));
        }
    }
    out.dedup_by(|a, b| a.0.distance(b.0) < 1e-6);
    if out.len() > 1 && out[0].0.distance(out[out.len() - 1].0) < 1e-6 {
        out.pop();
    }
    out
}

/// Une los puntos de un contorno cerrado con aristas restringidas.
/// Devuelve `false` si alguna se cruza con otra ya puesta.
fn close_ring(
    cdt: &mut ConstrainedDelaunayTriangulation<Point2<f64>>,
    handles: &[spade::handles::FixedVertexHandle],
) -> bool {
    for i in 0..handles.len() {
        let (a, b) = (handles[i], handles[(i + 1) % handles.len()]);
        if a != b && !cdt.can_add_constraint(a, b) {
            return false;
        }
        cdt.add_constraint(a, b);
    }
    true
}

fn point_in_polygon(p: Point, polygon: &[Point]) -> bool {
    let mut inside = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// Distancia de un punto a un contorno cerrado.
fn ring_distance(p: Point, ring: &[Point]) -> f64 {
    (0..ring.len())
        .map(|i| kurbo::Line::new(ring[i], ring[(i + 1) % ring.len()]).nearest(p, 1e-6).distance_sq.sqrt())
        .fold(f64::INFINITY, f64::min)
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let cross = |o: Point, p: Point, q: Point| (p - o).cross(q - o);
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
}

/// Si dos polilíneas abiertas se cruzan.
fn polylines_cross(a: &[Point], b: &[Point]) -> bool {
    a.windows(2).any(|s| b.windows(2).any(|t| segments_cross(s[0], s[1], t[0], t[1])))
}

/// Si dos contornos cerrados se cruzan.
fn rings_cross(a: &[Point], b: &[Point]) -> bool {
    let close = |r: &[Point]| r.iter().copied().chain(r.first().copied()).collect::<Vec<_>>();
    polylines_cross(&close(a), &close(b))
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

    #[test]
    fn holes_are_cut_out_and_can_be_sewn() {
        use pattern::SeamSide;
        let mut p = Pattern::new("t");
        let panel = p.add_rect("panel", Point::ZERO, 300.0, 200.0);
        let window = p.add_rect("ventana", Point::new(50.0, 80.0), 200.0, 30.0);
        p.piece_mut(window).unwrap().hole_in = Some(panel);
        let zip = p.add_rect("cremallera", Point::new(0.0, 300.0), 200.0, 30.0);
        let (top, zip_top) = (p.piece(window).unwrap().edges()[0].id, p.piece(zip).unwrap().edges()[0].id);
        p.add_seam("ventana", vec![SeamSide::new(window, top, false), SeamSide::new(zip, zip_top, false)]);

        let mesh = mesh_piece_in(&p, p.piece(panel).unwrap(), &MeshOptions::default()).unwrap();
        assert!((mesh.area() - (300.0 * 200.0 - 200.0 * 30.0)).abs() < 1e-6, "{}", mesh.area());
        assert_eq!(mesh.holes.len(), 1);
        assert!(mesh.edge_vertices(top).is_some());
        // Ningún triángulo dentro del agujero.
        for t in &mesh.triangles {
            let c = t.iter().fold([0.0, 0.0], |c, &i| {
                let q = mesh.positions[i as usize];
                [c[0] + q[0] / 3.0, c[1] + q[1] / 3.0]
            });
            assert!(!(c[0] > 50.0 && c[0] < 250.0 && c[1] > 80.0 && c[1] < 110.0), "{c:?}");
        }

        let all = mesh_pattern(&p, &MeshOptions::default());
        assert_eq!(all.pieces[1].as_ref().unwrap_err(), &MeshError::Hole);
        assert_eq!(all.seams.len(), 1);
        let seam = &all.seams[0];
        // El lado del agujero se cose en la malla del panel.
        assert_eq!(seam.sides[0].piece, 0);
        assert_eq!(seam.sides[0].vertices.len(), seam.sides[1].vertices.len());
        let pos = all.pieces[0].as_ref().unwrap().positions[seam.sides[0].vertices[0] as usize];
        assert_eq!(pos, [50.0, 80.0]);
    }

    #[test]
    fn inner_lines_become_mesh_edges() {
        use pattern::SeamSide;
        let mut p = Pattern::new("t");
        let front = p.add_rect("delantero", Point::ZERO, 300.0, 300.0);
        let line = p.add_line(front, Point::new(60.0, 110.0), Point::new(230.0, 110.0)).unwrap();
        let pocket = p.add_rect("bolsillo", Point::new(400.0, 0.0), 170.0, 150.0);
        let top = p.piece(pocket).unwrap().edges()[0].id;
        p.add_seam("bolsillo", vec![SeamSide::new(pocket, top, false), SeamSide::new(front, line, false)]);

        let all = mesh_pattern(&p, &MeshOptions::default());
        let mesh = all.pieces[0].as_ref().unwrap();
        // La línea no quita tela: el área es la del cuadrado entero.
        assert!((mesh.area() - 90_000.0).abs() < 1e-6, "{}", mesh.area());
        let vertices = mesh.edge_vertices(line).unwrap();
        assert!(vertices.len() >= 9);
        let edges = mesh.edges();
        for w in vertices.windows(2) {
            let e = if w[0] < w[1] { [w[0], w[1]] } else { [w[1], w[0]] };
            assert!(edges.binary_search(&e).is_ok(), "{e:?} no es arista de la malla");
        }
        // Los vértices de la línea son interiores: no son borde.
        let border = mesh.border_flags();
        assert!(vertices.iter().all(|&v| !border[v as usize]));
        // Cosida al bolsillo, punto a punto.
        assert_eq!(all.seams.len(), 1);
        assert_eq!(all.seams[0].sides[0].vertices.len(), all.seams[0].sides[1].vertices.len());

        // Una línea que se sale de la pieza no se malla, pero la pieza sí.
        p.add_line(front, Point::new(250.0, 200.0), Point::new(400.0, 200.0));
        let mesh = mesh_piece_in(&p, p.piece(front).unwrap(), &MeshOptions::default()).unwrap();
        assert_eq!(mesh.lines.len(), 1);
        assert!((mesh.area() - 90_000.0).abs() < 1e-6);
    }
}
