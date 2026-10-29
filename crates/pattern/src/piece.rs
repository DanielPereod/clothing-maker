use kurbo::{
    BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, ParamCurveNearest, PathEl, PathSeg, Point, Rect, Shape, Vec2,
};
use serde::{Deserialize, Serialize};

use crate::{EdgeId, PieceId};

/// Precisión (mm) para longitudes y búsquedas sobre curvas.
const ACCURACY: f64 = 1e-3;

/// Forma de una arista.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EdgeKind {
    Line,
    /// Bézier cúbica. Los manejadores son relativos: `h1` respecto al vértice
    /// inicial y `h2` respecto al final, así que al mover un vértice sus
    /// manejadores le acompañan.
    Cubic {
        h1: Vec2,
        h2: Vec2,
    },
}

/// Arista del contorno de una pieza. La arista `i` va del vértice `i` al
/// vértice `i + 1` (y la última vuelve al primero).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    #[serde(flatten)]
    pub kind: EdgeKind,
}

impl Edge {
    pub fn line(id: EdgeId) -> Self {
        Self { id, kind: EdgeKind::Line }
    }

    pub fn is_curve(&self) -> bool {
        matches!(self.kind, EdgeKind::Cubic { .. })
    }
}

/// Una pieza de tela: un contorno cerrado de vértices y aristas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawPiece")]
pub struct Piece {
    pub id: PieceId,
    pub name: String,
    vertices: Vec<Point>,
    edges: Vec<Edge>,
}

/// Forma serializada, para comprobar invariantes al deserializar.
#[derive(Deserialize)]
struct RawPiece {
    id: PieceId,
    name: String,
    vertices: Vec<Point>,
    edges: Vec<Edge>,
}

impl TryFrom<RawPiece> for Piece {
    type Error = String;

    fn try_from(raw: RawPiece) -> Result<Self, Self::Error> {
        let id = raw.id;
        Piece::from_parts(raw.id, raw.name, raw.vertices, raw.edges)
            .ok_or_else(|| format!("la pieza {} no tiene el mismo número de vértices que de aristas", id.0))
    }
}

impl Piece {
    /// Construye una pieza. Devuelve `None` si hay distinto número de
    /// vértices que de aristas.
    pub fn from_parts(id: PieceId, name: impl Into<String>, vertices: Vec<Point>, edges: Vec<Edge>) -> Option<Self> {
        (vertices.len() == edges.len()).then(|| Self { id, name: name.into(), vertices, edges })
    }

    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Índice de la arista con ese id.
    pub fn edge_index(&self, id: EdgeId) -> Option<usize> {
        self.edges.iter().position(|e| e.id == id)
    }

    /// Índices de los vértices inicial y final de la arista `i`.
    pub fn edge_endpoints(&self, i: usize) -> (usize, usize) {
        (i, (i + 1) % self.len())
    }

    /// Mueve un vértice. Los manejadores de sus aristas le acompañan.
    pub fn set_vertex(&mut self, i: usize, p: Point) {
        self.vertices[i] = p;
    }

    pub fn translate(&mut self, delta: Vec2) {
        for v in &mut self.vertices {
            *v += delta;
        }
    }

    pub fn set_edge_kind(&mut self, i: usize, kind: EdgeKind) {
        self.edges[i].kind = kind;
    }

    /// Convierte la arista `i` en curva (con los manejadores a 1/3 y 2/3 del
    /// segmento, así que al principio sigue siendo recta).
    pub fn make_curve(&mut self, i: usize) {
        if self.edges[i].is_curve() {
            return;
        }
        let (a, b) = self.edge_endpoints(i);
        let d = self.vertices[b] - self.vertices[a];
        self.edges[i].kind = EdgeKind::Cubic { h1: d / 3.0, h2: -d / 3.0 };
    }

    pub fn make_line(&mut self, i: usize) {
        self.edges[i].kind = EdgeKind::Line;
    }

    /// Puntos de control absolutos de la arista `i`, si es curva.
    pub fn control_points(&self, i: usize) -> Option<(Point, Point)> {
        let (a, b) = self.edge_endpoints(i);
        match self.edges[i].kind {
            EdgeKind::Line => None,
            EdgeKind::Cubic { h1, h2 } => Some((self.vertices[a] + h1, self.vertices[b] + h2)),
        }
    }

    /// Mueve un manejador (0 = el del vértice inicial, 1 = el del final) a
    /// una posición absoluta.
    pub fn set_control_point(&mut self, i: usize, handle: usize, p: Point) {
        let (a, b) = self.edge_endpoints(i);
        let (va, vb) = (self.vertices[a], self.vertices[b]);
        if let EdgeKind::Cubic { h1, h2 } = &mut self.edges[i].kind {
            if handle == 0 {
                *h1 = p - va;
            } else {
                *h2 = p - vb;
            }
        }
    }

