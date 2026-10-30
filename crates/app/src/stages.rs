//! Etapas de trabajo, como los espacios de trabajo de Fusion 360: una prenda
//! se hace en orden (patrón, telas, montaje, colocación y simulación) y cada
//! etapa tiene sus herramientas, sus propiedades y sus comprobaciones.
//!
//! Las comprobaciones dicen lo que falta o está mal en cada etapa antes de
//! seguir. Se calculan con el patrón y, las que dependen de la escena 3D
//! (piezas que no se pueden mallar, costuras que se cruzan al colocar), con
//! lo que cuenta la vista 3D.

use pattern::{Arrangement, BodyZone, Issue, Pattern, Piece, PieceId, Point, SeamId};

use crate::canvas::Selection;

/// Etapa de trabajo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Dibujar las piezas.
    Pattern,
    /// Elegir la tela de cada pieza.
    Fabrics,
    /// Coser: qué se une con qué, cómo y en qué orden.
    Assembly,
    /// Dónde empieza cada pieza en 3D antes de coser.
    Placement,
    /// Coser y asentar la prenda en 3D.
    Simulation,
}

impl Stage {
    pub const ALL: [Stage; 5] = [Stage::Pattern, Stage::Fabrics, Stage::Assembly, Stage::Placement, Stage::Simulation];

    pub fn label(self) -> &'static str {
        match self {
            Stage::Pattern => "Patrón",
            Stage::Fabrics => "Telas",
            Stage::Assembly => "Montaje",
            Stage::Placement => "Colocación",
            Stage::Simulation => "Simulación",
        }
    }

    /// Número de la etapa, empezando en 1.
    pub fn number(self) -> usize {
        Stage::ALL.iter().position(|&s| s == self).expect("está en la lista") + 1
    }

    pub fn hint(self) -> &'static str {
        match self {
            Stage::Pattern => "Dibuja las piezas: contornos, curvas y puntos",
            Stage::Fabrics => "Elige la tela de cada pieza y las capas pegadas (entretelas, refuerzos)",
            Stage::Assembly => "Cose las piezas: qué se une con qué, el tipo de costura, el margen y el orden",
            Stage::Placement => {
                "Decide dónde empieza cada pieza en 3D antes de coser: su lado, su zona del cuerpo y su cara"
            }
            Stage::Simulation => "Cose y asienta la prenda en 3D: obstáculo, relleno y precisión",
        }
    }

    pub fn next(self) -> Option<Stage> {
        Stage::ALL.get(self.number()).copied()
    }

    /// Si en esta etapa se ve la vista 3D a la fuerza.
    pub fn needs_3d(self) -> bool {
        matches!(self, Stage::Placement | Stage::Simulation)
    }
}

/// Algo que falta o está mal en una etapa.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub stage: Stage,
    pub text: String,
    /// Qué seleccionar para verlo.
    pub target: Option<Selection>,
}

/// Lo que cuenta la vista 3D de la escena cargada.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneReport {
    /// Piezas que no se han podido mallar.
    pub failed: Vec<PieceId>,
    /// Costuras cuyos lados quedan en sentidos contrarios al colocar las
    /// piezas: al cerrarse se retuercen.
    pub crossed: Vec<SeamId>,
}

