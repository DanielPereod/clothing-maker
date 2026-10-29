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

pub use io::{FORMAT_ID, FORMAT_VERSION, PatternFile, PatternIoError};
pub use kurbo::{Point, Vec2};
pub use piece::{Edge, EdgeKind, Piece, polygon_self_intersects};

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
    #[serde(default)]
    pub reversed: bool,
}

/// Una costura que une dos o más aristas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seam {
    pub id: SeamId,
    #[serde(default)]
    pub name: String,
    pub sides: Vec<SeamSide>,
}

/// Un patrón completo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    #[serde(default)]
    pub pieces: Vec<Piece>,
    #[serde(default)]
    pub seams: Vec<Seam>,
    /// Siguiente id libre (compartido por piezas, aristas y costuras).
    #[serde(default)]
    next_id: u64,
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
        Self { name: name.into(), pieces: Vec::new(), seams: Vec::new(), next_id: 1 }
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
        let edges = src.edges().iter().map(|e| Edge { id: EdgeId(self.alloc_id()), kind: e.kind }).collect();
        let points = src.vertices().iter().map(|p| *p + offset).collect();
        let piece = Piece::from_parts(new_id, format!("{} (copia)", src.name), points, edges).expect("misma longitud");
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

    /// Crea una costura entre dos o más aristas.
    pub fn add_seam(&mut self, name: impl Into<String>, sides: Vec<SeamSide>) -> Option<SeamId> {
        if sides.len() < 2 || !sides.iter().all(|s| self.edge_length(s.piece, s.edge).is_some()) {
            return None;
        }
        let id = SeamId(self.alloc_id());
        self.seams.push(Seam { id, name: name.into(), sides });
        Some(id)
    }

    /// Longitud real (mm) de una arista.
    pub fn edge_length(&self, piece: PieceId, edge: EdgeId) -> Option<f64> {
        let piece = self.piece(piece)?;
        let idx = piece.edge_index(edge)?;
        Some(piece.edge_length(idx))
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
    fn split_keeps_seam_edge_ids() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 100.0, 50.0);
        let e2 = p.piece(a).unwrap().edges()[2].id;
        p.split_edge(a, 0, 0.5).unwrap();
        let piece = p.piece(a).unwrap();
        assert_eq!(piece.vertices().len(), 5);
        assert_eq!(piece.edge_index(e2), Some(3));
    }
}
