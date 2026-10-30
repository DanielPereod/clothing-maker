use kurbo::{
    BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, ParamCurveDeriv, ParamCurveNearest, PathEl, PathSeg, Point,
    Rect, Shape, Vec2,
};
use serde::{Deserialize, Serialize};

use crate::{EdgeId, PieceId, TextileId};

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
    /// Margen de costura (mm) de este borde. `None` = el del patrón.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance: Option<f64>,
}

impl Edge {
    pub fn line(id: EdgeId) -> Self {
        Self { id, kind: EdgeKind::Line, allowance: None }
    }

    pub fn is_curve(&self) -> bool {
        matches!(self.kind, EdgeKind::Cubic { .. })
    }
}

/// Línea dentro de una pieza. No corta la tela: la malla la sigue, así que
/// se puede coser encima otra pieza (un bolsillo, un asa, una separación).
/// Sus extremos tienen que quedar dentro de la pieza.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InnerLine {
    /// Del mismo tipo que los de las aristas: una costura puede usar una
    /// línea como uno de sus lados.
    pub id: EdgeId,
    pub from: Point,
    pub to: Point,
    #[serde(flatten)]
    pub kind: EdgeKind,
}

impl InnerLine {
    pub fn line(id: EdgeId, from: Point, to: Point) -> Self {
        Self { id, from, to, kind: EdgeKind::Line }
    }

    pub fn segment(&self) -> PathSeg {
        match self.kind {
            EdgeKind::Line => PathSeg::Line(Line::new(self.from, self.to)),
            EdgeKind::Cubic { h1, h2 } => {
                PathSeg::Cubic(CubicBez::new(self.from, self.from + h1, self.to + h2, self.to))
            }
        }
    }

    pub fn length(&self) -> f64 {
        self.segment().arclen(ACCURACY)
    }
}

/// Dónde se coloca una pieza en 3D antes de coser, mirando la prenda (u
/// objeto) de frente. Las piezas se dibujan en 2D vistas desde fuera, con el
/// derecho de la tela hacia quien mira: la espalda, por ejemplo, se dibuja
/// como se vería desde detrás.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrangement {
    #[default]
    Front,
    Back,
    Left,
    Right,
    /// Tumbada encima, con el borde superior del dibujo hacia detrás.
    Top,
    /// Tumbada debajo, con el borde superior del dibujo hacia delante.
    Bottom,
}

impl Arrangement {
    pub const ALL: [Arrangement; 6] = [
        Arrangement::Front,
        Arrangement::Back,
        Arrangement::Left,
        Arrangement::Right,
        Arrangement::Top,
        Arrangement::Bottom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Arrangement::Front => "Delante",
            Arrangement::Back => "Detrás",
            Arrangement::Left => "Izquierda",
            Arrangement::Right => "Derecha",
            Arrangement::Top => "Arriba",
            Arrangement::Bottom => "Abajo",
        }
    }

    fn is_default(&self) -> bool {
        *self == Arrangement::default()
    }
}

/// Zona del cuerpo del maniquí alrededor de la que se coloca una pieza
/// (sus «puntos de colocación»). Dentro de la zona, [`Arrangement`] dice en
/// qué lado va. Sin maniquí no se usa.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyZone {
    #[default]
    Torso,
    /// Brazo derecho del maniquí (a la izquierda de quien lo mira de frente).
    RightArm,
    LeftArm,
    RightLeg,
    LeftLeg,
}

impl BodyZone {
    pub const ALL: [BodyZone; 5] =
        [BodyZone::Torso, BodyZone::RightArm, BodyZone::LeftArm, BodyZone::RightLeg, BodyZone::LeftLeg];

    pub fn label(self) -> &'static str {
        match self {
            BodyZone::Torso => "Torso",
            BodyZone::RightArm => "Brazo derecho",
            BodyZone::LeftArm => "Brazo izquierdo",
            BodyZone::RightLeg => "Pierna derecha",
            BodyZone::LeftLeg => "Pierna izquierda",
        }
    }

    fn is_default(&self) -> bool {
        *self == BodyZone::default()
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
    /// Colocación en 3D antes de coser.
    #[serde(default, skip_serializing_if = "Arrangement::is_default")]
    pub arrangement: Arrangement,
    /// Zona del maniquí en la que se coloca.
    #[serde(default, skip_serializing_if = "BodyZone::is_default")]
    pub zone: BodyZone,
    /// Tela de la pieza (la capa de fuera), de la biblioteca del patrón.
    #[serde(default, skip_serializing_if = "TextileId::is_cotton")]
    pub fabric: TextileId,
    /// Capas de tela extra cosidas o pegadas a toda la pieza, por dentro,
    /// de fuera hacia dentro (forro, entretela...). La firmeza, el peso y el
    /// grosor de todas las capas se suman.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<TextileId>,
    /// El derecho de la tela mira hacia dentro de la prenda (forros, vistas):
    /// se dibuja igual, vista desde fuera, pero lo que se ve es el revés.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub right_side_in: bool,
    /// Líneas interiores: dónde se cose algo encima de la pieza.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<InnerLine>,
    /// Si la pieza es un agujero: el contorno que se recorta de otra pieza
    /// (una ventana para una cremallera, un asa troquelada). No es tela, pero
    /// sus aristas se pueden coser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hole_in: Option<PieceId>,
    /// Capa en 3D: 0 es la de fuera (el exterior), 1 el forro, que va
    /// dentro, y así hacia dentro. Las piezas de cada capa se colocan aparte,
    /// una dentro de otra.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub layer: u8,
    /// Encima de qué pieza se coloca (un bolsillo de parche, un asa, un
    /// refuerzo). Si no se dice, va encima de la pieza a cuyas líneas
    /// interiores esté cosida ([`crate::Pattern::mount`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Mount>,
}

