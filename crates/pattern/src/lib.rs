//! Modelo de datos de un patrón de costura.
//!
//! Un [`Pattern`] es un conjunto de [`Piece`]s (piezas de tela planas) y de
//! [`Seam`]s (costuras) que unen aristas de esas piezas. Una costura puede
//! unir más de dos aristas a la vez (p. ej. tres paneles que se juntan en el
//! fondo de una mochila).
//!
//! Todas las coordenadas están en milímetros, en el plano de la tela, con el
//! eje Y hacia abajo (igual que la pantalla).

mod io;
mod piece;
mod seam;
pub mod textile;

pub use io::{FORMAT_ID, FORMAT_VERSION, PatternFile, PatternIoError};
use kurbo::{ParamCurve, ParamCurveArclen};
pub use kurbo::{Point, Vec2};
pub use piece::{Arrangement, BodyZone, Edge, EdgeKind, InnerLine, Mount, Piece, layer_label, polygon_self_intersects};
pub use seam::{AllowanceSide, DEFAULT_SEAM_ALLOWANCE, DEFAULT_STITCH_LENGTH, SeamFold, SeamKind, Topstitch};
pub use textile::{Textile, TextileId};

use serde::{Deserialize, Serialize};

/// Identificador estable de una pieza dentro de un patrón.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PieceId(pub u64);

/// Identificador estable de una arista. Se mantiene aunque se inserten o
/// borren vértices en otras partes de la pieza, para que las costuras no se
/// rompan al editar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EdgeId(pub u64);

/// Identificador estable de una costura.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeamId(pub u64);

/// Un lado de una costura: una arista concreta de una pieza.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeamSide {
    pub piece: PieceId,
    pub edge: EdgeId,
    /// Si es `true`, la arista se recorre al revés (de su vértice final al
    /// inicial) al emparejarla con los demás lados.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
}

impl SeamSide {
    pub fn new(piece: PieceId, edge: EdgeId, reversed: bool) -> Self {
        Self { piece, edge, reversed }
    }

    /// `true` si es la misma arista (en cualquier sentido).
    pub fn same_edge(&self, other: &SeamSide) -> bool {
        self.piece == other.piece && self.edge == other.edge
    }
}

/// Una costura que une dos o más aristas. Todos los lados se recorren a la
/// vez, del principio al final (según su sentido), y se unen punto a punto.
/// Dos piezas dibujadas vistas desde fuera recorren su borde común en
/// sentidos contrarios, así que lo normal es que uno de los dos lados vaya
/// invertido.
///
/// El orden de las costuras en [`Pattern::seams`] es el orden de montaje.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seam {
    pub id: SeamId,
    #[serde(default)]
    pub name: String,
    pub sides: Vec<SeamSide>,
    #[serde(default, skip_serializing_if = "SeamKind::is_default")]
    pub kind: SeamKind,
    /// Lado (índice en `sides`) hacia el que se plancha el margen. `None` =
    /// abierto, cada margen hacia su pieza (si el tipo lo permite).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed_to: Option<usize>,
    #[serde(default, skip_serializing_if = "Topstitch::is_default")]
    pub topstitch: Topstitch,
    /// Largo de puntada (mm).
    #[serde(default = "default_stitch_length", skip_serializing_if = "is_default_stitch_length")]
    pub stitch_length: f64,
}

fn default_stitch_length() -> f64 {
    DEFAULT_STITCH_LENGTH
}

fn is_default_stitch_length(x: &f64) -> bool {
    *x == DEFAULT_STITCH_LENGTH
}

impl Seam {
    /// Costura simple, con el margen abierto.
    pub fn new(id: SeamId, name: impl Into<String>, sides: Vec<SeamSide>) -> Self {
        Self {
            id,
            name: name.into(),
            sides,
            kind: SeamKind::default(),
            pressed_to: None,
            topstitch: Topstitch::default(),
            stitch_length: DEFAULT_STITCH_LENGTH,
        }
    }

    /// Cambia el tipo de costura y pone los pespuntes y el planchado que
    /// suele llevar.
    pub fn set_kind(&mut self, kind: SeamKind) {
        self.kind = kind;
        self.topstitch = kind.typical_topstitch();
        if kind.pressed_together() && self.pressed_to.is_none() {
            self.pressed_to = Some(0);
        }
    }

    /// Lado hacia el que va el margen, si va a un lado. Un índice que ya no
    /// existe (se ha quitado un lado) cuenta como el primero.
    pub fn pressed_side(&self) -> Option<usize> {
        let side = self.pressed_to.or(self.kind.pressed_together().then_some(0))?;
        Some(if side < self.sides.len() { side } else { 0 })
    }
}

/// Un patrón completo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    #[serde(default)]
    pub pieces: Vec<Piece>,
    #[serde(default)]
    pub seams: Vec<Seam>,
    /// Es una prenda que se viste sobre el maniquí (si no, un objeto suelto
    /// como una bolsa). La vista 3D pone o quita el maniquí según esto.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_avatar: bool,
    /// Presión interior (Pa) en la vista 3D, para objetos con relleno como
    /// un cojín. 0 = vacío, como una bolsa sin nada dentro.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pressure: f64,
    /// Biblioteca de telas que pueden usar las piezas. Empieza con la
    /// genérica ([`Textile::library`]) y solo se guarda si cambia.
    #[serde(default = "Textile::library", skip_serializing_if = "textile::is_generic_library")]
    pub textiles: Vec<Textile>,
    /// Margen de costura (mm) de los bordes que no dicen otro.
    #[serde(default = "default_allowance", skip_serializing_if = "is_default_allowance")]
    pub seam_allowance: f64,
    /// La prenda está del revés (volteada): el derecho de la tela queda
    /// dentro y los márgenes de las costuras, fuera.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inside_out: bool,
    /// Siguiente id libre (compartido por piezas, aristas y costuras).
    #[serde(default)]
    next_id: u64,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

