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
pub mod textile;
mod seam;

pub use io::{FORMAT_ID, FORMAT_VERSION, PatternFile, PatternIoError};
pub use kurbo::{Point, Vec2};
pub use piece::{Arrangement, BodyZone, Edge, EdgeKind, Piece, polygon_self_intersects};
pub use textile::{Textile, TextileId};
pub use seam::{AllowanceSide, DEFAULT_SEAM_ALLOWANCE, DEFAULT_STITCH_LENGTH, SeamFold, SeamKind, Topstitch};

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
}

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
        let pieces = self.pieces.iter().flat_map(|p| std::iter::once(p.id.0).chain(p.edges().iter().map(|e| e.id.0)));
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

    /// Duplica una pieza (con ids nuevos) desplazándola `offset`.
    pub fn duplicate_piece(&mut self, id: PieceId, offset: Vec2) -> Option<PieceId> {
        let src = self.piece(id)?.clone();
        let new_id = PieceId(self.alloc_id());
        let edges = src.edges().iter().map(|e| Edge { id: EdgeId(self.alloc_id()), ..*e }).collect();
        let points = src.vertices().iter().map(|p| *p + offset).collect();
        let mut piece =
            Piece::from_parts(new_id, format!("{} (copia)", src.name), points, edges).expect("misma longitud");
        piece.arrangement = src.arrangement;
        piece.zone = src.zone;
        piece.fabric = src.fabric;
        piece.layers = src.layers.clone();
        piece.right_side_in = src.right_side_in;
        self.pieces.push(piece);
        Some(new_id)
    }

    pub fn piece(&self, id: PieceId) -> Option<&Piece> {
        self.pieces.iter().find(|p| p.id == id)
    }

    pub fn piece_mut(&mut self, id: PieceId) -> Option<&mut Piece> {
        self.pieces.iter_mut().find(|p| p.id == id)
    }

    /// Borra una pieza y quita de las costuras los lados que la usaban.
    pub fn remove_piece(&mut self, id: PieceId) -> Option<Piece> {
        let idx = self.pieces.iter().position(|p| p.id == id)?;
        let piece = self.pieces.remove(idx);
        for seam in &mut self.seams {
            seam.sides.retain(|s| s.piece != id);
        }
        self.seams.retain(|s| s.sides.len() >= 2);
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

    /// Margen de costura (mm) de una arista: el suyo o el del patrón.
    pub fn allowance(&self, piece: PieceId, edge: EdgeId) -> f64 {
        let piece = self.piece(piece);
        piece.and_then(|p| p.edges()[p.edge_index(edge)?].allowance).unwrap_or(self.seam_allowance)
    }

    /// Márgenes (mm) de todas las aristas de una pieza, en orden.
    pub fn allowances(&self, piece: &Piece) -> Vec<f64> {
        piece.edges().iter().map(|e| e.allowance.unwrap_or(self.seam_allowance)).collect()
    }

    /// Costuras que usan una arista.
    pub fn seams_of_edge(&self, piece: PieceId, edge: EdgeId) -> impl Iterator<Item = &Seam> {
        self.seams.iter().filter(move |s| s.sides.iter().any(|side| side.piece == piece && side.edge == edge))
    }

    /// Longitud real (mm) de una arista.
    pub fn edge_length(&self, piece: PieceId, edge: EdgeId) -> Option<f64> {
        let piece = self.piece(piece)?;
        let idx = piece.edge_index(edge)?;
        Some(piece.edge_length(idx))
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
}