    /// Geometría de la arista `i`.
    pub fn segment(&self, i: usize) -> PathSeg {
        let (a, b) = self.edge_endpoints(i);
        let (pa, pb) = (self.vertices[a], self.vertices[b]);
        match self.edges[i].kind {
            EdgeKind::Line => PathSeg::Line(Line::new(pa, pb)),
            EdgeKind::Cubic { h1, h2 } => PathSeg::Cubic(CubicBez::new(pa, pa + h1, pb + h2, pb)),
        }
    }

    pub fn segments(&self) -> impl Iterator<Item = PathSeg> + '_ {
        (0..self.len()).map(|i| self.segment(i))
    }

    /// Contorno cerrado como `BezPath`.
    pub fn to_bezpath(&self) -> BezPath {
        let mut path = BezPath::new();
        if self.is_empty() {
            return path;
        }
        path.move_to(self.vertices[0]);
        for seg in self.segments() {
            match seg {
                PathSeg::Line(l) => path.line_to(l.p1),
                PathSeg::Cubic(c) => path.curve_to(c.p1, c.p2, c.p3),
                PathSeg::Quad(q) => path.quad_to(q.p1, q.p2),
            }
        }
        path.close_path();
        path
    }

    /// Área con signo (mm²). Positiva si el contorno va en sentido horario en
    /// pantalla (Y hacia abajo).
    pub fn area(&self) -> f64 {
        self.to_bezpath().area()
    }

    pub fn perimeter(&self) -> f64 {
        (0..self.len()).map(|i| self.edge_length(i)).sum()
    }

    pub fn edge_length(&self, i: usize) -> f64 {
        self.segment(i).arclen(ACCURACY)
    }

    pub fn bounding_box(&self) -> Rect {
        self.to_bezpath().bounding_box()
    }

    pub fn contains(&self, p: Point) -> bool {
        self.len() >= 3 && self.to_bezpath().contains(p)
    }

    /// Arista más cercana a `p`: `(índice, parámetro t, distancia)`.
    pub fn nearest_edge(&self, p: Point) -> Option<(usize, f64, f64)> {
        self.segments()
            .enumerate()
            .map(|(i, seg)| {
                let n = seg.nearest(p, ACCURACY);
                (i, n.t, n.distance_sq.sqrt())
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
    }

    /// Parte la arista `i` en `t`, sin cambiar la forma. La primera mitad
    /// conserva el id de la arista; la segunda recibe `new_edge`.
    /// Devuelve el índice del vértice insertado.
    pub fn split_edge(&mut self, i: usize, t: f64, new_edge: EdgeId) -> Option<usize> {
        if i >= self.len() || !(0.0..=1.0).contains(&t) {
            return None;
        }
        let seg = self.segment(i);
        let mid = seg.eval(t);
        let (first, second) = match seg {
            PathSeg::Cubic(_) => {
                let (PathSeg::Cubic(c0), PathSeg::Cubic(c1)) = (seg.subsegment(0.0..t), seg.subsegment(t..1.0)) else {
                    unreachable!()
                };
                (
                    EdgeKind::Cubic { h1: c0.p1 - c0.p0, h2: c0.p2 - c0.p3 },
                    EdgeKind::Cubic { h1: c1.p1 - c1.p0, h2: c1.p2 - c1.p3 },
                )
            }
            _ => (EdgeKind::Line, EdgeKind::Line),
        };
        self.edges[i].kind = first;
        self.edges.insert(i + 1, Edge { id: new_edge, kind: second });
        self.vertices.insert(i + 1, mid);
        Some(i + 1)
    }

    /// Borra el vértice `v`, fundiendo sus dos aristas en una. Hace falta que
    /// queden al menos 3 vértices. Devuelve el id de la arista eliminada.
    pub fn remove_vertex(&mut self, v: usize) -> Option<EdgeId> {
        let n = self.len();
        if n <= 3 || v >= n {
            return None;
        }
        let prev = (v + n - 1) % n;
        let merged = match (self.edges[prev].kind, self.edges[v].kind) {
            (EdgeKind::Line, EdgeKind::Line) => EdgeKind::Line,
            (a, b) => {
                let h1 = match a {
                    EdgeKind::Cubic { h1, .. } => h1,
                    EdgeKind::Line => (self.vertices[v] - self.vertices[prev]) / 3.0,
                };
                let next = (v + 1) % n;
                let h2 = match b {
                    EdgeKind::Cubic { h2, .. } => h2,
                    EdgeKind::Line => (self.vertices[v] - self.vertices[next]) / 3.0,
                };
                EdgeKind::Cubic { h1, h2 }
            }
        };
        self.edges[prev].kind = merged;
        let removed = self.edges.remove(v).id;
        self.vertices.remove(v);
        Some(removed)
    }

    /// Contorno aproximado por un polígono (sin repetir el primer punto).
    pub fn flatten(&self, tolerance: f64) -> Vec<Point> {
        let mut out = Vec::new();
        kurbo::flatten(self.to_bezpath(), tolerance, |el| match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => out.push(p),
            _ => {}
        });
        if out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out
    }

    /// `true` si el contorno se corta a sí mismo.
    pub fn is_self_intersecting(&self, tolerance: f64) -> bool {
        polygon_self_intersects(&self.flatten(tolerance))
    }
}

