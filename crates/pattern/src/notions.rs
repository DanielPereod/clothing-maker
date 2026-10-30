//! Fornituras: cremalleras, cinchas, bieses, vivos y elásticos. Cada una se
//! genera como piezas normales, con su tela, sus dobleces y sus costuras, que
//! luego se cambian y se cosen con las herramientas de siempre.

use kurbo::ParamCurve;

use crate::{
    EdgeId, Pattern, PieceId, Point, SeamFit, SeamId, SeamKind, SeamSide, Textile, TextileId, Topstitch, Vec2,
};

/// Ancho (mm) de cada cinta de una cremallera, con sus dientes.
pub const ZIPPER_TAPE_WIDTH: f64 = 15.0;
/// Separación (mm) en el patrón entre una fornitura y el borde en el que se
/// cose: la costura la junta al simular.
const NOTION_GAP: f64 = 10.0;
/// Cuánto más corto que su borde se corta un elástico.
pub const ELASTIC_RATIO: f64 = 0.8;

/// Las fornituras que se pueden generar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notion {
    Zipper,
    Webbing,
    Binding,
    Piping,
    Elastic,
}

impl Notion {
    pub const ALL: [Notion; 5] = [Notion::Zipper, Notion::Webbing, Notion::Binding, Notion::Piping, Notion::Elastic];

    pub fn label(self) -> &'static str {
        match self {
            Notion::Zipper => "Cremallera",
            Notion::Webbing => "Cincha",
            Notion::Binding => "Bies",
            Notion::Piping => "Vivo",
            Notion::Elastic => "Elástico",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Notion::Zipper => {
                "Dos cintas y sus dientes, que se abren y se cierran. Con una costura de dos lados elegida (la \
                 boca de un bolso), la cremallera va en su lugar: cada cinta se cose a un lado."
            }
            Notion::Webbing => {
                "Una cinta firme para asas y correas. Se cose después, por ejemplo sobre líneas interiores."
            }
            Notion::Binding => {
                "Tira al bies doblada por la mitad, ya planchada, cosida por sus dos cantos al borde elegido: \
                 remata bocas, escotes y sisas."
            }
            Notion::Piping => {
                "Tira doblada por la mitad que se mete en la costura elegida, con el doblez asomando entre las \
                 dos piezas."
            }
            Notion::Elastic => {
                "Cinta elástica más corta que el borde elegido, cosida estirada: al soltarse, frunce la tela."
            }
        }
    }

    /// Ancho (mm) por defecto: el de una cinta de la cremallera, el de la
    /// cincha o el elástico, o lo que se ve del bies o del vivo.
    pub fn default_width(self) -> f64 {
        match self {
            Notion::Zipper => ZIPPER_TAPE_WIDTH,
            Notion::Webbing => 25.0,
            Notion::Binding => 10.0,
            Notion::Piping => 5.0,
            Notion::Elastic => 20.0,
        }
    }

    /// Si se genera a partir de lo elegido: un borde (bies, elástico) o una
    /// costura (vivo). La cremallera y la cincha pueden ir sueltas.
    pub fn needs_edge(self) -> bool {
        matches!(self, Notion::Binding | Notion::Elastic)
    }

    pub fn needs_seam(self) -> bool {
        self == Notion::Piping
    }

    /// Su tela, con id 0: se le da uno al añadirla a un patrón.
    pub fn textile(self) -> Textile {
        let (name, firmness, weight, stretch, friction, thickness) = match self {
            Notion::Zipper => ("Cinta de cremallera", 62.0, 500.0, 5.0, 0.4, 2.0),
            Notion::Webbing => ("Cincha", 78.0, 650.0, 8.0, 0.6, 2.0),
            Notion::Binding => ("Bies", 38.0, 130.0, 65.0, 0.5, 1.0),
            Notion::Piping => ("Vivo", 55.0, 250.0, 30.0, 0.5, 3.0),
            Notion::Elastic => ("Elástico", 40.0, 300.0, 95.0, 0.6, 1.5),
        };
        Textile { id: TextileId(0), name: name.into(), firmness, weight, stretch, friction, thickness }
    }
}

