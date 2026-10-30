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
mod notions;
mod piece;
mod seam;
pub mod textile;

pub use io::{FORMAT_ID, FORMAT_VERSION, PatternFile, PatternIoError};
use kurbo::{ParamCurve, ParamCurveArclen};
pub use kurbo::{Point, Vec2};
pub use notions::{ELASTIC_RATIO, Notion, NotionParts, ZIPPER_TAPE_WIDTH};
pub use piece::{
    Arrangement, BodyZone, Edge, EdgeKind, FOLD_SNAP, Fold, InnerLine, Mark, Mount, Notch, Piece, layer_label,
    polygon_self_intersects,
};
pub use seam::{AllowanceSide, DEFAULT_SEAM_ALLOWANCE, DEFAULT_STITCH_LENGTH, SeamFit, SeamFold, SeamKind, Topstitch};
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

/// Identificador estable de una etapa de montaje.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StepId(pub u64);

/// Identificador estable de un punto marcado en una pieza.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MarkId(pub u64);

/// Identificador estable de un broche o una presilla.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FastenerId(pub u64);

/// Qué une dos puntos de la prenda.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FastenerKind {
    /// Un broche de presión: se abre y se cierra.
    #[default]
    Snap,
    /// Una presilla: unas puntadas apretadas que fijan dos capas en un
    /// punto (los extremos de un asa, el forro al fondo del exterior).
    BarTack,
}

impl FastenerKind {
    pub const ALL: [FastenerKind; 2] = [FastenerKind::Snap, FastenerKind::BarTack];

    pub fn label(self) -> &'static str {
        match self {
            FastenerKind::Snap => "Broche",
            FastenerKind::BarTack => "Presilla",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            FastenerKind::Snap => "Un broche de presión entre dos puntos: se abre y se cierra (una solapa, un cierre).",
            FastenerKind::BarTack => {
                "Puntadas que fijan dos puntos para siempre: el forro al exterior, el extremo de un asa, una \
                 trabilla."
            }
        }
    }

    fn is_default(&self) -> bool {
        *self == FastenerKind::default()
    }
}

/// Un extremo de un broche o una presilla: un punto marcado de una pieza.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkRef {
    pub piece: PieceId,
    pub mark: MarkId,
}

/// Un broche o una presilla: une dos puntos marcados (de dos piezas o de
/// la misma) en uno solo al montar la prenda.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fastener {
    pub id: FastenerId,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "FastenerKind::is_default")]
    pub kind: FastenerKind,
    pub ends: [MarkRef; 2],
    /// Un broche abierto no une nada. Una presilla siempre une.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
}

impl Fastener {
    /// Si une sus dos puntos ahora.
    pub fn joins(&self) -> bool {
        self.kind == FastenerKind::BarTack || !self.open
    }
}

/// Un lado de una costura: una arista de una pieza (o una línea interior),
/// varias seguidas del contorno, o un tramo de ellas.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeamSide {
    pub piece: PieceId,
    pub edge: EdgeId,
    /// Última arista si el lado sigue el contorno por varias (una base
    /// redonda hecha de cuatro curvas): desde `edge` hasta esta, las dos
    /// incluidas, en el orden de la pieza.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<EdgeId>,
    /// Tramo que se cose: desde y hasta, en fracciones (0..1) del largo del
    /// lado y en el orden de la pieza. Sin tramo, se cose entero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Si es `true`, la arista se recorre al revés (de su vértice final al
    /// inicial) al emparejarla con los demás lados.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
}

impl SeamSide {
    pub fn new(piece: PieceId, edge: EdgeId, reversed: bool) -> Self {
        Self { piece, edge, end: None, range: None, reversed }
    }

    /// El mismo lado, siguiendo el contorno hasta la arista `end`.
    pub fn through(self, end: EdgeId) -> Self {
        Self { end: (end != self.edge).then_some(end), ..self }
    }

    /// El mismo lado, cosido solo del tramo `from..to` (fracciones de su
    /// largo, en el orden de la pieza).
    pub fn span(self, from: f64, to: f64) -> Self {
        Self { range: Some([from, to]), ..self }
    }