/// Comprobaciones de todas las etapas, en orden de etapa.
pub fn checks(pattern: &Pattern, scene: &SceneReport) -> Vec<Check> {
    let piece_name = |id: PieceId| pattern.piece(id).map_or("?".to_owned(), |p| p.name.clone());
    let seam_name = |id: SeamId| pattern.seam(id).map_or("?".to_owned(), |s| s.name.clone());
    let mut out = Vec::new();
    let mut push = |stage, text: String, target| out.push(Check { stage, text, target });

    // Patrón.
    if pattern.pieces.is_empty() {
        push(Stage::Pattern, "Aún no hay piezas: dibuja una con la pluma (P) o el rectángulo (R)".into(), None);
    }
    let issues = pattern.validate();
    for issue in &issues {
        match *issue {
            Issue::TooFewVertices { piece } => {
                push(Stage::Pattern, format!("«{}» tiene menos de 3 puntos", piece_name(piece)), piece_target(piece))
            }
            Issue::DegeneratePiece { piece } => {
                push(Stage::Pattern, format!("«{}» no tiene área", piece_name(piece)), piece_target(piece))
            }
            Issue::SelfIntersecting { piece } => push(
                Stage::Pattern,
                format!("El contorno de «{}» se cruza consigo mismo", piece_name(piece)),
                piece_target(piece),
            ),
            _ => {}
        }
    }

    // Telas.
    for piece in &pattern.pieces {
        if piece.textiles().any(|t| pattern.textile(t).is_none()) {
            push(
                Stage::Fabrics,
                format!("«{}» usa una tela que no está en la biblioteca (se simula como algodón)", piece.name),
                piece_target(piece.id),
            );
        }
    }

    // Montaje.
    if pattern.pieces.len() > 1 && pattern.seams.is_empty() {
        push(Stage::Assembly, "Aún no hay costuras: une las piezas con la herramienta Costura (S)".into(), None);
    }
    for issue in &issues {
        match *issue {
            Issue::DanglingSeam { seam } => push(
                Stage::Assembly,
                format!("La costura «{}» apunta a una arista que no existe", seam_name(seam)),
                Some(Selection::Seam(seam)),
            ),
            Issue::SeamTooFewSides { seam } => push(
                Stage::Assembly,
                format!("La costura «{}» tiene menos de dos lados", seam_name(seam)),
                Some(Selection::Seam(seam)),
            ),
            Issue::SeamLengthMismatch { seam, min, max } => push(
                Stage::Assembly,
                format!(
                    "Los lados de la costura «{}» no miden lo mismo ({:.1}–{:.1} cm)",
                    seam_name(seam),
                    min / 10.0,
                    max / 10.0
                ),
                Some(Selection::Seam(seam)),
            ),
            _ => {}
        }
    }
    // Se ve al colocar las piezas en 3D, pero se arregla al coser.
    for &seam in scene.crossed.iter().filter(|&&id| pattern.seam(id).is_some()) {
        push(
            Stage::Assembly,
            format!(
                "La costura «{}» se retuerce al colocar las piezas: invierte uno de sus lados (⇄) o revisa el \
                 lado de sus piezas",
                seam_name(seam)
            ),
            Some(Selection::Seam(seam)),
        );
    }
    if !pattern.seams.is_empty() {
        for piece in &pattern.pieces {
            let sewn = pattern.seams.iter().any(|s| s.sides.iter().any(|side| side.piece == piece.id));
            if !sewn {
                push(
                    Stage::Assembly,
                    format!("«{}» no está cosida a ninguna pieza", piece.name),
                    piece_target(piece.id),
                );
            }
        }
    }

    // Colocación.
    for (a, b) in overlapping_pieces(pattern) {
        let (pa, pb) = (pattern.piece(a).expect("existe"), pattern.piece(b).expect("existe"));
        push(
            Stage::Placement,
            format!(
                "«{}» y «{}» se solapan en el patrón y van al mismo sitio ({}): en 3D se cruzarán. \
                 Sepáralas o cambia la colocación de una",
                pa.name,
                pb.name,
                slot_label(pattern, pa)
            ),
            piece_target(b),
        );
    }

    // Simulación.
    for &piece in scene.failed.iter().filter(|&&id| pattern.piece(id).is_some()) {
        push(Stage::Simulation, format!("No se ha podido mallar «{}»", piece_name(piece)), piece_target(piece));
    }
    out
}

fn piece_target(id: PieceId) -> Option<Selection> {
    Some(Selection::Piece(id))
}

/// Dónde va una pieza en 3D, en palabras: su lado y, en las prendas, su
/// zona del cuerpo.
pub fn slot_label(pattern: &Pattern, piece: &Piece) -> String {
    if pattern.on_avatar && piece.zone != BodyZone::Torso {
        format!("{} · {}", piece.zone.label(), piece.arrangement.label())
    } else {
        piece.arrangement.label().to_owned()
    }
}

/// Grupo de colocación de una pieza: las del mismo grupo se colocan juntas,
/// con la disposición del editor.
fn slot(pattern: &Pattern, piece: &Piece) -> (BodyZone, Arrangement) {
    let zone = if pattern.on_avatar { piece.zone } else { BodyZone::Torso };
    (zone, piece.arrangement)
}