/// Lo que se ha generado: las piezas nuevas y las costuras nuevas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotionParts {
    pub pieces: Vec<PieceId>,
    pub seams: Vec<SeamId>,
}

/// Una tira generada a lo largo de un lado: su pieza y sus aristas largas,
/// la de junto al lado y la de fuera, las dos en el sentido del lado.
struct Strip {
    piece: PieceId,
    near: SeamSide,
    far: SeamSide,
}

impl Pattern {
    /// La tela de una fornitura en la biblioteca: la que se llama igual o,
    /// si no hay, una nueva.
    pub fn notion_textile(&mut self, notion: Notion) -> TextileId {
        let textile = notion.textile();
        if let Some(t) = self.textiles.iter().find(|t| t.name == textile.name) {
            return t.id;
        }
        let id = TextileId(self.textiles.iter().map(|t| t.id.0).max().unwrap_or(0) + 1);
        self.textiles.push(Textile { id, ..textile });
        id
    }

    /// Principio, final y normal hacia fuera de la pieza de un lado de
    /// costura, en su sentido.
    pub fn side_ends(&self, side: &SeamSide) -> Option<(Point, Point, Vec2)> {
        let piece = self.piece(side.piece)?;
        let spans = self.side_spans(side)?;
        let (first, last) = (spans.first()?, spans.last()?);
        let mut start = piece.edge_segment(first.edge)?.eval(first.from);
        let mut end = piece.edge_segment(last.edge)?.eval(last.to);
        if side.reversed {
            std::mem::swap(&mut start, &mut end);
        }
        let chord = end - start;
        let len = chord.hypot();
        if len < 1e-9 {
            return None;
        }
        let mut normal = Vec2::new(chord.y, -chord.x) / len;
        let mid = start.midpoint(end);
        if piece.contains(mid + normal * 1.0) {
            normal = -normal;
        }
        Some((start, end, normal))
    }

    /// Una tira recta de una fornitura de `width` mm, a lo largo de un lado
    /// (de su principio a su final, `length` mm) y por fuera de su pieza, a
    /// `NOTION_GAP` de ella. Va en el mismo sitio en 3D que la pieza.
    fn strip_along(&mut self, notion: Notion, name: String, side: &SeamSide, length: f64, width: f64) -> Option<Strip> {
        let (start, end, normal) = self.side_ends(side)?;
        let dir = (end - start) / (end - start).hypot();
        // Centrada en el lado si mide otra cosa (un elástico más corto).
        let mid = start.midpoint(end);
        let q0 = mid - dir * (length / 2.0) + normal * NOTION_GAP;
        let q1 = mid + dir * (length / 2.0) + normal * NOTION_GAP;
        let points = vec![q0, q1, q1 + normal * width, q0 + normal * width];
        let id = self.add_polygon(name, points);
        let fabric = self.notion_textile(notion);
        let host = self.piece(side.piece)?.clone();
        let piece = self.piece_mut(id)?;
        piece.fabric = fabric;
        (piece.arrangement, piece.zone, piece.layer) = (host.arrangement, host.zone, host.layer);
        let edges: Vec<EdgeId> = piece.edges().iter().map(|e| e.id).collect();
        Some(Strip { piece: id, near: SeamSide::new(id, edges[0], false), far: SeamSide::new(id, edges[2], true) })
    }

    /// Una cincha o un elástico sueltos, de `length` × `width` mm, con la
    /// esquina en `origin` y a lo largo de X.
    pub fn add_strap(&mut self, notion: Notion, length: f64, width: f64, origin: Point) -> PieceId {
        let id = self.add_rect(notion.label(), origin, length, width);
        let fabric = self.notion_textile(notion);
        if let Some(piece) = self.piece_mut(id) {
            piece.fabric = fabric;
        }
        id
    }