/// Una pieza colocada encima de otra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub piece: PieceId,
    /// Va por el revés de la otra pieza en vez de por el derecho (por
    /// ejemplo, una vista que se vuelve hacia dentro por una ventana).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wrong_side: bool,
}

/// Nombre de una capa para la interfaz: la de fuera, el forro y las demás
/// por número.
pub fn layer_label(layer: u8) -> String {
    match layer {
        0 => "Exterior".to_owned(),
        1 => "Forro".to_owned(),
        n => format!("Capa {}", n + 1),
    }
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}

/// Forma serializada, para comprobar invariantes al deserializar.
#[derive(Deserialize)]
struct RawPiece {
    id: PieceId,
    name: String,
    vertices: Vec<Point>,
    edges: Vec<Edge>,
    #[serde(default)]
    arrangement: Arrangement,
    #[serde(default)]
    zone: BodyZone,
    #[serde(default)]
    fabric: TextileId,
    #[serde(default)]
    layers: Vec<TextileId>,
    #[serde(default)]
    right_side_in: bool,
    #[serde(default)]
    lines: Vec<InnerLine>,
    #[serde(default)]
    hole_in: Option<PieceId>,
    #[serde(default)]
    layer: u8,
    #[serde(default)]
    on: Option<Mount>,
}

impl TryFrom<RawPiece> for Piece {
    type Error = String;

    fn try_from(raw: RawPiece) -> Result<Self, Self::Error> {
        let id = raw.id;
        Piece::from_parts(raw.id, raw.name, raw.vertices, raw.edges)
            .map(|p| Piece {
                arrangement: raw.arrangement,
                zone: raw.zone,
                fabric: raw.fabric,
                layers: raw.layers,
                right_side_in: raw.right_side_in,
                lines: raw.lines,
                hole_in: raw.hole_in,
                layer: raw.layer,
                on: raw.on,
                ..p
            })
            .ok_or_else(|| format!("la pieza {} no tiene el mismo número de vértices que de aristas", id.0))
    }
}

impl Piece {
    /// Construye una pieza. Devuelve `None` si hay distinto número de
    /// vértices que de aristas.
    pub fn from_parts(id: PieceId, name: impl Into<String>, vertices: Vec<Point>, edges: Vec<Edge>) -> Option<Self> {
        (vertices.len() == edges.len()).then(|| Self {
            id,
            name: name.into(),
            vertices,
            edges,
            arrangement: Arrangement::default(),
            zone: BodyZone::default(),
            fabric: TextileId::default(),
            layers: Vec::new(),
            right_side_in: false,
            lines: Vec::new(),
            hole_in: None,
            layer: 0,
            on: None,
        })
    }

    /// Todas las capas de tela, de fuera hacia dentro.
    pub fn textiles(&self) -> impl Iterator<Item = TextileId> + '_ {
        std::iter::once(self.fabric).chain(self.layers.iter().copied())
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

    /// Índice de la línea interior con ese id.
    pub fn line_index(&self, id: EdgeId) -> Option<usize> {
        self.lines.iter().position(|l| l.id == id)
    }

    /// Geometría de una arista o de una línea interior, por su id.
    pub fn edge_segment(&self, id: EdgeId) -> Option<PathSeg> {
        match self.edge_index(id) {
            Some(i) => Some(self.segment(i)),
            None => self.lines.iter().find(|l| l.id == id).map(InnerLine::segment),
        }
    }

    /// Si es un agujero de otra pieza.
    pub fn is_hole(&self) -> bool {
        self.hole_in.is_some()
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
        for line in &mut self.lines {
            line.from += delta;
            line.to += delta;
        }
    }