    /// El tramo que se cose, ordenado y dentro de 0..1.
    pub fn sewn_range(&self) -> [f64; 2] {
        let [a, b] = self.range.unwrap_or([0.0, 1.0]);
        let (a, b) = (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
        [a.min(b), a.max(b)]
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
    /// Cómo casan los lados si no miden lo mismo (fruncida, estirada...).
    #[serde(default, skip_serializing_if = "SeamFit::is_default")]
    pub fit: SeamFit,
    /// Etapa de montaje en la que se cose. Sin etapa, va con las primeras.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<StepId>,
    /// En una cremallera, la parte abierta (0..1 de su largo) desde el
    /// principio de la costura, donde queda el tirador: 0 = cerrada.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub open: f64,
}

/// Una etapa de montaje: costuras que se cosen a la vez («Exterior»,
/// «Forro», «Unión en la boca»). Las etapas se cosen en orden.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssemblyStep {
    pub id: StepId,
    pub name: String,
}

/// Un tramo de arista que recorre un lado de costura.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideSpan {
    pub edge: EdgeId,
    /// Desde y hasta, en fracciones (0..1) del largo de la arista.
    pub from: f64,
    pub to: f64,
    /// Largo (mm) del tramo.
    pub length: f64,
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
            fit: SeamFit::default(),
            step: None,
            open: 0.0,
        }
    }

    /// Si el punto a `t` (0..1) de su largo está abierto: en una cremallera,
    /// lo que queda entre el principio y el tirador.
    pub fn is_open_at(&self, t: f64) -> bool {
        self.kind == SeamKind::Zipper && self.open > 0.0 && t <= self.open + 1e-9
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
    /// Etapas de montaje, en el orden en que se cosen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<AssemblyStep>,
    /// Broches y presillas: se cierran con la última etapa.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fasteners: Vec<Fastener>,
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
    /// Las aristas de la costura tienen longitudes distintas (en mm) y no
    /// está marcada como fruncida, embebida o estirada.
    SeamLengthMismatch { seam: SeamId, min: f64, max: f64 },
    /// Costura embebida con demasiada diferencia (en mm): no se puede
    /// embeber sin pliegues.
    TooMuchEase { seam: SeamId, min: f64, max: f64 },
    /// Los lados de la costura no tienen los mismos piquetes: no se usan.
    NotchesMismatch { seam: SeamId },
    /// Una línea interior se sale de la tela (fuera del contorno, dentro de
    /// un agujero o pegada al borde).
    LineOutside { piece: PieceId, line: EdgeId },
    /// Dos líneas interiores de la pieza se cruzan.
    LinesCross { piece: PieceId, line: EdgeId },
    /// Un doblez no va de un vértice del contorno a otro por dentro de la
    /// tela, así que no parte la pieza en dos.
    FoldNotAcross { piece: PieceId, line: EdgeId },
    /// Un agujero no está del todo dentro de su pieza (o su pieza no existe
    /// o es otro agujero), o se cruza con otro agujero.
    HoleOutside { hole: PieceId },
    /// Un extremo de un broche o una presilla no está en la tela de su
    /// pieza (o ya no existe).
    FastenerOutside { fastener: FastenerId },
}

/// Distancia mínima (mm) de una línea interior al borde de la tela.
pub const LINE_MARGIN: f64 = 1.0;

/// Diferencia de longitud (mm) a partir de la cual se avisa en una costura.
pub const SEAM_LENGTH_TOLERANCE: f64 = 1.0;

/// Lo más que puede medir el lado largo de una costura embebida, respecto
/// al corto.
pub const MAX_EASE: f64 = 1.1;