    /// Cremallera suelta de `length` mm, con cintas de `width` mm: las dos
    /// a lo largo de X, una encima de la otra, y la costura de los dientes
    /// entre ellas.
    pub fn add_zipper(&mut self, length: f64, width: f64, origin: Point) -> NotionParts {
        let w = width;
        let a = self.add_strap(Notion::Zipper, length, w, origin);
        let b = self.add_strap(Notion::Zipper, length, w, origin + Vec2::new(0.0, w + NOTION_GAP));
        for (id, name) in [(a, "Cremallera (cinta 1)"), (b, "Cremallera (cinta 2)")] {
            if let Some(piece) = self.piece_mut(id) {
                piece.name = name.into();
            }
        }
        // Los dientes: el borde de abajo de la primera y el de arriba de la
        // segunda, los dos de izquierda a derecha.
        let edge = |p: &Pattern, id: PieceId, i: usize| p.piece(id).map(|p| p.edges()[i].id);
        let teeth = match (edge(self, a, 2), edge(self, b, 0)) {
            (Some(ea), Some(eb)) => {
                self.teeth(SeamSide::new(a, ea, true), SeamSide::new(b, eb, false), "Cremallera".into(), None)
            }
            _ => None,
        };
        NotionParts { pieces: vec![a, b], seams: teeth.into_iter().collect() }
    }

    /// La costura de los dientes entre los cantos de dos cintas, que ya van
    /// en el mismo sentido. Los cantos se quedan sin margen.
    fn teeth(&mut self, a: SeamSide, b: SeamSide, name: String, step: Option<crate::StepId>) -> Option<SeamId> {
        let id = self.add_seam(name, vec![a, b])?;
        let seam = self.seam_mut(id)?;
        seam.set_kind(SeamKind::Zipper);
        seam.step = step;
        for side in [a, b] {
            if let Some(p) = self.piece_mut(side.piece)
                && let Some(i) = p.edge_index(side.edge)
            {
                p.set_edge_allowance(i, Some(0.0));
            }
        }
        Some(id)
    }

    /// Pone una cremallera en lugar de una costura de dos lados: una cinta
    /// de `width` mm por fuera de cada lado, cosida a él, y los dientes entre
    /// las dos. La cremallera se abre desde el principio de la costura.
    pub fn zip_seam(&mut self, seam: SeamId, width: f64) -> Option<NotionParts> {
        let mut draft = self.clone();
        let parts = draft.zip_seam_in_place(seam, width)?;
        *self = draft;
        Some(parts)
    }

    fn zip_seam_in_place(&mut self, seam: SeamId, width: f64) -> Option<NotionParts> {
        let old = self.seam(seam)?.clone();
        let [side_a, side_b] = old.sides[..] else { return None };
        self.remove_seam(seam);
        let mut parts = NotionParts::default();
        let mut tapes = Vec::new();
        for (k, side) in [side_a, side_b].iter().enumerate() {
            let length = self.side_length(side)?;
            let name = format!("{} (cinta {})", old.name, k + 1);
            let strip = self.strip_along(Notion::Zipper, name, side, length, width)?;
            let host = self.piece(side.piece)?.name.clone();
            let attach = self.add_seam(format!("{} · {host}", old.name), vec![*side, strip.near])?;
            let s = self.seam_mut(attach)?;
            s.step = old.step;
            s.pressed_to = Some(0);
            s.topstitch = Topstitch::Single;
            parts.pieces.push(strip.piece);
            parts.seams.push(attach);
            tapes.push(strip);
        }
        let id = self.teeth(tapes[0].far, tapes[1].far, old.name.clone(), old.step)?;
        parts.seams.push(id);
        Some(parts)
    }

    /// Una tira doblada por la mitad a lo largo de un lado (bies o vivo):
    /// de `2 · width` mm de ancho, con el doblez planchado y la mitad de
    /// fuera doblada sobre la de dentro, así sus dos cantos quedan junto al
    /// lado. Devuelve la tira.
    fn folded_strip(&mut self, notion: Notion, name: String, side: &SeamSide, width: f64) -> Option<Strip> {
        let length = self.side_length(side)?;
        let strip = self.strip_along(notion, name, side, length, 2.0 * width)?;
        let piece = self.piece(strip.piece)?;
        let v = piece.vertices();
        let (from, to) = (v[0].midpoint(v[3]), v[1].midpoint(v[2]));
        let far = v[2].midpoint(v[3]);
        let line = self.add_fold(strip.piece, from, to)?;
        let piece = self.piece_mut(strip.piece)?;
        let k = piece.line_index(line)?;
        let fold = &mut piece.lines[k];
        if !fold.in_flap(far)
            && let Some(f) = &mut fold.fold
        {
            f.flip = !f.flip;
        }
        // El doblez parte las aristas cortas: las largas siguen igual.
        Some(strip)
    }