fn default_allowance() -> f64 {
    DEFAULT_SEAM_ALLOWANCE
}

fn is_default_allowance(x: &f64) -> bool {
    *x == DEFAULT_SEAM_ALLOWANCE
}

impl Default for Pattern {
    fn default() -> Self {
        Self::new("Sin título")
    }
}

/// Problemas que [`Pattern::validate`] puede encontrar.
#[derive(Clone, Debug, PartialEq)]
pub enum Issue {
    /// La pieza tiene menos de 3 vértices.
    TooFewVertices { piece: PieceId },
    /// El contorno de la pieza no encierra área.
    DegeneratePiece { piece: PieceId },
    /// El contorno de la pieza se corta a sí mismo.
    SelfIntersecting { piece: PieceId },
    /// Una costura apunta a una pieza o arista que no existe.
    DanglingSeam { seam: SeamId },
    /// Una costura tiene menos de dos lados.
    SeamTooFewSides { seam: SeamId },
    /// Las aristas de la costura tienen longitudes distintas (en mm).
    SeamLengthMismatch { seam: SeamId, min: f64, max: f64 },
    /// Una línea interior se sale de la tela (fuera del contorno, dentro de
    /// un agujero o pegada al borde).
    LineOutside { piece: PieceId, line: EdgeId },
    /// Dos líneas interiores de la pieza se cruzan.
    LinesCross { piece: PieceId, line: EdgeId },
    /// Un agujero no está del todo dentro de su pieza (o su pieza no existe
    /// o es otro agujero), o se cruza con otro agujero.
    HoleOutside { hole: PieceId },
}

/// Distancia mínima (mm) de una línea interior al borde de la tela.
pub const LINE_MARGIN: f64 = 1.0;

/// Diferencia de longitud (mm) a partir de la cual se avisa en una costura.
pub const SEAM_LENGTH_TOLERANCE: f64 = 1.0;