    /// Aplica una transformación rígida: `point` a los vértices y extremos
    /// de líneas, `vector` a los manejadores (relativos).
    pub fn transform(&mut self, point: impl Fn(Point) -> Point, vector: impl Fn(Vec2) -> Vec2) {
        let kind = |k: EdgeKind| match k {
            EdgeKind::Line => EdgeKind::Line,
            EdgeKind::Cubic { h1, h2 } => EdgeKind::Cubic { h1: vector(h1), h2: vector(h2) },
        };
        for v in &mut self.vertices {
            *v = point(*v);
        }
        for e in &mut self.edges {
            e.kind = kind(e.kind);
        }
        for line in &mut self.lines {
            (line.from, line.to, line.kind) = (point(line.from), point(line.to), kind(line.kind));
        }
    }

    /// Refleja la pieza en la recta vertical `x = axis`, como al darle la
    /// vuelta a la tela. Los vértices se recorren al revés para que el
    /// contorno siga en el mismo sentido; cada arista conserva su id.
    pub fn mirror_x(&mut self, axis: f64) {
        let point = |p: Point| Point::new(2.0 * axis - p.x, p.y);
        let vector = |v: Vec2| Vec2::new(-v.x, v.y);
        let n = self.len();
        let vertices: Vec<Point> = (0..n).map(|k| point(self.vertices[(n - k) % n])).collect();
        // La arista k nueva es la n-1-k vieja recorrida al revés.
        let edges = (0..n)
            .map(|k| {
                let old = self.edges[(2 * n - k - 1) % n];
                let kind = match old.kind {
                    EdgeKind::Line => EdgeKind::Line,
                    EdgeKind::Cubic { h1, h2 } => EdgeKind::Cubic { h1: vector(h2), h2: vector(h1) },
                };
                Edge { kind, ..old }
            })
            .collect();
        self.vertices = vertices;
        self.edges = edges;
        for line in &mut self.lines {
            line.from = point(line.from);
            line.to = point(line.to);
            if let EdgeKind::Cubic { h1, h2 } = line.kind {
                line.kind = EdgeKind::Cubic { h1: vector(h1), h2: vector(h2) };
            }
        }
    }

    /// Despliega la pieza «al doblez»: la arista recta `i` es el doblez de la
    /// tela, y la pieza pasa a ser la entera, con la otra mitad reflejada en
    /// esa arista. La arista del doblez desaparece; las demás conservan su
    /// id y las reflejadas toman los de `new_ids` (hacen falta `len() - 1`
    /// para el contorno y uno por cada línea interior). Devuelve `false`, sin
    /// cambiar nada, si la arista no es recta o faltan ids.
    pub fn unfold(&mut self, i: usize, new_ids: &mut impl Iterator<Item = EdgeId>) -> bool {
        let n = self.len();
        if i >= n || self.edges[i].is_curve() || n < 3 {
            return false;
        }
        let (a, b) = (self.vertices[i], self.vertices[(i + 1) % n]);
        let axis = b - a;
        if axis.hypot2() < 1e-12 {
            return false;
        }
        let reflect_vec = |v: Vec2| axis * (2.0 * v.dot(axis) / axis.hypot2()) - v;
        let reflect = |p: Point| a + reflect_vec(p - a);
        // De B a A por el contorno, sin la arista del doblez.
        let path: Vec<usize> = (1..=n).map(|k| (i + k) % n).collect();
        let kept_edges: Vec<Edge> = (1..n).map(|k| self.edges[(i + k) % n]).collect();
        let mut vertices: Vec<Point> = path.iter().map(|&v| self.vertices[v]).collect();
        let mut edges = kept_edges.clone();
        // La mitad reflejada, de A a B: el mismo camino al revés, sin A ni B,
        // que están en el doblez.
        vertices.extend(path[1..n - 1].iter().rev().map(|&v| reflect(self.vertices[v])));
        for old in kept_edges.iter().rev() {
            let Some(id) = new_ids.next() else { return false };
            let kind = match old.kind {
                EdgeKind::Line => EdgeKind::Line,
                EdgeKind::Cubic { h1, h2 } => EdgeKind::Cubic { h1: reflect_vec(h2), h2: reflect_vec(h1) },
            };
            edges.push(Edge { id, kind, allowance: old.allowance });
        }
        let mut lines = self.lines.clone();
        for line in &self.lines {
            let Some(id) = new_ids.next() else { return false };
            let kind = match line.kind {
                EdgeKind::Line => EdgeKind::Line,
                EdgeKind::Cubic { h1, h2 } => EdgeKind::Cubic { h1: reflect_vec(h1), h2: reflect_vec(h2) },
            };
            lines.push(InnerLine { id, from: reflect(line.from), to: reflect(line.to), kind });
        }
        self.vertices = vertices;
        self.edges = edges;
        self.lines = lines;
        true
    }

    pub fn set_edge_kind(&mut self, i: usize, kind: EdgeKind) {
        self.edges[i].kind = kind;
    }

