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
use pattern::{EdgeId, MarkId, Pattern, Piece, PieceId, SeamId, polygon_self_intersects};
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
    /// El vértice de cada punto marcado (donde va un broche o una presilla)
    /// que cae en la tela.
    pub marks: Vec<(MarkId, u32)>,
}

/// Los vértices de la malla que siguen una línea interior.
#[derive(Clone, Debug, PartialEq)]
pub struct LineVertices {
    pub line: EdgeId,
    pub vertices: Vec<u32>,
    /// Dónde cae cada vértice, en fracciones (0..1) del largo de la línea.
    pub s: Vec<f64>,
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
        Some(self.edge_samples(edge)?.into_iter().map(|(v, _)| v).collect())
    }

    /// Como [`Self::edge_vertices`], con dónde cae cada vértice en la
    /// arista (fracciones 0..1 de su largo).
    pub fn edge_samples(&self, edge: EdgeId) -> Option<Vec<(u32, f64)>> {
        let in_loop = |contour: &[BoundaryVertex]| {
            let first = contour.iter().position(|b| b.edge == edge)?;
            let mut out: Vec<(u32, f64)> =
                contour[first..].iter().take_while(|b| b.edge == edge).map(|b| (b.vertex, b.s)).collect();
            let end = (first + out.len()) % contour.len();
            out.push((contour[end].vertex, 1.0));
            Some(out)
        };
        in_loop(&self.boundary).or_else(|| self.holes.iter().find_map(|h| in_loop(h))).or_else(|| {
            let line = self.lines.iter().find(|l| l.line == edge)?;
            Some(line.vertices.iter().copied().zip(line.s.iter().copied()).collect())
        })
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

/// Cómo se reparten los puntos de la malla a lo largo de una arista o una
/// línea: cortes (fracciones 0..1 de su largo, del 0 al 1) y en cuántos
/// tramos iguales se divide cada trozo entre dos cortes.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgePlan {
    pub breaks: Vec<f64>,
    pub counts: Vec<usize>,
}

impl EdgePlan {
    /// `n` tramos iguales.
    pub fn uniform(n: usize) -> Self {
        Self { breaks: vec![0.0, 1.0], counts: vec![n.max(1)] }
    }

    /// Dónde cae cada punto, del 0 al 1 incluidos.
    pub fn samples(&self) -> Vec<f64> {
        let mut out = Vec::new();
        for (k, &n) in self.counts.iter().enumerate() {
            let (a, b) = (self.breaks[k], self.breaks[k + 1]);
            out.extend((0..n.max(1)).map(|m| a + (b - a) * m as f64 / n.max(1) as f64));
        }
        out.push(1.0);
        out
    }
}

/// Fracciones de una arista que cuentan como el mismo punto.
const BREAK_EPS: f64 = 1e-7;

/// Un lado de costura ya repartido en trozos de arista (arista, trozo en su
/// [`EdgePlan`]), agrupados por los piquetes y en el sentido de la costura.
struct SideUnits {
    piece: usize,
    side: usize,
    spans: Vec<pattern::SideSpan>,
    reversed: bool,
    segments: Vec<Vec<(EdgeId, usize)>>,
}