    /// Remata un borde con bies: una tira doblada por la mitad, cosida por
    /// sus dos cantos al lado `side`.
    pub fn bind_edge(&mut self, side: SeamSide, width: f64) -> Option<NotionParts> {
        let mut draft = self.clone();
        let host = draft.piece(side.piece)?.name.clone();
        let strip = draft.folded_strip(Notion::Binding, format!("Bies de {host}"), &side, width)?;
        let seam = draft.add_seam(format!("Bies de {host}"), vec![side, strip.near, strip.far])?;
        *self = draft;
        Some(NotionParts { pieces: vec![strip.piece], seams: vec![seam] })
    }

    /// Mete un vivo en una costura: una tira doblada por la mitad a lo largo
    /// de su primer lado, con los dos cantos cosidos en la costura.
    pub fn pipe_seam(&mut self, seam: SeamId, width: f64) -> Option<NotionParts> {
        let mut draft = self.clone();
        let old = draft.seam(seam)?.clone();
        let first = *old.sides.first()?;
        let strip = draft.folded_strip(Notion::Piping, format!("Vivo de {}", old.name), &first, width)?;
        draft.seam_mut(seam)?.sides.extend([strip.near, strip.far]);
        *self = draft;
        Some(NotionParts { pieces: vec![strip.piece], seams: Vec::new() })
    }