impl Pattern {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pieces: Vec::new(),
            seams: Vec::new(),
            steps: Vec::new(),
            fasteners: Vec::new(),
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
            std::iter::once(p.id.0)
                .chain(p.edges().iter().map(|e| e.id.0))
                .chain(p.lines.iter().map(|l| l.id.0))
                .chain(p.marks.iter().map(|m| m.id.0))
        });
        let seams = self.seams.iter().map(|s| s.id.0);
        let steps = self.steps.iter().map(|s| s.id.0);
        let fasteners = self.fasteners.iter().map(|f| f.id.0);
        pieces.chain(seams).chain(steps).chain(fasteners).max().unwrap_or(0)
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
        let edges: Vec<Edge> = src.edges().iter().map(|e| Edge { id: EdgeId(self.alloc_id()), ..*e }).collect();
        let renamed = |id: EdgeId| src.edge_index(id).map(|i| edges[i].id);
        let notches =
            src.notches.iter().filter_map(|n| Some(piece::Notch { edge: renamed(n.edge)?, t: n.t })).collect();
        let mut piece = Piece::from_parts(new_id, name(&src.name), src.vertices().to_vec(), edges)?;
        piece.notches = notches;
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
            seam.sides.retain(|s| !(s.piece == id && (s.edge == fold || s.end == Some(fold))));
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

    /// Añade un doblez recto a una pieza, de un punto de su contorno a otro
    /// (los más cercanos a `from` y `to`, o el vértice que haya a menos de
    /// 1 mm): parte las aristas por ahí si hace falta. La solapa es la parte
    /// más pequeña. `None` (y el patrón sin tocar) si los dos puntos caen en
    /// la misma arista o en vértices contiguos.
    pub fn add_fold(&mut self, piece: PieceId, from: Point, to: Point) -> Option<EdgeId> {
        let mut draft = self.clone();
        let id = draft.add_fold_in_place(piece, from, to)?;
        *self = draft;
        Some(id)
    }

    fn add_fold_in_place(&mut self, piece: PieceId, from: Point, to: Point) -> Option<EdgeId> {
        let (ia, ..) = self.piece(piece)?.nearest_edge(from)?;
        let (ib, ..) = self.piece(piece)?.nearest_edge(to)?;
        if ia == ib {
            return None;
        }
        let a = self.outline_point(piece, from)?;
        let b = self.outline_point(piece, to)?;
        let p = self.piece(piece)?;
        let n = p.len();
        let index = |q: Point| p.vertices().iter().position(|v| v.distance(q) < FOLD_SNAP);
        let (ia, ib) = (index(a)?, index(b)?);
        if ia == ib || (ia + 1) % n == ib || (ib + 1) % n == ia {
            return None;
        }
        // Las dos partes del contorno, de a a b y de b a a.
        let part = |from: usize, to: usize| -> Vec<Point> {
            let count = (to + n - from) % n + 1;
            (0..count).map(|k| p.vertices()[(from + k) % n]).collect()
        };
        let area = |ring: &[Point]| {
            (0..ring.len()).map(|k| ring[k].to_vec2().cross(ring[(k + 1) % ring.len()].to_vec2())).sum::<f64>().abs()
        };
        let (one, other) = (part(ia, ib), part(ib, ia));
        let small = if area(&one) <= area(&other) { one } else { other };
        // El punto de la parte pequeña más lejos de la línea.
        let off_line = |q: &Point| (b - a).cross(*q - a).abs();
        let probe = *small[1..small.len() - 1].iter().max_by(|x, y| off_line(x).total_cmp(&off_line(y)))?;
        let id = EdgeId(self.alloc_id());
        let mut line = InnerLine { fold: Some(Fold::default()), ..InnerLine::line(id, a, b) };
        if !line.in_flap(probe) {
            line.fold = Some(Fold { flip: true, ..Fold::default() });
        }
        self.piece_mut(piece)?.lines.push(line);
        Some(id)
    }

    /// El punto del contorno de una pieza más cercano a `p`: un vértice si
    /// hay uno a menos de 1 mm, o uno nuevo partiendo la arista.
    fn outline_point(&mut self, piece: PieceId, p: Point) -> Option<Point> {
        const SNAP: f64 = 1.0;
        let shape = self.piece(piece)?;
        let (i, t, _) = shape.nearest_edge(p)?;
        let at = shape.segment(i).eval(t);
        let (a, b) = shape.edge_endpoints(i);
        for v in [shape.vertices()[a], shape.vertices()[b]] {
            if v.distance(at) < SNAP {
                return Some(v);
            }
        }
        let v = self.split_edge(piece, i, t)?;
        Some(self.piece(piece)?.vertices()[v])
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
        self.fasteners.retain(|f| f.ends.iter().all(|e| !gone.contains(&e.piece)));
        Some(piece)
    }

    /// Un broche o una presilla entre dos puntos (de la misma pieza o de
    /// dos): marca los puntos en sus piezas. `None` si no caen en la tela.
    pub fn add_fastener(&mut self, kind: FastenerKind, a: (PieceId, Point), b: (PieceId, Point)) -> Option<FastenerId> {
        for (piece, at) in [a, b] {
            let p = self.piece(piece)?;
            if p.is_hole() || !self.in_fabric(p, at, 0.0) {
                return None;
            }
        }
        let ends = [a, b].map(|(piece, at)| {
            let mark = MarkId(self.alloc_id());
            if let Some(p) = self.piece_mut(piece) {
                p.marks.push(Mark { id: mark, at });
            }
            MarkRef { piece, mark }
        });
        let id = FastenerId(self.alloc_id());
        let name = format!("{} {}", kind.label(), self.fasteners.len() + 1);
        self.fasteners.push(Fastener { id, name, kind, ends, open: false });
        Some(id)
    }

    pub fn fastener(&self, id: FastenerId) -> Option<&Fastener> {
        self.fasteners.iter().find(|f| f.id == id)
    }

    pub fn fastener_mut(&mut self, id: FastenerId) -> Option<&mut Fastener> {
        self.fasteners.iter_mut().find(|f| f.id == id)
    }

    /// Quita un broche o una presilla y sus puntos marcados.
    pub fn remove_fastener(&mut self, id: FastenerId) -> Option<Fastener> {
        let idx = self.fasteners.iter().position(|f| f.id == id)?;
        let fastener = self.fasteners.remove(idx);
        for end in fastener.ends {
            if let Some(p) = self.piece_mut(end.piece) {
                p.marks.retain(|m| m.id != end.mark);
            }
        }
        Some(fastener)
    }

    /// Dónde está un extremo de un broche o una presilla.
    pub fn mark_point(&self, end: MarkRef) -> Option<Point> {
        self.piece(end.piece)?.marks.iter().find(|m| m.id == end.mark).map(|m| m.at)
    }

    /// Parte la arista `edge_index` de la pieza en el parámetro `t` (0..1),
    /// insertando un vértice nuevo. Devuelve el índice del vértice nuevo.
    pub fn split_edge(&mut self, piece: PieceId, edge_index: usize, t: f64) -> Option<usize> {
        let new_edge = EdgeId(self.alloc_id());
        let old_edge = self.piece(piece)?.edges().get(edge_index)?.id;
        let vertex = self.piece_mut(piece)?.split_edge(edge_index, t, new_edge)?;
        // Los lados que acababan en la arista partida siguen hasta el final
        // de la segunda mitad: cosen lo mismo que antes.
        for side in self.seams.iter_mut().flat_map(|s| &mut s.sides) {
            if side.piece == piece && side.end.unwrap_or(side.edge) == old_edge {
                side.end = Some(new_edge);
            }
        }
        Some(vertex)
    }

    /// Borra un vértice de una pieza (las dos aristas contiguas se funden).
    /// Las costuras que usaban la arista eliminada pierden ese lado.
    pub fn remove_vertex(&mut self, piece: PieceId, vertex_index: usize) -> bool {
        let Some(removed) = self.piece_mut(piece).and_then(|p| p.remove_vertex(vertex_index)) else {
            return false;
        };
        // La arista de antes del vértice se queda con el tramo de la borrada.
        let merged = self.piece(piece).map(|p| {
            let n = p.len();
            p.edges()[(vertex_index + n - 1) % n].id
        });
        for seam in &mut self.seams {
            seam.sides.retain(|s| !(s.piece == piece && s.edge == removed));
            for side in seam.sides.iter_mut().filter(|s| s.piece == piece && s.end == Some(removed)) {
                side.end = merged.filter(|&m| m != side.edge);
            }
        }
        self.seams.retain(|s| s.sides.len() >= 2);
        true
    }

    /// Crea una costura entre dos o más aristas distintas.
    pub fn add_seam(&mut self, name: impl Into<String>, sides: Vec<SeamSide>) -> Option<SeamId> {
        let repeated = sides.iter().enumerate().any(|(i, a)| sides[..i].iter().any(|b| a.same_edge(b)));
        if sides.len() < 2 || repeated || !sides.iter().all(|s| self.side_length(s).is_some()) {
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

    /// Costuras que usan una arista (también como parte de una cadena).
    pub fn seams_of_edge(&self, piece: PieceId, edge: EdgeId) -> impl Iterator<Item = &Seam> {
        self.seams.iter().filter(move |s| s.sides.iter().any(|side| self.side_covers(side, piece, edge)))
    }

    /// Aristas que recorre un lado de costura, en el orden de la pieza, o
    /// `None` si alguna no existe. Una línea interior va sola.
    pub fn side_edges(&self, side: &SeamSide) -> Option<Vec<EdgeId>> {
        let piece = self.piece(side.piece)?;
        let Some(start) = piece.edge_index(side.edge) else {
            let line = piece.line_index(side.edge).is_some() && side.end.is_none_or(|e| e == side.edge);
            return line.then(|| vec![side.edge]);
        };
        let n = piece.len();
        let mut out = Vec::new();
        for k in 0..n {
            let id = piece.edges()[(start + k) % n].id;
            out.push(id);
            if side.end.is_none_or(|e| e == id) {
                return Some(out);
            }
        }
        None
    }

    /// Tramos de arista que se cosen en un lado, en el orden de la pieza, o
    /// `None` si el lado no existe o no tiene largo.
    pub fn side_spans(&self, side: &SeamSide) -> Option<Vec<SideSpan>> {
        let piece = self.piece(side.piece)?;
        let edges = self.side_edges(side)?;
        let lengths: Vec<f64> =
            edges.iter().map(|&e| piece.edge_segment(e).map(|s| s.arclen(1e-3))).collect::<Option<_>>()?;
        let total: f64 = lengths.iter().sum();
        let [from, to] = side.sewn_range();
        let (from, to) = (from * total, to * total);
        let mut out = Vec::new();
        let mut start = 0.0;
        for (&edge, &length) in edges.iter().zip(&lengths) {
            let (a, b) = (from.max(start), to.min(start + length));
            if b - a > 1e-9 && length > 0.0 {
                out.push(SideSpan { edge, from: (a - start) / length, to: (b - start) / length, length: b - a });
            }
            start += length;
        }
        (!out.is_empty()).then_some(out)
    }

    /// Largo (mm) que se cose de un lado.
    pub fn side_length(&self, side: &SeamSide) -> Option<f64> {
        Some(self.side_spans(side)?.iter().map(|s| s.length).sum())
    }

    /// Si un lado de costura pasa por una arista de una pieza.
    pub fn side_covers(&self, side: &SeamSide, piece: PieceId, edge: EdgeId) -> bool {
        side.piece == piece && (side.edge == edge || self.side_edges(side).is_some_and(|e| e.contains(&edge)))
    }

    /// Dónde caen los piquetes de la pieza en el tramo cosido de un lado:
    /// fracciones (0..1) de su largo, en el sentido de la costura (al revés
    /// si el lado va invertido) y de menor a mayor.
    pub fn side_notches(&self, side: &SeamSide) -> Vec<f64> {
        let (Some(piece), Some(spans)) = (self.piece(side.piece), self.side_spans(side)) else { return Vec::new() };
        let total: f64 = spans.iter().map(|s| s.length).sum();
        let mut out = Vec::new();
        let mut start = 0.0;
        for span in &spans {
            let Some(seg) = piece.edge_segment(span.edge) else { continue };
            let length = seg.arclen(1e-3);
            for notch in piece.notches.iter().filter(|n| n.edge == span.edge) {
                let at = if length > 0.0 { seg.subsegment(0.0..notch.t).arclen(1e-3) / length } else { 0.0 };
                let pos = (start + (at - span.from) * length) / total;
                if at > span.from && at < span.to && pos > 1e-6 && pos < 1.0 - 1e-6 {
                    out.push(if side.reversed { 1.0 - pos } else { pos });
                }
            }
            start += span.length;
        }
        out.sort_by(f64::total_cmp);
        out
    }

    /// Añade una etapa de montaje al final.
    pub fn add_step(&mut self, name: impl Into<String>) -> StepId {
        let id = StepId(self.alloc_id());
        self.steps.push(AssemblyStep { id, name: name.into() });
        id
    }

    pub fn step(&self, id: StepId) -> Option<&AssemblyStep> {
        self.steps.iter().find(|s| s.id == id)
    }

    pub fn step_mut(&mut self, id: StepId) -> Option<&mut AssemblyStep> {
        self.steps.iter_mut().find(|s| s.id == id)
    }

    /// Borra una etapa: sus costuras se quedan sin etapa.
    pub fn remove_step(&mut self, id: StepId) -> bool {
        let before = self.steps.len();
        self.steps.retain(|s| s.id != id);
        for seam in self.seams.iter_mut().filter(|s| s.step == Some(id)) {
            seam.step = None;
        }
        self.steps.len() < before
    }

    /// Mueve una etapa en el orden de montaje (`delta` < 0 = antes).
    pub fn move_step(&mut self, id: StepId, delta: isize) {
        let Some(from) = self.steps.iter().position(|s| s.id == id) else { return };
        let to = (from as isize + delta).clamp(0, self.steps.len() as isize - 1) as usize;
        let step = self.steps.remove(from);
        self.steps.insert(to, step);
    }

    /// Posición en el montaje de la etapa de una costura: 0 sin etapa (o con
    /// una que ya no existe), 1 la primera etapa, etc.
    pub fn seam_stage(&self, seam: &Seam) -> usize {
        seam.step.and_then(|id| self.steps.iter().position(|s| s.id == id)).map_or(0, |i| i + 1)
    }

    /// Turno en el que se cose cada costura (en el orden de `seams`): las
    /// etapas, en orden, y, con `one_by_one`, cada costura de una etapa
    /// detrás de la anterior. Los turnos van seguidos desde 0.
    pub fn sewing_turns(&self, one_by_one: bool) -> Vec<u32> {
        let keys: Vec<(usize, usize)> =
            self.seams.iter().enumerate().map(|(i, s)| (self.seam_stage(s), if one_by_one { i } else { 0 })).collect();
        let mut distinct = keys.clone();
        distinct.sort_unstable();
        distinct.dedup();
        keys.iter().map(|k| distinct.binary_search(k).unwrap_or(0) as u32).collect()
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
                if line.fold.is_some() {
                    // Va de borde a borde: por dentro tiene que ir por la tela.
                    let inner = &samples[k][1..samples[k].len() - 1];
                    if !piece.fold_spans(line) || !inner.iter().all(|&p| self.in_fabric(piece, p, 0.0)) {
                        issues.push(Issue::FoldNotAcross { piece: piece.id, line: line.id });
                    } else if samples[..k].iter().any(|other| polylines_cross(&samples[k], other)) {
                        issues.push(Issue::LinesCross { piece: piece.id, line: line.id });
                    }
                } else if !samples[k].iter().all(|&p| self.in_fabric(piece, p, LINE_MARGIN)) {
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
            let lengths: Option<Vec<f64>> = seam.sides.iter().map(|s| self.side_length(s)).collect();
            match lengths {
                None => issues.push(Issue::DanglingSeam { seam: seam.id }),
                Some(lengths) if !lengths.is_empty() => {
                    let min = lengths.iter().copied().fold(f64::INFINITY, f64::min);
                    let max = lengths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let differ = max - min > SEAM_LENGTH_TOLERANCE;
                    match seam.fit {
                        SeamFit::Exact if differ => {
                            issues.push(Issue::SeamLengthMismatch { seam: seam.id, min, max });
                        }
                        SeamFit::Eased if max > min * MAX_EASE => {
                            issues.push(Issue::TooMuchEase { seam: seam.id, min, max });
                        }
                        _ => {}
                    }
                    let notches: Vec<usize> = seam.sides.iter().map(|s| self.side_notches(s).len()).collect();
                    if notches.iter().any(|&n| n != notches[0]) {
                        issues.push(Issue::NotchesMismatch { seam: seam.id });
                    }
                }
                Some(_) => {}
            }
        }
        for fastener in &self.fasteners {
            let inside = fastener.ends.iter().all(|&end| {
                let (Some(piece), Some(at)) = (self.piece(end.piece), self.mark_point(end)) else { return false };
                self.in_fabric(piece, at, 0.0)
            });
            if !inside {
                issues.push(Issue::FastenerOutside { fastener: fastener.id });
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
        let sides = ids.iter().map(|&id| SeamSide::new(id, p.piece(id).unwrap().edges()[2].id, false)).collect();
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
        p.add_seam("", vec![SeamSide::new(a, ea, false), SeamSide::new(b, eb, true)]);
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

    /// Un círculo de radio `r` hecho con cuatro curvas, alrededor de `c`.
    fn circle(p: &mut Pattern, c: Point, r: f64) -> PieceId {
        let pts = [(0.0, -r), (r, 0.0), (0.0, r), (-r, 0.0)].map(|(x, y)| c + Vec2::new(x, y));
        let id = p.add_polygon("base", pts.to_vec());
        let k = r * 0.552_284_75;
        let piece = p.piece_mut(id).unwrap();
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            let tangent = |q: Point| Vec2::new((c - q).y, -(c - q).x).normalize() * k;
            piece.set_edge_kind(i, EdgeKind::Cubic { h1: tangent(a), h2: -tangent(b) });
        }
        id
    }

    /// Una base redonda cosida de una vez al bajo de un cuerpo: el lado de
    /// la base es la cadena de sus cuatro curvas y mide lo mismo que el bajo.
    #[test]
    fn chains_follow_the_outline() {
        let mut p = Pattern::new("t");
        let r = 60.0;
        let body = p.add_rect("cuerpo", Point::ZERO, 2.0 * std::f64::consts::PI * r, 130.0);
        let base = circle(&mut p, Point::new(200.0, 300.0), r);
        let e = |p: &Pattern, id: PieceId, i: usize| p.piece(id).unwrap().edges()[i].id;
        let around = SeamSide::new(base, e(&p, base, 1), true).through(e(&p, base, 0));
        assert_eq!(p.side_edges(&around).unwrap(), [1, 2, 3, 0].map(|i| e(&p, base, i)));
        let seam = p.add_seam("base", vec![SeamSide::new(body, e(&p, body, 2), false), around]).unwrap();
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        assert_eq!(p.seams_of_edge(base, e(&p, base, 3)).count(), 1);

        // Partir una curva de la cadena no cambia lo que se cose, tampoco
        // la última.
        let before = p.side_length(&p.seam(seam).unwrap().sides[1]).unwrap();
        p.split_edge(base, 0, 0.5).unwrap();
        let side = p.seam(seam).unwrap().sides[1];
        assert_eq!(p.side_edges(&side).unwrap().len(), 5);
        assert!((p.side_length(&side).unwrap() - before).abs() < 1e-6);
        // Una cadena que no llega a su arista final no vale.
        let lost = SeamSide { end: Some(EdgeId(9999)), ..side };
        assert!(p.side_edges(&lost).is_none());
    }

    /// Un tramo cose solo parte del lado, y los piquetes dentro de él se
    /// cuentan en el sentido de la costura.
    #[test]
    fn ranges_and_notches() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("a", Point::ZERO, 200.0, 100.0);
        let top = p.piece(a).unwrap().edges()[0].id;
        let half = SeamSide::new(a, top, false).span(0.25, 0.75);
        let spans = p.side_spans(&half).unwrap();
        assert_eq!(spans.len(), 1);
        assert!((spans[0].length - 100.0).abs() < 1e-6);
        let piece = p.piece_mut(a).unwrap();
        assert!(piece.toggle_notch(top, 0.4, 1.0));
        assert!(piece.toggle_notch(top, 0.9, 1.0));
        let notches = p.side_notches(&half);
        assert!(notches.len() == 1 && (notches[0] - 0.3).abs() < 1e-9, "{notches:?}");
        assert!((p.side_notches(&SeamSide { reversed: true, ..half })[0] - 0.7).abs() < 1e-9);
        // Otra vez en el mismo sitio, se quita.
        assert!(!p.piece_mut(a).unwrap().toggle_notch(top, 0.4, 1.0));
        assert!(p.side_notches(&half).is_empty());

        // Al partir la arista, el piquete se queda donde estaba.
        let piece = p.piece_mut(a).unwrap();
        let at = piece.notch_point(&piece.notches[0]).unwrap();
        p.split_edge(a, 0, 0.5).unwrap();
        let piece = p.piece(a).unwrap();
        assert!(piece.notch_point(&piece.notches[0]).unwrap().distance(at) < 1e-6);
        assert_ne!(piece.notches[0].edge, top);
    }

    /// Un fruncido o un estirado no avisa de que los lados no miden lo
    /// mismo; un embebido, solo si es demasiado.
    #[test]
    fn fit_allows_different_lengths() {
        let mut p = Pattern::new("t");
        let a = p.add_rect("falda", Point::ZERO, 300.0, 200.0);
        let b = p.add_rect("cinturilla", Point::new(0.0, 300.0), 200.0, 40.0);
        let side = |p: &Pattern, id: PieceId, i: usize| SeamSide::new(id, p.piece(id).unwrap().edges()[i].id, false);
        let seam = p.add_seam("cintura", vec![side(&p, a, 0), side(&p, b, 2)]).unwrap();
        assert!(matches!(p.validate()[..], [Issue::SeamLengthMismatch { .. }]));
        p.seam_mut(seam).unwrap().fit = SeamFit::Gathered;
        assert!(p.validate().is_empty());
        p.seam_mut(seam).unwrap().fit = SeamFit::Eased;
        assert!(matches!(p.validate()[..], [Issue::TooMuchEase { .. }]));
        // Un piquete en un lado y no en el otro.
        p.seam_mut(seam).unwrap().fit = SeamFit::Stretched;
        let top = p.piece(a).unwrap().edges()[0].id;
        p.piece_mut(a).unwrap().toggle_notch(top, 0.5, 1.0);
        assert_eq!(p.validate(), [Issue::NotchesMismatch { seam }]);
    }

    /// Las etapas se cosen en orden; sin etapa, primero. Coser de una en
    /// una separa las costuras de cada etapa.
    /// Un doblez parte las aristas por donde empieza y acaba, dobla la parte
    /// pequeña, sigue a sus vértices y cambia de lado al reflejar la pieza.
    #[test]
    fn folds_go_across_the_piece() {
        let mut p = Pattern::new("t");
        let cuff = p.add_rect("puño", Point::ZERO, 300.0, 120.0);
        // Un poco por encima de la mitad: la parte de arriba es más pequeña.
        let fold = p.add_fold(cuff, Point::new(-2.0, 50.0), Point::new(303.0, 50.0)).unwrap();
        let piece = p.piece(cuff).unwrap();
        assert_eq!(piece.len(), 6, "parte las dos aristas de los lados");
        let line = piece.lines[piece.line_index(fold).unwrap()];
        assert!(piece.fold_spans(&line));
        assert!(line.in_flap(Point::new(150.0, 10.0)) && !line.in_flap(Point::new(150.0, 100.0)));
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        // En la misma arista no hay doblez.
        assert!(p.add_fold(cuff, Point::new(10.0, -1.0), Point::new(200.0, 1.0)).is_none());
        // Ni hasta la esquina de al lado, y sin dejar la arista partida.
        let before = p.clone();
        assert!(p.add_fold(cuff, Point::new(150.0, 0.0), Point::new(0.0, 0.0)).is_none());
        assert_eq!(p, before);

        // Al mover el vértice, el doblez lo sigue.
        let piece = p.piece_mut(cuff).unwrap();
        let v = piece.vertices().iter().position(|v| v.distance(Point::new(300.0, 50.0)) < 1e-6).unwrap();
        piece.set_vertex(v, Point::new(310.0, 60.0));
        let line = piece.lines[0];
        assert_eq!(line.to, Point::new(310.0, 60.0));
        assert!(p.validate().is_empty(), "{:?}", p.validate());

        // Reflejada, la solapa sigue siendo la parte de arriba.
        let mirrored = p.mirror_piece(cuff).unwrap();
        let line = p.piece(mirrored).unwrap().lines[0];
        let top = p.piece(mirrored).unwrap().vertices().iter().map(|v| v.y).fold(f64::INFINITY, f64::min);
        let center_x = (line.from.x + line.to.x) / 2.0;
        assert!(line.in_flap(Point::new(center_x, top + 5.0)));

        // Un doblez suelto por dentro de la pieza no vale.
        let piece = p.piece_mut(cuff).unwrap();
        piece.lines[0].from = Point::new(20.0, 50.0);
        assert!(p.validate().contains(&Issue::FoldNotAcross { piece: cuff, line: fold }), "{:?}", p.validate());
    }

    #[test]
    fn fasteners_mark_their_points_and_move_with_the_piece() {
        let mut p = Pattern::new("t");
        let flap = p.add_rect("solapa", Point::ZERO, 200.0, 100.0);
        let body = p.add_rect("cuerpo", Point::new(0.0, 150.0), 200.0, 200.0);
        // Fuera de la tela no se puede.
        assert!(
            p.add_fastener(FastenerKind::Snap, (flap, Point::new(-5.0, 50.0)), (body, Point::new(100.0, 200.0)))
                .is_none()
        );
        assert!(p.pieces.iter().all(|piece| piece.marks.is_empty()));
        let snap = p
            .add_fastener(FastenerKind::Snap, (flap, Point::new(100.0, 80.0)), (body, Point::new(100.0, 200.0)))
            .unwrap();
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        assert!(p.fastener(snap).unwrap().joins());
        p.fastener_mut(snap).unwrap().open = true;
        assert!(!p.fastener(snap).unwrap().joins());
        // Se mueve con su pieza.
        p.piece_mut(flap).unwrap().translate(Vec2::new(10.0, 0.0));
        let end = p.fastener(snap).unwrap().ends[0];
        assert_eq!(p.mark_point(end), Some(Point::new(110.0, 80.0)));
        // Si la pieza encoge y el punto queda fuera, se avisa.
        let piece = p.piece_mut(flap).unwrap();
        for v in 2..4 {
            let at = piece.vertices()[v];
            piece.set_vertex(v, Point::new(at.x, 50.0));
        }
        assert_eq!(p.validate(), [Issue::FastenerOutside { fastener: snap }]);
        // Al quitarlo, se van sus puntos; al quitar una pieza, sus broches.
        p.remove_fastener(snap);
        assert!(p.pieces.iter().all(|piece| piece.marks.is_empty()));
        let tack =
            p.add_fastener(FastenerKind::BarTack, (body, Point::new(50.0, 200.0)), (body, Point::new(150.0, 200.0)));
        assert!(tack.is_some());
        p.remove_piece(body);
        assert!(p.fasteners.is_empty());
    }

    #[test]
    fn steps_set_the_sewing_turns() {
        let mut p = Pattern::new("t");
        let ids: Vec<PieceId> = (0..4).map(|i| p.add_rect(format!("{i}"), Point::ZERO, 100.0, 100.0)).collect();
        let side = |p: &Pattern, id: PieceId| SeamSide::new(id, p.piece(id).unwrap().edges()[0].id, false);
        let seams: Vec<SeamId> =
            (0..3).map(|i| p.add_seam(format!("{i}"), vec![side(&p, ids[i]), side(&p, ids[i + 1])]).unwrap()).collect();
        assert_eq!(p.sewing_turns(false), [0, 0, 0]);
        assert_eq!(p.sewing_turns(true), [0, 1, 2]);
        let (outer, lining) = (p.add_step("Exterior"), p.add_step("Forro"));
        p.seam_mut(seams[0]).unwrap().step = Some(lining);
        p.seam_mut(seams[1]).unwrap().step = Some(outer);
        assert_eq!(p.sewing_turns(false), [2, 1, 0]);
        p.move_step(lining, -1);
        assert_eq!(p.sewing_turns(false), [1, 2, 0]);
        p.seam_mut(seams[2]).unwrap().step = Some(lining);
        assert_eq!(p.sewing_turns(true), [0, 2, 1]);
        assert!(p.remove_step(lining));
        assert_eq!(p.sewing_turns(false), [0, 1, 0]);
        assert!(p.seams.iter().all(|s| s.step != Some(lining)));
        // Las etapas no repiten ids con lo demás.
        assert!(p.add_step("Unión").0 > seams[2].0);
    }
}