/// Malla todas las piezas de un patrón. Los lados de cada costura reciben
/// el mismo número de puntos y se unen punto a punto. Entre cada dos
/// piquetes (si todos los lados tienen los mismos) cada lado pone el mismo
/// número de puntos, así que los piquetes caen uno sobre otro. Si un lado es
/// un tramo o una cadena de aristas, sus extremos y las esquinas entre
/// aristas son puntos de la malla.
pub fn mesh_pattern(pattern: &Pattern, options: &MeshOptions) -> PatternMesh {
    let target = options.target_edge_length.max(0.1);
    // Las aristas de un agujero se mallan con la pieza en la que está.
    let owner = |id: PieceId| pattern.piece(id).and_then(|p| p.hole_in).unwrap_or(id);
    let index: HashMap<PieceId, usize> = pattern.pieces.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let segment_of = |id: PieceId, edge: EdgeId| pattern.piece(id).and_then(|p| p.edge_segment(edge));

    // 1. Cortes de cada arista cosida: el principio y el final, los
    // extremos de los tramos y los piquetes que se usan.
    let mut breaks: HashMap<EdgeId, Vec<f64>> = HashMap::new();
    let mut sides: Vec<Vec<(SideUnits, Vec<f64>)>> = Vec::new();
    for seam in &pattern.seams {
        let notches: Vec<Vec<f64>> = seam.sides.iter().map(|s| pattern.side_notches(s)).collect();
        let use_notches = notches.iter().all(|n| n.len() == notches[0].len());
        let mut seam_sides = Vec::new();
        for (k, side) in seam.sides.iter().enumerate() {
            let (Some(&piece), Some(spans)) = (index.get(&owner(side.piece)), pattern.side_spans(side)) else {
                continue;
            };
            // Los piquetes, en el orden de la pieza.
            let mut anchors: Vec<f64> = if use_notches { notches[k].clone() } else { Vec::new() };
            if side.reversed {
                anchors = anchors.iter().rev().map(|f| 1.0 - f).collect();
            }
            let total: f64 = spans.iter().map(|s| s.length).sum();
            for span in &spans {
                let cuts = breaks.entry(span.edge).or_insert_with(|| vec![0.0, 1.0]);
                cuts.extend([span.from, span.to]);
            }
            let mut start = 0.0;
            for span in &spans {
                for &f in &anchors {
                    let at = (f * total - start) / span.length;
                    if at > 0.0 && at < 1.0 {
                        breaks.entry(span.edge).or_default().push(span.from + at * (span.to - span.from));
                    }
                }
                start += span.length;
            }
            let units = SideUnits { piece, side: k, spans, reversed: side.reversed, segments: Vec::new() };
            seam_sides.push((units, anchors));
        }
        sides.push(seam_sides);
    }
    for cuts in breaks.values_mut() {
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < BREAK_EPS);
    }

    // 2. Trozos de arista de cada lado, agrupados entre piquetes.
    for (units, anchors) in sides.iter_mut().flatten() {
        let total: f64 = units.spans.iter().map(|s| s.length).sum();
        let mut segments = vec![Vec::new(); anchors.len() + 1];
        let mut start = 0.0;
        for span in &units.spans {
            let cuts = &breaks[&span.edge];
            for k in 0..cuts.len() - 1 {
                let (a, b) = (cuts[k], cuts[k + 1]);
                if a < span.from - BREAK_EPS || b > span.to + BREAK_EPS {
                    continue;
                }
                let mid = (start + ((a + b) / 2.0 - span.from) / (span.to - span.from) * span.length) / total;
                let j = anchors.iter().filter(|&&f| f < mid).count();
                segments[j].push((span.edge, k));
            }
            start += span.length;
        }
        if units.reversed {
            segments.reverse();
            for segment in &mut segments {
                segment.reverse();
            }
        }
        units.segments = segments;
    }

    // 3. Tramos de cada trozo: los que pide su largo, y luego más en los
    // lados que se quedan cortos, hasta que todos los lados de cada costura
    // tienen los mismos entre cada dos piquetes.
    let mut counts: HashMap<(EdgeId, usize), usize> = HashMap::new();
    let mut lengths: HashMap<(EdgeId, usize), f64> = HashMap::new();
    for (units, _) in sides.iter().flatten() {
        let piece = &pattern.pieces[units.piece];
        for span in &units.spans {
            // La arista puede ser de un agujero de la pieza.
            let seg = segment_of(piece.id, span.edge)
                .or_else(|| pattern.holes_of(piece.id).find_map(|h| h.edge_segment(span.edge)));
            let Some(seg) = seg else { continue };
            let (len, curve) = (seg.arclen(1e-3), curve_min_divisions(&seg, options.curve_tolerance) as f64);
            let cuts = &breaks[&span.edge];
            for k in 0..cuts.len() - 1 {
                let part = cuts[k + 1] - cuts[k];
                let n = ((part * len / target).ceil() as usize).max((part * curve).ceil() as usize).max(1);
                counts.insert((span.edge, k), n);
                lengths.insert((span.edge, k), part * len);
            }
        }
    }
    for _ in 0..100 {
        let mut changed = false;
        for seam_sides in &sides {
            let parts = seam_sides.first().map_or(0, |(u, _)| u.segments.len());
            if seam_sides.iter().any(|(u, _)| u.segments.len() != parts) {
                continue;
            }
            for j in 0..parts {
                let sum = |u: &SideUnits, counts: &HashMap<(EdgeId, usize), usize>| -> usize {
                    u.segments[j].iter().map(|k| counts.get(k).copied().unwrap_or(1)).sum()
                };
                let max = seam_sides.iter().map(|(u, _)| sum(u, &counts)).max().unwrap_or(0);
                for (units, _) in seam_sides {
                    for _ in sum(units, &counts)..max {
                        // Un tramo más donde los tramos son más largos.
                        let longest = units.segments[j].iter().max_by(|a, b| {
                            let per = |k: &(EdgeId, usize)| lengths.get(k).copied().unwrap_or(0.0) / counts[k] as f64;
                            per(a).total_cmp(&per(b))
                        });
                        if let Some(k) = longest {
                            *counts.entry(*k).or_insert(1) += 1;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let plans: HashMap<EdgeId, EdgePlan> = breaks
        .iter()
        .map(|(&edge, cuts)| {
            let counts = (0..cuts.len() - 1).map(|k| counts.get(&(edge, k)).copied().unwrap_or(1)).collect();
            (edge, EdgePlan { breaks: cuts.clone(), counts })
        })
        .collect();

    let pieces: Vec<Result<Mesh2D, MeshError>> = pattern
        .pieces
        .iter()
        .map(|piece| {
            if piece.is_hole() {
                return Err(MeshError::Hole);
            }
            let holes: Vec<&Piece> = pattern.holes_of(piece.id).collect();
            mesh_piece_with(piece, &holes, options, |edge| plans.get(&edge).cloned())
        })
        .collect();

    // 4. Los vértices de cada lado, en el sentido de la costura.
    let mut seams = Vec::new();
    for (seam, seam_sides) in pattern.seams.iter().zip(&sides) {
        let mut out: Vec<SeamSideMesh> = Vec::new();
        for (units, _) in seam_sides {
            let Ok(mesh) = &pieces[units.piece] else { continue };
            let mut vertices: Vec<u32> = Vec::new();
            for span in &units.spans {
                let Some(samples) = mesh.edge_samples(span.edge) else { continue };
                let inside = samples.iter().filter(|(_, s)| *s > span.from - 1e-6 && *s < span.to + 1e-6);
                for &(v, _) in inside {
                    if vertices.last() != Some(&v) {
                        vertices.push(v);
                    }
                }
            }
            if units.reversed {
                vertices.reverse();
            }
            out.push(SeamSideMesh { piece: units.piece, side: units.side, vertices });
        }
        // Por si el remuestreo ha fundido algún punto (aristas casi nulas):
        // solo se cosen los lados con el número de puntos esperado.
        let expected = seam_sides
            .first()
            .map(|(u, _)| u.segments.iter().flatten().map(|k| counts.get(k).copied().unwrap_or(1)).sum::<usize>() + 1);
        out.retain(|s| Some(s.vertices.len()) == expected);
        if out.len() >= 2 {
            seams.push(SeamMesh { seam: seam.id, sides: out });
        }
    }
    PatternMesh { pieces, seams }
}

fn edge_divisions(seg: &PathSeg, target: f64, options: &MeshOptions) -> usize {
    let len = seg.arclen(1e-3);
    ((len / target).ceil() as usize).max(curve_min_divisions(seg, options.curve_tolerance)).max(1)
}

/// Malla una pieza con sus agujeros (`holes`, piezas con su contorno).
/// `plan` puede fijar cómo se reparten los puntos a lo largo de una arista
/// o una línea interior (si devuelve `None`, en tramos iguales de la
/// longitud objetivo).
///
/// Un agujero que no cabe en la pieza o una línea que se sale de la tela o
/// cruza otra se dejan fuera (el patrón avisa al validarlo) en vez de
/// estropear la malla entera.
pub fn mesh_piece_with(
    piece: &Piece,
    holes: &[&Piece],
    options: &MeshOptions,
    plan: impl Fn(EdgeId) -> Option<EdgePlan>,
) -> Result<Mesh2D, MeshError> {
    if piece.len() < 3 {
        return Err(MeshError::TooFewVertices);
    }
    let target = options.target_edge_length.max(0.1);
    let count = |edge: EdgeId, seg: &PathSeg| {
        plan(edge).unwrap_or_else(|| EdgePlan::uniform(edge_divisions(seg, target, options)))
    };

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
    let mut lines: Vec<(EdgeId, Vec<Point>, Vec<f64>)> = Vec::new();
    for line in &piece.lines {
        let seg = line.segment();
        let len = seg.arclen(1e-3);
        if len < 1e-6 {
            continue;
        }
        let samples = count(line.id, &seg).samples();
        let mut points: Vec<Point> = samples.iter().map(|&f| seg.eval(seg.inv_arclen(f * len, 1e-4))).collect();
        let clear = |p: Point| {
            in_fabric(p) && std::iter::once(&polygon).chain(&rings).all(|ring| ring_distance(p, ring) > LINE_GAP)
        };
        // Un doblez va de un vértice del contorno a otro: sus extremos son
        // esos puntos del contorno.
        let fold = line.fold.is_some();
        if fold && points.len() >= 2 {
            let last = points.len() - 1;
            (points[0], points[last]) = (line.from, line.to);
        }
        let on_outline = |p: Point| polygon.iter().any(|q| q.distance(p) < 1e-9);
        let n = points.len();
        let fits = points
            .iter()
            .enumerate()
            .all(|(k, &p)| if fold && (k == 0 || k + 1 == n) { on_outline(p) } else { clear(p) });
        if fits && !lines.iter().any(|(_, other, _)| polylines_cross(&points, other)) {
            lines.push((line.id, points, samples));
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
    for (id, points, samples) in &lines {
        let hs: Vec<_> = points.iter().map(|&p| insert(&mut cdt, p)).collect::<Result<_, _>>()?;
        let distinct = hs.windows(2).all(|w| w[0] != w[1]);
        if distinct && hs.windows(2).all(|w| cdt.can_add_constraint(w[0], w[1])) {
            for w in hs.windows(2) {
                cdt.add_constraint(w[0], w[1]);
            }
            line_handles.push((*id, hs, samples));
        }
    }
    // Los puntos marcados que caen en la tela, lejos del borde.
    let mut mark_handles = Vec::new();
    for mark in &piece.marks {
        let clear = std::iter::once(&polygon).chain(&rings).all(|ring| ring_distance(mark.at, ring) > LINE_GAP);
        if in_fabric(mark.at) && clear {
            mark_handles.push((mark.id, insert(&mut cdt, mark.at)?));
        }
    }
    // Sin líneas interiores, spade sabe qué caras quedan fuera (el exterior
    // y los agujeros, por paridad). Una línea abierta le confunde, así que
    // entonces se refina todo y se mira después qué triángulos son tela.
    let simple = line_handles.is_empty();
    let max_area = 3f64.sqrt() / 4.0 * target * target;
    // Sin el área mínima, spade intenta arreglar las astillas que quedan
    // entre puntos casi alineados de un lado inclinado (fuera de la tela) y
    // a veces se pierde. Si aun así falla, la pieza no se malla, pero la
    // aplicación sigue.
    let parameters = RefinementParameters::<f64>::new()
        .exclude_outer_faces(simple)
        .keep_constraint_edges()
        .with_max_allowed_area(max_area)
        .with_min_required_area(max_area * 1e-3)
        .with_max_additional_vertices(200_000);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cdt.refine(parameters)))
        .map_err(|_| MeshError::Triangulation("spade no ha podido refinar la malla".into()))?;
    let excluded: std::collections::HashSet<_> = result.excluded_faces.into_iter().collect();
    let fabric = if simple { None } else { Some(fabric_faces(&cdt, std::iter::once(&handles).chain(&hole_handles))) };

    // 4. Exportación, solo con los vértices que usan los triángulos de tela.
    let triangles: Vec<[u32; 3]> = cdt
        .inner_faces()
        .filter(|f| match &fabric {
            None => !excluded.contains(&f.fix()),
            Some(fabric) => fabric[f.fix().index()],
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
        .map(|(line, hs, samples)| LineVertices {
            line: *line,
            vertices: hs.iter().map(|&h| at(h)).collect(),
            s: samples.to_vec(),
        })
        .collect();
    let marks = mark_handles.iter().map(|&(id, h)| (id, at(h))).collect();
    Ok(Mesh2D { positions, triangles, boundary, holes, lines, marks })
}

/// Puntos del contorno de una pieza repartidos por longitud de arco: cada
/// arista en los tramos que diga `count`. Cada punto lleva su arista y su
/// posición en ella (0..1).
fn resample(piece: &Piece, plan: &impl Fn(EdgeId, &PathSeg) -> EdgePlan) -> Vec<(Point, EdgeId, f64)> {
    let mut out: Vec<(Point, EdgeId, f64)> = Vec::new();
    for (i, seg) in piece.segments().enumerate() {
        let edge = piece.edges()[i].id;
        let len = seg.arclen(1e-3);
        let samples = plan(edge, &seg).samples();
        for &s in &samples[..samples.len() - 1] {
            let t = if s == 0.0 { 0.0 } else { seg.inv_arclen(s * len, 1e-4) };
            out.push((seg.eval(t), edge, s));
        }
    }
    out.dedup_by(|a, b| a.0.distance(b.0) < 1e-6);
    if out.len() > 1 && out[0].0.distance(out[out.len() - 1].0) < 1e-6 {
        out.pop();
    }
    out
}

/// Qué caras (por índice) son tela: se recorren desde fuera y cada vez que
/// se cruza el contorno o el borde de un agujero (no una línea interior) se
/// pasa de fuera a dentro o al revés. Mirar si el centro de cada triángulo
/// cae dentro no basta: en un lado inclinado quedan astillas planas entre
/// puntos casi alineados del contorno, con el centro justo en el borde.
fn fabric_faces<'a>(
    cdt: &ConstrainedDelaunayTriangulation<Point2<f64>>,
    rings: impl Iterator<Item = &'a Vec<spade::handles::FixedVertexHandle>>,
) -> Vec<bool> {
    let mut borders = std::collections::HashSet::new();
    for ring in rings {
        for i in 0..ring.len() {
            let (a, b) = (ring[i].index(), ring[(i + 1) % ring.len()].index());
            borders.insert((a.min(b), a.max(b)));
        }
    }
    let crosses = |e: spade::handles::DirectedEdgeHandle<'_, Point2<f64>, (), spade::CdtEdge<()>, ()>| {
        let (a, b) = (e.from().fix().index(), e.to().fix().index());
        borders.contains(&(a.min(b), a.max(b)))
    };
    let mut inside: Vec<Option<bool>> = vec![None; cdt.num_all_faces()];
    let mut queue = std::collections::VecDeque::new();
    for face in cdt.inner_faces() {
        for e in face.adjacent_edges() {
            if e.rev().face().is_outer() && inside[face.fix().index()].is_none() {
                inside[face.fix().index()] = Some(crosses(e));
                queue.push_back(face);
            }
        }
    }
    while let Some(face) = queue.pop_front() {
        let here = inside[face.fix().index()].unwrap_or(false);
        for e in face.adjacent_edges() {
            let Some(next) = e.rev().face().as_inner() else { continue };
            if inside[next.fix().index()].is_none() {
                inside[next.fix().index()] = Some(here != crosses(e));
                queue.push_back(next);
            }
        }
    }
    inside.into_iter().map(|i| i.unwrap_or(false)).collect()
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

    /// El ángulo más pequeño (en grados) de un triángulo de la malla.
    fn min_angle(mesh: &Mesh2D, t: [u32; 3]) -> f64 {
        let p = t.map(|i| Point::new(mesh.positions[i as usize][0], mesh.positions[i as usize][1]));
        (0..3)
            .map(|i| {
                let (u, v) = (p[(i + 1) % 3] - p[i], p[(i + 2) % 3] - p[i]);
                (u.dot(v) / (u.hypot() * v.hypot())).clamp(-1.0, 1.0).acos().to_degrees()
            })
            .fold(f64::INFINITY, f64::min)
    }

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

    /// Posición 2D del vértice `k` de un lado de costura.
    fn side_pos(mesh: &PatternMesh, side: &SeamSideMesh, k: usize) -> Point {
        let [x, y] = mesh.pieces[side.piece].as_ref().unwrap().positions[side.vertices[k] as usize];
        Point::new(x, y)
    }

    /// Una base redonda de cuatro curvas cosida de una vez al bajo de un
    /// cuerpo: el lado de la base da la vuelta entera y vuelve al principio.
    #[test]
    fn chains_mesh_as_one_side() {
        use pattern::{EdgeKind, SeamSide, Vec2};
        let mut p = Pattern::new("t");
        let r = 60.0;
        let body = p.add_rect("cuerpo", Point::ZERO, 2.0 * std::f64::consts::PI * r, 130.0);
        let c = Point::new(200.0, 300.0);
        let pts = [(0.0, -r), (r, 0.0), (0.0, r), (-r, 0.0)].map(|(x, y)| c + Vec2::new(x, y));
        let base = p.add_polygon("base", pts.to_vec());
        for i in 0..4 {
            let tangent = |q: Point| Vec2::new((c - q).y, -(c - q).x).normalize() * r * 0.552_284_75;
            let kind = EdgeKind::Cubic { h1: tangent(pts[i]), h2: -tangent(pts[(i + 1) % 4]) };
            p.piece_mut(base).unwrap().set_edge_kind(i, kind);
        }
        let e = |p: &Pattern, id, i: usize| p.piece(id).unwrap().edges()[i].id;
        let around = SeamSide::new(base, e(&p, base, 0), false).through(e(&p, base, 3));
        p.add_seam("base", vec![SeamSide::new(body, e(&p, body, 2), true), around]).unwrap();
        let mesh = mesh_pattern(&p, &MeshOptions::default());
        let seam = &mesh.seams[0];
        let (bottom, ring) = (&seam.sides[0].vertices, &seam.sides[1].vertices);
        assert_eq!(bottom.len(), ring.len());
        assert!(ring.len() > 19, "{}", ring.len());
        assert_eq!(ring.first(), ring.last());
        // Las cuatro esquinas de la base son puntos del lado.
        for corner in pts {
            assert!((0..ring.len()).any(|k| side_pos(&mesh, &seam.sides[1], k).distance(corner) < 1e-9));
        }
    }

    /// Un tramo empieza y acaba donde dice, y los piquetes de los dos lados
    /// caen en el mismo punto de la costura: el fruncido se reparte como
    /// dicen.
    #[test]
    fn ranges_and_notches_place_the_points() {
        use pattern::SeamSide;
        let mut p = Pattern::new("t");
        let skirt = p.add_rect("falda", Point::ZERO, 300.0, 200.0);
        let band = p.add_rect("cinturilla", Point::new(0.0, 300.0), 200.0, 40.0);
        let (top, bottom) = (p.piece(skirt).unwrap().edges()[0].id, p.piece(band).unwrap().edges()[2].id);
        // La falda frunce entera en la mitad derecha de la cinturilla (su
        // arista de abajo va de x = 200 a x = 0)...
        p.piece_mut(skirt).unwrap().toggle_notch(top, 0.8, 1.0);
        p.piece_mut(band).unwrap().toggle_notch(bottom, 0.25, 1.0);
        let half = SeamSide::new(band, bottom, true).span(0.0, 0.5);
        p.add_seam("cintura", vec![SeamSide::new(skirt, top, false), half]).unwrap();
        // ... y la otra mitad de la misma arista se cose a otra pieza.
        let tab = p.add_rect("trabilla", Point::new(400.0, 0.0), 100.0, 30.0);
        let tab_top = p.piece(tab).unwrap().edges()[0].id;
        let rest = SeamSide::new(band, bottom, true).span(0.5, 1.0);
        p.add_seam("trabilla", vec![SeamSide::new(tab, tab_top, false), rest]).unwrap();

        let mesh = mesh_pattern(&p, &MeshOptions::default());
        assert_eq!(mesh.seams.len(), 2);
        let seam = &mesh.seams[0];
        let n = seam.sides[0].vertices.len();
        assert_eq!(seam.sides[1].vertices.len(), n);
        // La cinturilla va invertida: del medio, x = 100, al extremo, x = 200.
        assert!(side_pos(&mesh, &seam.sides[1], 0).distance(Point::new(100.0, 340.0)) < 1e-9);
        assert!(side_pos(&mesh, &seam.sides[1], n - 1).distance(Point::new(200.0, 340.0)) < 1e-9);
        // Los piquetes: x = 240 en la falda y x = 150 en la cinturilla, en
        // el mismo punto de la costura.
        let k = (0..n).find(|&k| side_pos(&mesh, &seam.sides[0], k).distance(Point::new(240.0, 0.0)) < 1e-9);
        let k = k.expect("el piquete de la falda es un punto de la costura");
        assert!(side_pos(&mesh, &seam.sides[1], k).distance(Point::new(150.0, 340.0)) < 1e-9);
        // El otro tramo acaba en el medio de la cinturilla.
        let other = &mesh.seams[1];
        let m = other.sides[1].vertices.len();
        assert!(side_pos(&mesh, &other.sides[1], 0).distance(Point::new(0.0, 340.0)) < 1e-9);
        assert!(side_pos(&mesh, &other.sides[1], m - 1).distance(Point::new(100.0, 340.0)) < 1e-9);
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

    /// Un doblez va de borde a borde: sus extremos son puntos del contorno y
    /// ningún triángulo lo cruza.
    #[test]
    fn marks_become_vertices() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 200.0, 100.0);
        let b = p.add_rect("b", Point::new(0.0, 150.0), 200.0, 100.0);
        let at = Point::new(73.3, 41.7);
        p.add_fastener(pattern::FastenerKind::Snap, (a, at), (b, Point::new(20.0, 170.0))).unwrap();
        let piece = p.piece(a).unwrap();
        let mesh = mesh_piece_in(&p, piece, &MeshOptions::default()).unwrap();
        let [(id, v)] = mesh.marks[..] else { panic!("{:?}", mesh.marks) };
        assert_eq!(id, piece.marks[0].id);
        assert_eq!(mesh.positions[v as usize], [at.x, at.y]);
        assert!(mesh.triangles.iter().any(|t| t.contains(&v)));
    }

    #[test]
    fn folds_split_the_mesh() {
        let mut p = Pattern::new("t");
        let id = p.add_rect("puño", Point::ZERO, 300.0, 120.0);
        let fold = p.add_fold(id, Point::new(0.0, 60.0), Point::new(300.0, 60.0)).unwrap();
        let mesh = mesh_piece_in(&p, p.piece(id).unwrap(), &MeshOptions::default()).unwrap();
        assert!((mesh.area() - 300.0 * 120.0).abs() < 1.0, "área {}", mesh.area());
        let line = mesh.lines.iter().find(|l| l.line == fold).expect("el doblez está en la malla");
        let border: Vec<u32> = mesh.boundary.iter().map(|b| b.vertex).collect();
        assert!(border.contains(&line.vertices[0]) && border.contains(line.vertices.last().unwrap()));
        for t in &mesh.triangles {
            let ys = t.map(|v| mesh.positions[v as usize][1]);
            assert!(ys.iter().all(|&y| y <= 60.0 + 1e-6) || ys.iter().all(|&y| y >= 60.0 - 1e-6), "{ys:?}");
        }
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

    /// Con líneas interiores también se mallan piezas de lados inclinados o
    /// curvos (sus puntos no quedan del todo alineados) en cualquier tamaño,
    /// sin astillas planas a lo largo del lado inclinado (en la simulación,
    /// un triángulo sin área hace explotar la tela).
    #[test]
    fn slanted_pieces_with_lines_mesh() {
        for size in [12.0, 15.0, 20.0, 25.0, 30.0, 40.0] {
            for taper in [3.0, 10.0, 37.0] {
                let mut p = Pattern::new("t");
                let pts = [(0.0, 0.0), (380.0, 0.0), (380.0 - taper, 410.0), (taper, 410.0)];
                let piece = p.add_polygon("forro", pts.map(|(x, y)| Point::new(x, 500.0 + y)).to_vec());
                let corner = |x: f64, y: f64| Point::new(110.0 + x, 610.0 + y);
                p.add_line(piece, corner(0.0, 0.0), corner(0.0, 160.0));
                p.add_line(piece, corner(0.0, 160.0), corner(160.0, 160.0));
                p.add_line(piece, corner(160.0, 160.0), corner(160.0, 0.0));
                p.piece_mut(piece).unwrap().make_curve(0);
                let options = MeshOptions { target_edge_length: size, ..Default::default() };
                let mesh = mesh_piece_in(&p, p.piece(piece).unwrap(), &options).unwrap();
                assert_eq!(mesh.lines.len(), 3, "{size} {taper}");
                let expected = p.piece(piece).unwrap().area().abs();
                assert!((mesh.area() - expected).abs() < 5e-3 * expected, "{size} {taper}: {}", mesh.area());
                for &t in &mesh.triangles {
                    assert!(min_angle(&mesh, t) > 20.0, "{size} {taper}: astilla {t:?}");
                }
            }
        }
    }
}