    /// Cose un elástico a un borde: más corto que él (`ELASTIC_RATIO`) y
    /// estirado al coserlo. Al soltarlo, el elástico recupera su largo y
    /// frunce la tela (que se arruga con mucha menos fuerza de la que hace
    /// falta para estirarlo), así que la costura es fruncida.
    pub fn add_elastic(&mut self, side: SeamSide, width: f64) -> Option<NotionParts> {
        let mut draft = self.clone();
        let host = draft.piece(side.piece)?.name.clone();
        let length = draft.side_length(&side)? * ELASTIC_RATIO;
        let strip = draft.strip_along(Notion::Elastic, format!("Elástico de {host}"), &side, length, width)?;
        let seam = draft.add_seam(format!("Elástico de {host}"), vec![side, strip.near])?;
        let s = draft.seam_mut(seam)?;
        s.fit = SeamFit::Gathered;
        s.set_kind(SeamKind::Overlocked);
        *self = draft;
        Some(NotionParts { pieces: vec![strip.piece], seams: vec![seam] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un bolso de dos piezas con la boca cosida: delante y detrás, una
    /// encima de la otra en el patrón.
    fn bag() -> (Pattern, PieceId, PieceId, SeamId) {
        let mut p = Pattern::new("bolso");
        let front = p.add_rect("Delante", Point::new(0.0, 0.0), 300.0, 250.0);
        let back = p.add_rect("Detrás", Point::new(400.0, 0.0), 300.0, 250.0);
        let top = |p: &Pattern, id: PieceId| p.piece(id).unwrap().edges()[0].id;
        let (ta, tb) = (top(&p, front), top(&p, back));
        let mouth = p.add_seam("Boca", vec![SeamSide::new(front, ta, false), SeamSide::new(back, tb, false)]).unwrap();
        (p, front, back, mouth)
    }

    #[test]
    fn zippers_replace_the_seam_with_two_tapes() {
        let (mut p, front, back, mouth) = bag();
        let parts = p.zip_seam(mouth, ZIPPER_TAPE_WIDTH).unwrap();
        assert_eq!(parts.pieces.len(), 2);
        assert!(p.seam(mouth).is_none(), "la costura de la boca ya no está");
        let teeth = p.seams.iter().find(|s| s.kind == SeamKind::Zipper).unwrap();
        assert_eq!(teeth.name, "Boca");
        // Cada cinta, por fuera de su pieza (encima, con Y hacia abajo) y
        // con la tela de cremallera.
        for (&tape, host) in parts.pieces.iter().zip([front, back]) {
            let piece = p.piece(tape).unwrap();
            assert_eq!(p.textile(piece.fabric).unwrap().name, "Cinta de cremallera");
            assert!(piece.bounding_box().y1 < p.piece(host).unwrap().bounding_box().y0);
            assert!((p.piece(tape).unwrap().bounding_box().width() - 300.0).abs() < 1e-6);
        }
        // Los dientes van en el sentido de la boca: sus principios, a la
        // izquierda.
        for side in &teeth.sides {
            let (start, end, _) = p.side_ends(side).unwrap();
            assert!(start.x < end.x, "{start:?} → {end:?}");
            assert_eq!(p.allowance(side.piece, side.edge), 0.0);
        }
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        // Una costura de tres lados no se puede cambiar por una cremallera.
        let extra = p.add_rect("x", Point::new(0.0, 400.0), 100.0, 100.0);
        let e = p.piece(extra).unwrap().edges()[0].id;
        let seam = p.seams[0].id;
        p.seam_mut(seam).unwrap().sides.push(SeamSide::new(extra, e, false));
        let before = p.clone();
        assert!(p.zip_seam(seam, ZIPPER_TAPE_WIDTH).is_none());
        assert_eq!(p, before);
    }

    #[test]
    fn open_zippers_leave_their_start_unsewn() {
        let mut p = Pattern::new("z");
        let parts = p.add_zipper(200.0, ZIPPER_TAPE_WIDTH, Point::ZERO);
        let seam = p.seam_mut(parts.seams[0]).unwrap();
        assert_eq!(seam.kind, SeamKind::Zipper);
        assert!(!seam.is_open_at(0.0));
        seam.open = 0.5;
        assert!(seam.is_open_at(0.2) && seam.is_open_at(0.5) && !seam.is_open_at(0.7));
        assert!(p.validate().is_empty(), "{:?}", p.validate());
    }

    #[test]
    fn bindings_and_piping_fold_in_half() {
        let (mut p, front, _, mouth) = bag();
        let bottom = SeamSide::new(front, p.piece(front).unwrap().edges()[2].id, false);
        let binding = p.bind_edge(bottom, 10.0).unwrap();
        let strip = p.piece(binding.pieces[0]).unwrap();
        assert_eq!(strip.lines.len(), 1);
        let fold = strip.lines[0];
        assert!(fold.fold.is_some_and(|f| f.pressed && f.angle == 180.0));
        let seam = p.seam(binding.seams[0]).unwrap();
        assert_eq!(seam.sides.len(), 3, "el borde y los dos cantos del bies");
        // La mitad de fuera es la que se dobla.
        let far = p.side_ends(&seam.sides[2]).unwrap().0;
        assert!(fold.in_flap(far + (fold.from - far) * 0.1), "{far:?}");

        let piping = p.pipe_seam(mouth, 5.0).unwrap();
        assert_eq!(p.seam(mouth).unwrap().sides.len(), 4);
        assert_eq!(p.textile(p.piece(piping.pieces[0]).unwrap().fabric).unwrap().name, "Vivo");
        assert!(p.validate().is_empty(), "{:?}", p.validate());
    }

    #[test]
    fn elastic_is_shorter_and_gathers_its_edge() {
        let (mut p, front, ..) = bag();
        let bottom = SeamSide::new(front, p.piece(front).unwrap().edges()[2].id, false);
        let parts = p.add_elastic(bottom, 20.0).unwrap();
        let seam = p.seam(parts.seams[0]).unwrap();
        assert_eq!(seam.fit, SeamFit::Gathered);
        let lengths: Vec<f64> = seam.sides.iter().map(|s| p.side_length(s).unwrap()).collect();
        assert!((lengths[1] / lengths[0] - ELASTIC_RATIO).abs() < 1e-6, "{lengths:?}");
        // La tela elástica, una sola vez en la biblioteca.
        p.add_elastic(SeamSide::new(front, p.piece(front).unwrap().edges()[1].id, false), 20.0).unwrap();
        assert_eq!(p.textiles.iter().filter(|t| t.name == "Elástico").count(), 1);
    }
}