    /// Pone el margen de costura (mm) de la arista `i`; `None` = el del patrón.
    pub fn set_edge_allowance(&mut self, i: usize, allowance: Option<f64>) {
        if let Some(edge) = self.edges.get_mut(i) {
            edge.allowance = allowance.map(|a| a.max(0.0));
        }
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

    /// Normal unitaria hacia fuera de la pieza en el punto `t` de la arista `i`.
    pub fn outward_normal(&self, i: usize, t: f64) -> Vec2 {
        let d = match self.segment(i) {
            PathSeg::Line(l) => l.p1 - l.p0,
            PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
            PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
        };
        let len = d.hypot();
        if len < 1e-12 {
            return Vec2::ZERO;
        }
        // Con Y hacia abajo, área positiva = sentido horario en pantalla, y
        // el exterior queda a la izquierda del avance: (d.y, -d.x).
        let n = Vec2::new(d.y, -d.x) / len;
        if self.area() >= 0.0 { n } else { -n }
    }

    /// Línea de corte: el contorno desplazado hacia fuera el margen de cada
    /// arista (`allowances[i]`, en mm). Las esquinas se cortan en inglete
    /// (las dos rectas se prolongan hasta cruzarse) salvo si son muy agudas.
    pub fn allowance_outline(&self, allowances: &[f64]) -> Vec<Point> {
        let n = self.len();
        if n < 3 || allowances.len() != n {
            return Vec::new();
        }
        // Puntos desplazados de cada arista, del principio al final.
        let offset: Vec<Vec<Point>> = (0..n)
            .map(|i| {
                let steps = if self.edges[i].is_curve() { 16 } else { 1 };
                let seg = self.segment(i);
                (0..=steps)
                    .map(|k| {
                        let t = k as f64 / steps as f64;
                        seg.eval(t) + self.outward_normal(i, t) * allowances[i]
                    })
                    .collect()
            })
            .collect();
        let mut out = Vec::new();
        for i in 0..n {
            let prev = (i + n - 1) % n;
            let a = &offset[prev];
            let b = &offset[i];
            let (p, q) = (a[a.len() - 1], b[0]);
            let corner = self.vertices[i];
            let reach = 4.0 * allowances[prev].max(allowances[i]).max(1.0);
            let join = line_intersection(p, p - a[a.len() - 2], q, b[1] - q).filter(|x| x.distance(corner) < reach);
            match join {
                Some(x) => out.push(x),
                None => out.extend([p, q]),
            }
            out.extend(&b[1..b.len() - 1]);
        }
        out
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
        self.edges.insert(i + 1, Edge { id: new_edge, kind: second, allowance: self.edges[i].allowance });
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

/// Punto donde se cruzan la recta que pasa por `p` con dirección `dp` y la
/// que pasa por `q` con dirección `dq`, si no son casi paralelas.
fn line_intersection(p: Point, dp: Vec2, q: Point, dq: Vec2) -> Option<Point> {
    let denom = dp.cross(dq);
    if denom.abs() < 1e-9 * dp.hypot() * dq.hypot() {
        return None;
    }
    let s = (q - p).cross(dq) / denom;
    Some(p + dp * s)
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

    #[test]
    fn allowance_outline_of_a_rectangle() {
        let (p, id) = square();
        let piece = p.piece(id).unwrap();
        let size = piece.bounding_box();
        let outline = piece.allowance_outline(&[10.0, 5.0, 10.0, 0.0]);
        assert_eq!(outline.len(), 4);
        let bbox = outline.iter().fold(Rect::from_points(outline[0], outline[0]), |r, p| r.union_pt(*p));
        // Arriba 10, derecha 5, abajo 10, izquierda 0.
        assert!((bbox.y0 - (size.y0 - 10.0)).abs() < 1e-9, "{bbox:?}");
        assert!((bbox.x1 - (size.x1 + 5.0)).abs() < 1e-9, "{bbox:?}");
        assert!((bbox.y1 - (size.y1 + 10.0)).abs() < 1e-9, "{bbox:?}");
        assert!((bbox.x0 - size.x0).abs() < 1e-9, "{bbox:?}");
    }

    #[test]
    fn allowance_outline_follows_curves() {
        let (mut p, id) = square();
        let piece = p.piece_mut(id).unwrap();
        piece.make_curve(0);
        piece.set_control_point(0, 0, Point::new(30.0, -40.0));
        let piece = p.piece(id).unwrap();
        let outline = piece.allowance_outline(&[8.0; 4]);
        // Cada punto queda a unos 8 mm del contorno.
        let path = piece.to_bezpath();
        for q in &outline {
            let d = path.segments().map(|seg| seg.nearest(*q, 1e-6).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
            assert!(d > 7.9 && d < 8.0 * 1.5, "{q:?} a {d} mm");
        }
    }
}