impl Pattern {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pieces: Vec::new(),
            seams: Vec::new(),
            on_avatar: false,
            pressure: 0.0,
            textiles: Textile::library(),
            seam_allowance: DEFAULT_SEAM_ALLOWANCE,
            inside_out: false,
            next_id: 1,
        }
    }

    /// Reserva un id nuevo. Sirve para piezas, aristas y costuras.
    pub fn alloc_id(&mut self) -> u64 {
        // Por si el fichero venía sin `next_id` o con uno desfasado.
        let max_used = self.max_used_id();
        if self.next_id <= max_used {
            self.next_id = max_used + 1;
        }
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn max_used_id(&self) -> u64 {
        let pieces = self.pieces.iter().flat_map(|p| {
            std::iter::once(p.id.0).chain(p.edges().iter().map(|e| e.id.0)).chain(p.lines.iter().map(|l| l.id.0))
        });
        let seams = self.seams.iter().map(|s| s.id.0);
        pieces.chain(seams).max().unwrap_or(0)
    }

    /// Añade una pieza poligonal (todas las aristas rectas).
    pub fn add_polygon(&mut self, name: impl Into<String>, points: Vec<Point>) -> PieceId {
        let id = PieceId(self.alloc_id());
        let edges = points.iter().map(|_| Edge::line(EdgeId(self.alloc_id()))).collect();
        let piece = Piece::from_parts(id, name, points, edges).expect("misma longitud");
        self.pieces.push(piece);
        id
    }

    /// Añade un rectángulo con la esquina superior izquierda en `origin`.
    pub fn add_rect(&mut self, name: impl Into<String>, origin: Point, width: f64, height: f64) -> PieceId {
        let p = origin;
        let points =
            vec![p, Point::new(p.x + width, p.y), Point::new(p.x + width, p.y + height), Point::new(p.x, p.y + height)];
        self.add_polygon(name, points)
    }

    /// Duplica una pieza (con ids nuevos) desplazándola `offset`. Sus
    /// agujeros se duplican con ella.
    pub fn duplicate_piece(&mut self, id: PieceId, offset: Vec2) -> Option<PieceId> {
        self.copy_piece(id, |piece| piece.translate(offset), " (copia)")
    }

    /// Copia en espejo de una pieza (con sus agujeros), a su derecha: la
    /// pieza del otro lado (la manga izquierda a partir de la derecha).
    pub fn mirror_piece(&mut self, id: PieceId) -> Option<PieceId> {
        let bbox = self.piece(id)?.bounding_box();
        let axis = bbox.x1 + 20.0;
        self.copy_piece(id, |piece| piece.mirror_x(axis), " (espejo)")
    }

    /// Copia una pieza y sus agujeros con ids nuevos, cambiando su forma con
    /// `shape`.
    fn copy_piece(&mut self, id: PieceId, shape: impl Fn(&mut Piece), suffix: &str) -> Option<PieceId> {
        let src = self.piece(id)?.clone();
        let holes: Vec<Piece> = self.holes_of(id).cloned().collect();
        let new_id = self.copy_one(&src, &shape, |name| format!("{name}{suffix}"))?;
        if let Some(piece) = self.piece_mut(new_id)
            && src.is_hole()
        {
            piece.hole_in = src.hole_in;
        }
        for hole in holes {
            let copy = self.copy_one(&hole, &shape, |name| name.to_owned())?;
            self.piece_mut(copy)?.hole_in = Some(new_id);
        }
        Some(new_id)
    }

    fn copy_one(&mut self, src: &Piece, shape: &impl Fn(&mut Piece), name: impl Fn(&str) -> String) -> Option<PieceId> {
        let new_id = PieceId(self.alloc_id());
        let edges = src.edges().iter().map(|e| Edge { id: EdgeId(self.alloc_id()), ..*e }).collect();
        let mut piece = Piece::from_parts(new_id, name(&src.name), src.vertices().to_vec(), edges)?;
        piece.arrangement = src.arrangement;
        piece.zone = src.zone;
        piece.fabric = src.fabric;
        piece.layers = src.layers.clone();
        piece.right_side_in = src.right_side_in;
        piece.layer = src.layer;
        piece.on = src.on;
        piece.lines = src.lines.iter().map(|l| InnerLine { id: EdgeId(self.alloc_id()), ..*l }).collect();
        shape(&mut piece);
        self.pieces.push(piece);
        Some(new_id)
    }

    /// Despliega una pieza cortada al doblez: la arista recta `edge_index`
    /// es el doblez y la pieza pasa a ser la entera (ver [`Piece::unfold`]).
    /// Las costuras de la arista del doblez pierden ese lado. Sus agujeros
    /// se reflejan también.
    pub fn unfold_piece(&mut self, id: PieceId, edge_index: usize) -> bool {
        let Some(piece) = self.piece(id) else { return false };
        if edge_index >= piece.len() || piece.edges()[edge_index].is_curve() {
            return false;
        }
        let fold = piece.edges()[edge_index].id;
        let (a, b) = (piece.vertices()[edge_index], piece.vertices()[(edge_index + 1) % piece.len()]);
        let needed = piece.len() - 1 + piece.lines.len();
        let mut ids: Vec<EdgeId> = (0..needed).map(|_| EdgeId(self.alloc_id())).collect::<Vec<_>>();
        let mut iter = ids.drain(..);
        let unfolded = self.piece_mut(id).is_some_and(|p| p.unfold(edge_index, &mut iter));
        drop(iter);
        if !unfolded {
            return false;
        }
        for seam in &mut self.seams {
            seam.sides.retain(|s| !(s.piece == id && s.edge == fold));
        }
        self.seams.retain(|s| s.sides.len() >= 2);
        // Cada agujero, reflejado en el mismo doblez, como un agujero más.
        let holes: Vec<PieceId> = self.holes_of(id).map(|h| h.id).collect();
        for hole in holes {
            let reflect = move |piece: &mut Piece| reflect_piece(piece, a, b);
            self.copy_piece(hole, reflect, "");
        }
        true
    }

    /// Los agujeros de una pieza.
    pub fn holes_of(&self, id: PieceId) -> impl Iterator<Item = &Piece> {
        self.pieces.iter().filter(move |p| p.hole_in == Some(id))
    }

    /// Encima de qué pieza se coloca una pieza en 3D: la que diga la pieza
    /// (`on`) o, si no dice nada, la única a cuyas líneas interiores va
    /// cosida, por su derecho (un bolsillo de parche, un asa). Los agujeros
    /// y las piezas que no van encima de otra devuelven `None`.
    pub fn mount(&self, id: PieceId) -> Option<Mount> {
        let piece = self.piece(id)?;
        if piece.is_hole() {
            return None;
        }
        let host_ok = |host: PieceId| host != id && self.piece(host).is_some_and(|h| !h.is_hole());
        if let Some(mount) = piece.on {
            return host_ok(mount.piece).then_some(mount);
        }
        self.sewn_on_lines(id).map(|piece| Mount { piece, wrong_side: false })
    }

    /// La única pieza a cuyas líneas interiores va cosida esta: encima de
    /// ella va si la pieza no dice otra cosa (ver [`Self::mount`]).
    pub fn sewn_on_lines(&self, id: PieceId) -> Option<PieceId> {
        let host_ok = |host: PieceId| host != id && self.piece(host).is_some_and(|h| !h.is_hole());
        let mut hosts = self
            .seams
            .iter()
            .filter(|seam| seam.sides.iter().any(|s| s.piece == id))
            .flat_map(|seam| &seam.sides)
            .filter(|s| s.piece != id && self.piece(s.piece).is_some_and(|p| p.line_index(s.edge).is_some()))
            .map(|s| s.piece);
        let first = hosts.next()?;
        (host_ok(first) && hosts.all(|h| h == first)).then_some(first)
    }

    /// Añade una línea interior recta a una pieza.
    pub fn add_line(&mut self, piece: PieceId, from: Point, to: Point) -> Option<EdgeId> {
        self.piece(piece)?;
        let id = EdgeId(self.alloc_id());
        self.piece_mut(piece)?.lines.push(InnerLine::line(id, from, to));
        Some(id)
    }

    /// Quita una línea interior y los lados de costura que la usaban.
    pub fn remove_line(&mut self, piece: PieceId, line: EdgeId) -> bool {
        let Some(p) = self.piece_mut(piece) else { return false };
        let before = p.lines.len();
        p.lines.retain(|l| l.id != line);
        if p.lines.len() == before {
            return false;
        }
        for seam in &mut self.seams {
            seam.sides.retain(|s| !(s.piece == piece && s.edge == line));
        }
        self.seams.retain(|s| s.sides.len() >= 2);
        true
    }

    /// Si un punto está en la tela de una pieza: dentro del contorno, fuera
    /// de sus agujeros y a más de `margin` mm de todos sus bordes.
    pub fn in_fabric(&self, piece: &Piece, p: Point, margin: f64) -> bool {
        let clear = |shape: &Piece| shape.nearest_edge(p).is_none_or(|(_, _, d)| d > margin);
        piece.contains(p) && clear(piece) && self.holes_of(piece.id).all(|h| !h.contains(p) && clear(h))
    }

    pub fn piece(&self, id: PieceId) -> Option<&Piece> {
        self.pieces.iter().find(|p| p.id == id)
    }

    pub fn piece_mut(&mut self, id: PieceId) -> Option<&mut Piece> {
        self.pieces.iter_mut().find(|p| p.id == id)
    }

    /// Borra una pieza (y sus agujeros) y quita de las costuras los lados
    /// que la usaban.
    pub fn remove_piece(&mut self, id: PieceId) -> Option<Piece> {
        let idx = self.pieces.iter().position(|p| p.id == id)?;
        let piece = self.pieces.remove(idx);
        let gone: Vec<PieceId> = std::iter::once(id).chain(self.holes_of(id).map(|h| h.id)).collect();
        self.pieces.retain(|p| !gone.contains(&p.id));
        for seam in &mut self.seams {
            seam.sides.retain(|s| !gone.contains(&s.piece));
        }
        self.seams.retain(|s| s.sides.len() >= 2);
        for other in &mut self.pieces {
            if other.on.is_some_and(|m| gone.contains(&m.piece)) {
                other.on = None;
            }
        }
        Some(piece)
    }

    /// Parte la arista `edge_index` de la pieza en el parámetro `t` (0..1),
    /// insertando un vértice nuevo. Devuelve el índice del vértice nuevo.
    pub fn split_edge(&mut self, piece: PieceId, edge_index: usize, t: f64) -> Option<usize> {
        let new_edge = EdgeId(self.alloc_id());
        self.piece_mut(piece)?.split_edge(edge_index, t, new_edge)
    }

    /// Borra un vértice de una pieza (las dos aristas contiguas se funden).
    /// Las costuras que usaban la arista eliminada pierden ese lado.
    pub fn remove_vertex(&mut self, piece: PieceId, vertex_index: usize) -> bool {
        let Some(removed) = self.piece_mut(piece).and_then(|p| p.remove_vertex(vertex_index)) else {
            return false;
        };
        for seam in &mut self.seams {
            seam.sides.retain(|s| !(s.piece == piece && s.edge == removed));
        }
        self.seams.retain(|s| s.sides.len() >= 2);
        true
    }

    /// Crea una costura entre dos o más aristas distintas.
    pub fn add_seam(&mut self, name: impl Into<String>, sides: Vec<SeamSide>) -> Option<SeamId> {
        let repeated = sides.iter().enumerate().any(|(i, a)| sides[..i].iter().any(|b| a.same_edge(b)));
        if sides.len() < 2 || repeated || !sides.iter().all(|s| self.edge_length(s.piece, s.edge).is_some()) {
            return None;
        }
        let id = SeamId(self.alloc_id());
        self.seams.push(Seam::new(id, name, sides));
        Some(id)
    }

    pub fn seam(&self, id: SeamId) -> Option<&Seam> {
        self.seams.iter().find(|s| s.id == id)
    }

    pub fn seam_mut(&mut self, id: SeamId) -> Option<&mut Seam> {
        self.seams.iter_mut().find(|s| s.id == id)
    }

    pub fn remove_seam(&mut self, id: SeamId) -> Option<Seam> {
        let idx = self.seams.iter().position(|s| s.id == id)?;
        Some(self.seams.remove(idx))
    }

    /// Mueve una costura en el orden de montaje (`delta` < 0 = antes).
    pub fn move_seam(&mut self, id: SeamId, delta: isize) {
        let Some(from) = self.seams.iter().position(|s| s.id == id) else { return };
        let to = (from as isize + delta).clamp(0, self.seams.len() as isize - 1) as usize;
        let seam = self.seams.remove(from);
        self.seams.insert(to, seam);
    }

    /// Margen de costura (mm) de una arista: el suyo o el del patrón. Las
    /// líneas interiores no tienen: lo que se cose encima pone el suyo.
    pub fn allowance(&self, piece: PieceId, edge: EdgeId) -> f64 {
        let Some(piece) = self.piece(piece) else { return self.seam_allowance };
        if piece.line_index(edge).is_some() {
            return 0.0;
        }
        piece.edge_index(edge).and_then(|i| piece.edges()[i].allowance).unwrap_or(self.seam_allowance)
    }

    /// Márgenes (mm) de todas las aristas de una pieza, en orden.
    pub fn allowances(&self, piece: &Piece) -> Vec<f64> {
        piece.edges().iter().map(|e| e.allowance.unwrap_or(self.seam_allowance)).collect()
    }

    /// Costuras que usan una arista.
    pub fn seams_of_edge(&self, piece: PieceId, edge: EdgeId) -> impl Iterator<Item = &Seam> {
        self.seams.iter().filter(move |s| s.sides.iter().any(|side| side.piece == piece && side.edge == edge))
    }

    /// Longitud real (mm) de una arista o una línea interior.
    pub fn edge_length(&self, piece: PieceId, edge: EdgeId) -> Option<f64> {
        Some(self.piece(piece)?.edge_segment(edge)?.arclen(1e-3))
    }

    pub fn textile(&self, id: TextileId) -> Option<&Textile> {
        self.textiles.iter().find(|t| t.id == id)
    }

    pub fn textile_mut(&mut self, id: TextileId) -> Option<&mut Textile> {
        self.textiles.iter_mut().find(|t| t.id == id)
    }

    /// Las capas de tela de una pieza, de fuera hacia dentro. Si alguna no
    /// está en la biblioteca (un fichero editado a mano), cuenta como
    /// algodón.
    pub fn piece_textiles(&self, piece: &Piece) -> Vec<Textile> {
        piece.textiles().map(|id| self.textile(id).cloned().unwrap_or_default()).collect()
    }

    /// Cuántas piezas usan una tela (por fuera o como capa).
    pub fn textile_uses(&self, id: TextileId) -> usize {
        self.pieces.iter().filter(|p| p.textiles().any(|t| t == id)).count()
    }

    /// Añade a la biblioteca una copia de `from` con otro nombre, para
    /// ajustarla sin cambiar la original.
    pub fn duplicate_textile(&mut self, from: TextileId) -> Option<TextileId> {
        let mut textile = self.textile(from)?.clone();
        textile.id = TextileId(self.textiles.iter().map(|t| t.id.0).max().unwrap_or(0) + 1);
        textile.name = format!("{} (copia)", textile.name);
        let id = textile.id;
        self.textiles.push(textile);
        Some(id)
    }

    /// Quita una tela de la biblioteca si ninguna pieza la usa.
    pub fn remove_textile(&mut self, id: TextileId) -> bool {
        if self.textile_uses(id) > 0 {
            return false;
        }
        let before = self.textiles.len();
        self.textiles.retain(|t| t.id != id);
        self.textiles.len() < before
    }

    /// Revisa el patrón y devuelve los problemas encontrados.
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        for piece in &self.pieces {
            if piece.vertices().len() < 3 {
                issues.push(Issue::TooFewVertices { piece: piece.id });
                continue;
            }
            if piece.area().abs() < 1e-6 {
                issues.push(Issue::DegeneratePiece { piece: piece.id });
            }
            if piece.is_self_intersecting(0.1) {
                issues.push(Issue::SelfIntersecting { piece: piece.id });
            }
            if piece.is_hole() && !self.hole_fits(piece) {
                issues.push(Issue::HoleOutside { hole: piece.id });
            }
            let samples: Vec<Vec<Point>> = piece.lines.iter().map(line_samples).collect();
            for (k, line) in piece.lines.iter().enumerate() {
                if !samples[k].iter().all(|&p| self.in_fabric(piece, p, LINE_MARGIN)) {
                    issues.push(Issue::LineOutside { piece: piece.id, line: line.id });
                } else if samples[..k].iter().any(|other| polylines_cross(&samples[k], other)) {
                    issues.push(Issue::LinesCross { piece: piece.id, line: line.id });
                }
            }
        }
        for seam in &self.seams {
            if seam.sides.len() < 2 {
                issues.push(Issue::SeamTooFewSides { seam: seam.id });
            }
            let lengths: Option<Vec<f64>> = seam.sides.iter().map(|s| self.edge_length(s.piece, s.edge)).collect();
            match lengths {
                None => issues.push(Issue::DanglingSeam { seam: seam.id }),
                Some(lengths) if !lengths.is_empty() => {
                    let min = lengths.iter().copied().fold(f64::INFINITY, f64::min);
                    let max = lengths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    if max - min > SEAM_LENGTH_TOLERANCE {
                        issues.push(Issue::SeamLengthMismatch { seam: seam.id, min, max });
                    }
                }
                Some(_) => {}
            }
        }
        issues
    }
}

