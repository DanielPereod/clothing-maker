//! Patrones de ejemplo para probar el editor.

use pattern::{EdgeKind, Pattern, PieceId, Point, SeamSide, Vec2};

/// Camiseta básica: delantero y espalda con escote y sisas curvas, unidos
/// por hombros y costados.
pub fn t_shirt() -> Pattern {
    let mut p = Pattern::new("Camiseta básica");
    let front = t_shirt_panel(&mut p, "Delantero", Point::ZERO, 95.0);
    let back = t_shirt_panel(&mut p, "Espalda", Point::new(600.0, 0.0), 25.0);
    for (name, edge) in [("Hombro dcho.", 1), ("Costado dcho.", 3), ("Costado izdo.", 5), ("Hombro izdo.", 7)] {
        let side = |id: PieceId| SeamSide { piece: id, edge: p.piece(id).unwrap().edges()[edge].id, reversed: false };
        let sides = vec![side(front), side(back)];
        p.add_seam(name, sides);
    }
    p
}

fn t_shirt_panel(p: &mut Pattern, name: &str, origin: Point, neck_depth: f64) -> PieceId {
    let o = origin.to_vec2();
    let pts = [
        (180.0, 0.0),
        (320.0, 0.0),
        (470.0, 40.0),
        (500.0, 230.0),
        (500.0, 700.0),
        (0.0, 700.0),
        (0.0, 230.0),
        (30.0, 40.0),
    ]
    .map(|(x, y)| Point::new(x, y) + o);
    let id = p.add_polygon(name, pts.to_vec());
    let piece = p.piece_mut(id).unwrap();
    // Escote.
    piece.set_edge_kind(0, EdgeKind::Cubic { h1: Vec2::new(5.0, neck_depth), h2: Vec2::new(-5.0, neck_depth) });
    // Sisas.
    piece.set_edge_kind(2, EdgeKind::Cubic { h1: Vec2::new(-20.0, 80.0), h2: Vec2::new(-40.0, -10.0) });
    piece.set_edge_kind(6, EdgeKind::Cubic { h1: Vec2::new(40.0, -10.0), h2: Vec2::new(20.0, 80.0) });
    id
}

/// Bolsa tote: dos paneles y dos asas. Un objeto sin avatar.
pub fn tote_bag() -> Pattern {
    let mut p = Pattern::new("Bolsa tote");
    let front = p.add_rect("Delantero", Point::ZERO, 380.0, 420.0);
    let back = p.add_rect("Trasero", Point::new(450.0, 0.0), 380.0, 420.0);
    p.add_rect("Asa 1", Point::new(0.0, 500.0), 600.0, 60.0);
    p.add_rect("Asa 2", Point::new(0.0, 600.0), 600.0, 60.0);
    // Aristas del rectángulo: 0 arriba, 1 derecha, 2 abajo, 3 izquierda.
    let e = |p: &Pattern, id: PieceId, i: usize| p.piece(id).unwrap().edges()[i].id;
    for (name, a, b) in [("Fondo", 2, 2), ("Lateral dcho.", 1, 3), ("Lateral izdo.", 3, 1)] {
        let sides = vec![
            SeamSide { piece: front, edge: e(&p, front, a), reversed: false },
            SeamSide { piece: back, edge: e(&p, back, b), reversed: true },
        ];
        p.add_seam(name, sides);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_are_valid() {
        for p in [t_shirt(), tote_bag()] {
            assert!(p.validate().is_empty(), "{}: {:?}", p.name, p.validate());
            for piece in &p.pieces {
                mesher::mesh_piece(piece, &Default::default()).unwrap();
            }
        }
    }
}