/// Comprueba si un polígono cerrado tiene aristas no contiguas que se cortan.
pub fn polygon_self_intersects(poly: &[Point]) -> bool {
    let n = poly.len();
    if n < 4 {
        return false;
    }
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue; // contiguas por el cierre
            }
            let (c, d) = (poly[j], poly[(j + 1) % n]);
            if segments_intersect(a, b, c, d) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let cross = |o: Point, p: Point, q: Point| (p - o).cross(q - o);
    let d1 = cross(c, d, a);
    let d2 = cross(c, d, b);
    let d3 = cross(a, b, c);
    let d4 = cross(a, b, d);
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0) && d1 != 0.0 && d2 != 0.0 && d3 != 0.0 && d4 != 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pattern, Point};

    fn square() -> (Pattern, PieceId) {
        let mut p = Pattern::new("t");
        let id = p.add_rect("sq", Point::ZERO, 100.0, 100.0);
        (p, id)
    }

    #[test]
    fn rect_area_and_perimeter() {
        let (p, id) = square();
        let piece = p.piece(id).unwrap();
        assert!((piece.area().abs() - 10_000.0).abs() < 1e-9);
        assert!((piece.perimeter() - 400.0).abs() < 1e-6);
        assert!(piece.contains(Point::new(50.0, 50.0)));
        assert!(!piece.contains(Point::new(150.0, 50.0)));
    }

    #[test]
    fn make_curve_keeps_shape_until_handles_move() {
        let (mut p, id) = square();
        let piece = p.piece_mut(id).unwrap();
        piece.make_curve(0);
        assert!((piece.edge_length(0) - 100.0).abs() < 1e-3);
        piece.set_control_point(0, 0, Point::new(33.0, -40.0));
        piece.set_control_point(0, 1, Point::new(66.0, -40.0));
        assert!(piece.edge_length(0) > 100.0);
    }

    #[test]
    fn split_curve_preserves_geometry() {
        let (mut p, id) = square();
        let piece = p.piece_mut(id).unwrap();
        piece.make_curve(0);
        piece.set_control_point(0, 0, Point::new(20.0, -50.0));
        let before = piece.edge_length(0);
        let area_before = piece.area();
        piece.split_edge(0, 0.3, EdgeId(999)).unwrap();
        let after = piece.edge_length(0) + piece.edge_length(1);
        assert!((before - after).abs() < 1e-2);
        assert!((area_before - piece.area()).abs() < 1e-3);
    }

    #[test]
    fn remove_vertex_merges_edges() {
        let (mut p, id) = square();
        let removed_edge = p.piece(id).unwrap().edges()[1].id;
        p.split_edge(id, 0, 0.5);
        assert!(p.remove_vertex(id, 1));
        let piece = p.piece(id).unwrap();
        assert_eq!(piece.len(), 4);
        assert!(piece.edge_index(removed_edge).is_some());
        // No se puede bajar de 3 vértices.
        assert!(p.remove_vertex(id, 0));
        assert!(!p.remove_vertex(id, 0));
    }

    #[test]
    fn detects_self_intersection() {
        let mut p = Pattern::new("t");
        let bow = p.add_polygon(
            "pajarita",
            vec![Point::new(0.0, 0.0), Point::new(100.0, 100.0), Point::new(100.0, 0.0), Point::new(0.0, 100.0)],
        );
        assert!(p.piece(bow).unwrap().is_self_intersecting(0.1));
        let (p, sq) = square();
        assert!(!p.piece(sq).unwrap().is_self_intersecting(0.1));
    }

    #[test]
    fn nearest_edge_finds_closest() {
        let (p, id) = square();
        let (i, t, d) = p.piece(id).unwrap().nearest_edge(Point::new(100.0 + 5.0, 25.0)).unwrap();
        assert_eq!(i, 1);
        assert!((t - 0.25).abs() < 1e-6);
        assert!((d - 5.0).abs() < 1e-6);
    }
}