/// Pares de piezas del mismo grupo de colocación que se solapan en el
/// patrón: en 3D quedarían una encima de otra y se cruzarían.
pub fn overlapping_pieces(pattern: &Pattern) -> Vec<(PieceId, PieceId)> {
    let outlines: Vec<Vec<Point>> = pattern.pieces.iter().map(|p| p.flatten(1.0)).collect();
    let mut out = Vec::new();
    for (i, a) in pattern.pieces.iter().enumerate() {
        for (j, b) in pattern.pieces.iter().enumerate().skip(i + 1) {
            if slot(pattern, a) != slot(pattern, b) || a.len() < 3 || b.len() < 3 {
                continue;
            }
            if !a.bounding_box().intersect(b.bounding_box()).is_zero_area()
                && outlines_overlap(&outlines[i], &outlines[j], a, b)
            {
                out.push((a.id, b.id));
            }
        }
    }
    out
}

/// Dos contornos se solapan si sus bordes se cortan o uno está dentro del
/// otro.
fn outlines_overlap(a: &[Point], b: &[Point], pa: &Piece, pb: &Piece) -> bool {
    let crossing = (0..a.len()).any(|i| {
        let (p, q) = (a[i], a[(i + 1) % a.len()]);
        (0..b.len()).any(|j| segments_cross(p, q, b[j], b[(j + 1) % b.len()]))
    });
    crossing || a.first().is_some_and(|&p| pb.contains(p)) || b.first().is_some_and(|&p| pa.contains(p))
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let cross = |o: Point, p: Point, q: Point| (p - o).cross(q - o);
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0) && d1 != 0.0 && d2 != 0.0 && d3 != 0.0 && d4 != 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples;

    #[test]
    fn stages_are_in_order() {
        assert_eq!(Stage::Pattern.number(), 1);
        assert_eq!(Stage::Pattern.next(), Some(Stage::Fabrics));
        assert_eq!(Stage::Simulation.next(), None);
    }

    /// Los ejemplos están bien hechos: ninguna etapa tiene nada pendiente.
    #[test]
    fn examples_pass_every_check() {
        for (name, _, make) in examples::ALL {
            let found = checks(&make(), &SceneReport::default());
            assert!(found.is_empty(), "{name}: {found:?}");
        }
    }

    #[test]
    fn empty_pattern_asks_for_pieces() {
        let found = checks(&Pattern::new("t"), &SceneReport::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].stage, Stage::Pattern);
    }

    #[test]
    fn unsewn_and_overlapping_pieces() {
        let mut p = examples::tote_bag();
        // Un bolsillo encima del delantero, en el mismo lado y sin coser.
        let pocket = p.add_rect("Bolsillo", Point::new(100.0, 100.0), 150.0, 150.0);
        let found = checks(&p, &SceneReport::default());
        let stages: Vec<Stage> = found.iter().map(|c| c.stage).collect();
        assert_eq!(stages, [Stage::Assembly, Stage::Placement], "{found:?}");
        assert!(found.iter().all(|c| c.target == Some(Selection::Piece(pocket))));
        // En otro lado ya no se solapa con nada.
        p.piece_mut(pocket).unwrap().arrangement = Arrangement::Left;
        let found = checks(&p, &SceneReport::default());
        assert_eq!(found.iter().map(|c| c.stage).collect::<Vec<_>>(), [Stage::Assembly]);
    }

    #[test]
    fn scene_problems_go_to_their_stage() {
        let p = examples::tote_bag();
        let report = SceneReport { failed: vec![p.pieces[0].id], crossed: vec![p.seams[0].id] };
        let found = checks(&p, &report);
        assert_eq!(found.iter().map(|c| c.stage).collect::<Vec<_>>(), [Stage::Assembly, Stage::Simulation]);
        // Lo que ya no está en el patrón no cuenta.
        let mut p = p;
        p.remove_seam(p.seams[0].id);
        p.remove_piece(p.pieces[0].id);
        let found = checks(&p, &report);
        assert!(!found.iter().any(|c| c.stage == Stage::Simulation || c.text.contains("retuerce")), "{found:?}");
    }
}