impl Pattern {
    /// Si un agujero cabe en su pieza: la pieza existe y no es otro
    /// agujero, el agujero está dentro sin tocar el borde y no se cruza con
    /// los demás agujeros de la pieza.
    fn hole_fits(&self, hole: &Piece) -> bool {
        let Some(parent) = hole.hole_in.and_then(|id| self.piece(id)) else { return false };
        if parent.is_hole() || parent.id == hole.id {
            return false;
        }
        let outline = hole.flatten(0.5);
        let clear = |shape: &Piece, p: Point| shape.nearest_edge(p).is_none_or(|(_, _, d)| d > LINE_MARGIN);
        let inside = outline.iter().all(|&p| parent.contains(p) && clear(parent, p));
        let others = self.holes_of(parent.id).filter(|h| h.id != hole.id);
        inside && others.into_iter().all(|other| !outline.iter().any(|&p| other.contains(p) || !clear(other, p)))
    }
}

/// Puntos a lo largo de una línea interior, para comprobarla.
fn line_samples(line: &InnerLine) -> Vec<Point> {
    let seg = line.segment();
    (0..=16).map(|k| seg.eval(k as f64 / 16.0)).collect()
}

/// Si dos polilíneas abiertas se cruzan.
fn polylines_cross(a: &[Point], b: &[Point]) -> bool {
    let cross = |o: Point, p: Point, q: Point| (p - o).cross(q - o);
    a.windows(2).any(|s| {
        b.windows(2).any(|t| {
            let (d1, d2) = (cross(t[0], t[1], s[0]), cross(t[0], t[1], s[1]));
            let (d3, d4) = (cross(s[0], s[1], t[0]), cross(s[0], s[1], t[1]));
            (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
        })
    })
}

/// Refleja una pieza en la recta que pasa por `a` y `b` (ver
/// [`Piece::mirror_x`]).
fn reflect_piece(piece: &mut Piece, a: Point, b: Point) {
    // Reflejar en una recta cualquiera es girar hasta ponerla vertical,
    // reflejar en vertical y deshacer el giro.
    let d = b - a;
    let angle = d.y.atan2(d.x) - std::f64::consts::FRAC_PI_2;
    let rotate = |p: Point, angle: f64| {
        let (s, c) = angle.sin_cos();
        let v = p - a;
        a + Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
    };
    piece.transform(|p| rotate(p, -angle), |v| rotate(a + v, -angle) - a);
    piece.mirror_x(a.x);
    piece.transform(|p| rotate(p, angle), |v| rotate(a + v, angle) - a);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        let b = p.add_rect("b", Point::ZERO, 100.0, 50.0);
        assert_ne!(a, b);
        let mut all: Vec<u64> =
            p.pieces.iter().flat_map(|p| std::iter::once(p.id.0).chain(p.edges().iter().map(|e| e.id.0))).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
    }

    #[test]
    fn seam_between_three_pieces() {
        let mut p = Pattern::new("mochila");
        let ids: Vec<_> = (0..3).map(|i| p.add_rect(format!("panel {i}"), Point::ZERO, 200.0, 300.0)).collect();
        let sides = ids
            .iter()
            .map(|&id| SeamSide { piece: id, edge: p.piece(id).unwrap().edges()[2].id, reversed: false })
            .collect();
        let seam = p.add_seam("fondo", sides).unwrap();
        assert!(p.validate().is_empty());

        // Al borrar una pieza la costura se queda con dos lados.
        p.remove_piece(ids[0]);
        assert_eq!(p.seams.iter().find(|s| s.id == seam).unwrap().sides.len(), 2);
        // Y al borrar otra, desaparece.
        p.remove_piece(ids[1]);
        assert!(p.seams.is_empty());
    }

    #[test]
    fn seam_length_mismatch_is_reported() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        let b = p.add_rect("b", Point::ZERO, 120.0, 50.0);
        let ea = p.piece(a).unwrap().edges()[0].id;
        let eb = p.piece(b).unwrap().edges()[0].id;
        p.add_seam(
            "",
            vec![SeamSide { piece: a, edge: ea, reversed: false }, SeamSide { piece: b, edge: eb, reversed: true }],
        );
        assert!(matches!(p.validate()[..], [Issue::SeamLengthMismatch { .. }]));
    }

    #[test]
    fn seam_rejects_repeated_edges() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        let e = p.piece(a).unwrap().edges()[0].id;
        assert!(p.add_seam("", vec![SeamSide::new(a, e, false), SeamSide::new(a, e, true)]).is_none());
        let f = p.piece(a).unwrap().edges()[2].id;
        let id = p.add_seam("", vec![SeamSide::new(a, e, false), SeamSide::new(a, f, true)]).unwrap();
        assert_eq!(p.seams_of_edge(a, f).count(), 1);
        assert!(p.remove_seam(id).is_some());
        assert!(p.seams.is_empty());
    }

    #[test]
    fn textile_library() {
        let mut p = Pattern::new("bolso");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        assert_eq!(p.piece_textiles(p.piece(a).unwrap())[0].name, "Algodón");
        let leather = p.textiles.iter().find(|t| t.name == "Cuero").unwrap().id;
        let linen = p.textiles.iter().find(|t| t.name == "Lino").unwrap().id;
        let piece = p.piece_mut(a).unwrap();
        piece.fabric = leather;
        piece.layers.push(linen);
        let names: Vec<String> = p.piece_textiles(p.piece(a).unwrap()).into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["Cuero", "Lino"]);
        // Lo que está en uso no se borra; lo demás, sí.
        assert!(!p.remove_textile(linen));
        let copy = p.duplicate_textile(linen).unwrap();
        assert_eq!(p.textile(copy).unwrap().name, "Lino (copia)");
        assert!(p.remove_textile(copy));
        // La copia de una pieza lleva las mismas telas.
        let b = p.duplicate_piece(a, Vec2::ZERO).unwrap();
        assert_eq!(p.piece(b).unwrap().layers, [linen]);
        assert_eq!(p.textile_uses(linen), 2);
    }

    #[test]
    fn split_keeps_seam_edge_ids() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        let e2 = p.piece(a).unwrap().edges()[2].id;
        p.split_edge(a, 0, 0.5).unwrap();
        let piece = p.piece(a).unwrap();
        assert_eq!(piece.vertices().len(), 5);
        assert_eq!(piece.edge_index(e2), Some(3));
    }

    #[test]
    fn seam_options_round_trip_and_default_to_plain() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        let b = p.add_rect("b", Point::new(0.0, 50.0), 100.0, 50.0);
        let ea = p.piece(a).unwrap().edges()[2].id;
        let eb = p.piece(b).unwrap().edges()[0].id;
        let id = p.add_seam("", vec![SeamSide::new(a, ea, false), SeamSide::new(b, eb, true)]).unwrap();

        // Sin tocar nada, el fichero no cambia: no aparece ningún campo nuevo.
        let plain = p.to_json().unwrap();
        for field in ["kind", "pressed_to", "topstitch", "stitch_length", "seam_allowance", "inside_out", "allowance"] {
            assert!(!plain.contains(field), "{field} en {plain}");
        }
        assert_eq!(p.allowance(a, ea), DEFAULT_SEAM_ALLOWANCE);

        let seam = p.seam_mut(id).unwrap();
        seam.set_kind(SeamKind::FlatFelled);
        assert_eq!(seam.topstitch, Topstitch::Double);
        assert_eq!(seam.pressed_side(), Some(0));
        p.piece_mut(b).unwrap().right_side_in = true;
        p.inside_out = true;
        p.seam_allowance = 12.0;
        let i = p.piece(a).unwrap().edge_index(ea).unwrap();
        p.piece_mut(a).unwrap().set_edge_allowance(i, Some(15.0));
        assert_eq!(p.allowance(a, ea), 15.0);
        assert_eq!(p.allowance(b, eb), 12.0);

        let back = Pattern::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn split_edge_keeps_its_allowance() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        p.piece_mut(a).unwrap().set_edge_allowance(0, Some(20.0));
        p.split_edge(a, 0, 0.5).unwrap();
        let piece = p.piece(a).unwrap();
        assert_eq!(piece.edges()[1].allowance, Some(20.0));
        assert_eq!(p.allowances(piece), vec![20.0, 20.0, 10.0, 10.0, 10.0]);
    }

    #[test]
    fn seams_can_be_reordered() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        let e: Vec<EdgeId> = p.piece(a).unwrap().edges().iter().map(|e| e.id).collect();
        let s1 = p.add_seam("1", vec![SeamSide::new(a, e[0], false), SeamSide::new(a, e[2], true)]).unwrap();
        let s2 = p.add_seam("2", vec![SeamSide::new(a, e[1], false), SeamSide::new(a, e[3], true)]).unwrap();
        p.move_seam(s2, -1);
        assert_eq!(p.seams.iter().map(|s| s.id).collect::<Vec<_>>(), vec![s2, s1]);
        p.move_seam(s2, -1);
        assert_eq!(p.seams[0].id, s2);
        p.move_seam(s2, 5);
        assert_eq!(p.seams[1].id, s2);
    }

    #[test]
    fn inner_lines_can_be_sewn_and_are_checked() {
        let mut p = Pattern::new("bolso");
        let front = p.add_rect("delantero", Point::ZERO, 300.0, 300.0);
        let pocket = p.add_rect("bolsillo", Point::new(400.0, 0.0), 150.0, 150.0);
        let line = p.add_line(front, Point::new(75.0, 100.0), Point::new(225.0, 100.0)).unwrap();
        assert_eq!(p.edge_length(front, line), Some(150.0));
        assert_eq!(p.allowance(front, line), 0.0);
        let top = p.piece(pocket).unwrap().edges()[0].id;
        let seam = p.add_seam("boca", vec![SeamSide::new(pocket, top, false), SeamSide::new(front, line, false)]);
        assert!(seam.is_some());
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        // Los ids de las líneas no se repiten.
        assert!(p.alloc_id() > line.0);

        // Una línea que se sale de la pieza, o que cruza otra, se avisa.
        let out = p.add_line(front, Point::new(250.0, 50.0), Point::new(350.0, 50.0)).unwrap();
        let cross = p.add_line(front, Point::new(150.0, 50.0), Point::new(150.0, 150.0)).unwrap();
        let issues = p.validate();
        assert!(issues.contains(&Issue::LineOutside { piece: front, line: out }), "{issues:?}");
        assert!(issues.contains(&Issue::LinesCross { piece: front, line: cross }), "{issues:?}");

        // Quitar la línea quita la costura que la usaba.
        assert!(p.remove_line(front, line));
        assert!(p.seams.is_empty());
        // La copia de una pieza lleva sus líneas, con ids nuevos.
        let copy = p.duplicate_piece(front, Vec2::new(0.0, 400.0)).unwrap();
        let lines = &p.piece(copy).unwrap().lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].from, Point::new(250.0, 450.0));
        assert!(lines.iter().all(|l| l.id != out && l.id != cross));
    }

    #[test]
    fn pieces_sewn_on_lines_go_on_top() {
        let mut p = Pattern::new("bolso");
        let front = p.add_rect("delantero", Point::ZERO, 300.0, 300.0);
        let lining = p.add_rect("forro", Point::new(400.0, 0.0), 300.0, 300.0);
        let pocket = p.add_rect("bolsillo", Point::new(0.0, 400.0), 150.0, 150.0);
        let piece = p.piece_mut(lining).unwrap();
        piece.layer = 1;
        piece.right_side_in = true;
        assert_eq!(p.mount(pocket), None);

        // Cosido a una línea del delantero, va encima, por el derecho.
        let line = p.add_line(front, Point::new(75.0, 100.0), Point::new(225.0, 100.0)).unwrap();
        let top = p.piece(pocket).unwrap().edges()[0].id;
        p.add_seam("boca", vec![SeamSide::new(pocket, top, false), SeamSide::new(front, line, false)]);
        assert_eq!(p.mount(pocket), Some(Mount { piece: front, wrong_side: false }));
        // El delantero no va encima del bolsillo, ni el forro encima de nada.
        assert_eq!(p.mount(front), None);
        assert_eq!(p.mount(lining), None);

        // Dicho a mano, manda lo que diga la pieza.
        let on_lining = Mount { piece: lining, wrong_side: true };
        p.piece_mut(pocket).unwrap().on = Some(on_lining);
        assert_eq!(p.mount(pocket), Some(on_lining));
        // Se guarda y se lee igual.
        let back = Pattern::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back.piece(pocket).unwrap().on, Some(on_lining));
        assert_eq!(back.piece(lining).unwrap().layer, 1);
        // Sin el forro, vuelve a ir sobre la línea del delantero.
        p.remove_piece(lining);
        assert_eq!(p.piece(pocket).unwrap().on, None);
        assert_eq!(p.mount(pocket).map(|m| m.piece), Some(front));
    }

    #[test]
    fn holes_live_in_their_piece() {
        let mut p = Pattern::new("t");
        let panel = p.add_rect("panel", Point::ZERO, 300.0, 200.0);
        let window = p.add_rect("ventana", Point::new(50.0, 80.0), 200.0, 20.0);
        p.piece_mut(window).unwrap().hole_in = Some(panel);
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        assert_eq!(p.holes_of(panel).count(), 1);
        assert!(!p.in_fabric(p.piece(panel).unwrap(), Point::new(100.0, 90.0), 0.0));
        assert!(p.in_fabric(p.piece(panel).unwrap(), Point::new(100.0, 150.0), 0.0));

        // Fuera de su pieza, se avisa.
        p.piece_mut(window).unwrap().translate(Vec2::new(200.0, 0.0));
        assert_eq!(p.validate(), [Issue::HoleOutside { hole: window }]);
        p.piece_mut(window).unwrap().translate(Vec2::new(-200.0, 0.0));

        // Duplicar la pieza duplica el agujero; borrarla, lo borra.
        let copy = p.duplicate_piece(panel, Vec2::new(0.0, 300.0)).unwrap();
        assert_eq!(p.holes_of(copy).count(), 1);
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        p.remove_piece(panel);
        assert_eq!(p.pieces.len(), 2);
        assert!(p.piece(window).is_none());
    }

    #[test]
    fn mirrored_copy_keeps_its_shape() {
        let mut p = Pattern::new("t");
        let sleeve = p.add_polygon(
            "manga",
            vec![Point::new(0.0, 0.0), Point::new(100.0, 20.0), Point::new(120.0, 200.0), Point::new(0.0, 200.0)],
        );
        p.piece_mut(sleeve).unwrap().make_curve(0);
        p.piece_mut(sleeve).unwrap().set_control_point(0, 0, Point::new(40.0, -30.0));
        p.add_line(sleeve, Point::new(20.0, 100.0), Point::new(60.0, 120.0));
        let original = p.piece(sleeve).unwrap().clone();
        let copy = p.mirror_piece(sleeve).unwrap();
        let mirror = p.piece(copy).unwrap();
        assert!((mirror.area() - original.area()).abs() < 1e-6, "mismo sentido y área");
        assert!((mirror.perimeter() - original.perimeter()).abs() < 1e-6);
        let (a, b) = (original.bounding_box(), mirror.bounding_box());
        assert!((a.width() - b.width()).abs() < 1e-6 && b.x0 > a.x1);
        // El punto más a la derecha pasa a ser el más a la izquierda.
        let leftmost = mirror.vertices().iter().map(|v| v.x).fold(f64::INFINITY, f64::min);
        assert!((leftmost - b.x0).abs() < 1e-9);
        assert_eq!(mirror.lines.len(), 1);
        assert!(p.validate().is_empty(), "{:?}", p.validate());
    }

    #[test]
    fn unfold_on_the_fold() {
        let mut p = Pattern::new("t");
        // Media espalda: el doblez es la arista de la izquierda (x = 0).
        let half = p.add_polygon(
            "espalda",
            vec![Point::new(0.0, 0.0), Point::new(150.0, 30.0), Point::new(180.0, 400.0), Point::new(0.0, 400.0)],
        );
        let b = p.add_rect("otra", Point::new(300.0, 0.0), 100.0, 100.0);
        let fold = p.piece(half).unwrap().edges()[3].id;
        let side = p.piece(half).unwrap().edges()[1].id;
        let eb = p.piece(b).unwrap().edges()[3].id;
        p.add_seam("doblez", vec![SeamSide::new(half, fold, false), SeamSide::new(b, eb, true)]);
        p.add_seam("costado", vec![SeamSide::new(half, side, false), SeamSide::new(b, eb, true)]);
        let area = p.piece(half).unwrap().area();
        assert!(!p.unfold_piece(half, 7));
        assert!(p.unfold_piece(half, 3));
        let whole = p.piece(half).unwrap();
        assert_eq!(whole.len(), 6);
        assert!((whole.area() - 2.0 * area).abs() < 1e-6, "{} vs {}", whole.area(), 2.0 * area);
        assert!(whole.edge_index(fold).is_none());
        assert!(whole.edge_index(side).is_some());
        let bbox = whole.bounding_box();
        assert!((bbox.x0 + 180.0).abs() < 1e-9 && (bbox.x1 - 180.0).abs() < 1e-9, "{bbox:?}");
        assert!(!whole.is_self_intersecting(0.1));
        // La costura del doblez desaparece; la otra sigue.
        assert_eq!(p.seams.len(), 1);
        assert_eq!(p.seams[0].name, "costado");
    }
}
