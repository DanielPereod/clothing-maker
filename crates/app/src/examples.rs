//! Patrones de ejemplo para probar el editor.
//!
//! Las piezas se dibujan vistas desde fuera (con el derecho de la tela hacia
//! quien mira), así que dos piezas vecinas recorren su borde común en
//! sentidos contrarios y uno de los dos lados de cada costura va invertido.

use pattern::{Arrangement, EdgeKind, Pattern, PieceId, Point, SeamSide, Vec2};

/// Camiseta básica: delantero y espalda con escote y sisas curvas, unidos
/// por hombros y costados.
pub fn t_shirt() -> Pattern {
    let mut p = Pattern::new("Camiseta básica");
    let front = t_shirt_panel(&mut p, "Delantero", Point::ZERO, 95.0);
    let back = t_shirt_panel(&mut p, "Espalda", Point::new(600.0, 0.0), 25.0);
    p.piece_mut(back).unwrap().arrangement = Arrangement::Back;
    // Aristas del panel: 1 y 7 hombros, 3 y 5 costados. La espalda se ve
    // desde detrás, así que su lado derecho casa con el izquierdo del
    // delantero.
    for (name, a, b) in
        [("Hombro dcho.", 1, 7), ("Costado dcho.", 3, 5), ("Costado izdo.", 5, 3), ("Hombro izdo.", 7, 1)]
    {
        sew(&mut p, name, &[(front, a, false), (back, b, true)]);
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

// Aristas de un rectángulo: 0 arriba, 1 derecha, 2 abajo, 3 izquierda.
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// Bolsa tote: dos paneles cosidos por el fondo y los laterales. Un objeto
/// sin avatar.
pub fn tote_bag() -> Pattern {
    let mut p = Pattern::new("Bolsa tote");
    let front = p.add_rect("Delantero", Point::ZERO, 380.0, 420.0);
    let back = p.add_rect("Trasero", Point::new(450.0, 0.0), 380.0, 420.0);
    p.piece_mut(back).unwrap().arrangement = Arrangement::Back;
    sew(&mut p, "Fondo", &[(front, BOTTOM, false), (back, BOTTOM, true)]);
    sew(&mut p, "Lateral dcho.", &[(front, RIGHT, false), (back, LEFT, true)]);
    sew(&mut p, "Lateral izdo.", &[(front, LEFT, false), (back, RIGHT, true)]);
    p
}

/// Bolsa con fondo: cuatro paredes y una base, como una caja de tela.
/// Sirve para probar la colocación a los lados y debajo.
pub fn box_bag() -> Pattern {
    let (w, d, h) = (380.0, 120.0, 300.0);
    let mut p = Pattern::new("Bolsa con fondo");
    let front = p.add_rect("Delantero", Point::new(d + 30.0, 0.0), w, h);
    let right = p.add_rect("Lateral dcho.", Point::new(w + d + 60.0, 0.0), d, h);
    let back = p.add_rect("Trasero", Point::new(w + 2.0 * d + 90.0, 0.0), w, h);
    let left = p.add_rect("Lateral izdo.", Point::ZERO, d, h);
    let base = p.add_rect("Base", Point::new(d + 30.0, h + 40.0), w, d);
    for (id, a) in
        [(right, Arrangement::Right), (back, Arrangement::Back), (left, Arrangement::Left), (base, Arrangement::Bottom)]
    {
        p.piece_mut(id).unwrap().arrangement = a;
    }
    // Esquinas verticales, dando la vuelta: delantero → derecho → trasero → izquierdo.
    sew(&mut p, "Esquina delantera dcha.", &[(front, RIGHT, false), (right, LEFT, true)]);
    sew(&mut p, "Esquina trasera dcha.", &[(right, RIGHT, false), (back, LEFT, true)]);
    sew(&mut p, "Esquina trasera izda.", &[(back, RIGHT, false), (left, LEFT, true)]);
    sew(&mut p, "Esquina delantera izda.", &[(left, RIGHT, false), (front, LEFT, true)]);
    // La base se ve desde abajo, con su borde de arriba hacia delante.
    sew(&mut p, "Base delantera", &[(front, BOTTOM, false), (base, TOP, true)]);
    sew(&mut p, "Base dcha.", &[(right, BOTTOM, false), (base, RIGHT, true)]);
    sew(&mut p, "Base trasera", &[(back, BOTTOM, false), (base, BOTTOM, true)]);
    sew(&mut p, "Base izda.", &[(left, BOTTOM, false), (base, LEFT, true)]);
    p
}

/// Cose las aristas dadas por (pieza, índice de arista, invertida).
fn sew(p: &mut Pattern, name: &str, sides: &[(PieceId, usize, bool)]) {
    let sides = sides
        .iter()
        .map(|&(id, i, reversed)| SeamSide::new(id, p.piece(id).unwrap().edges()[i].id, reversed))
        .collect();
    p.add_seam(name, sides).expect("costura válida");
}

/// Un ejemplo: nombre para el menú, fichero en `examples/` y cómo se crea.
pub type Example = (&'static str, &'static str, fn() -> Pattern);

/// Todos los ejemplos.
pub const ALL: [Example; 3] = [
    ("Camiseta básica", "camiseta-basica", t_shirt),
    ("Bolsa tote", "bolsa-tote", tote_bag),
    ("Bolsa con fondo", "bolsa-con-fondo", box_bag),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_are_valid() {
        for (_, _, make) in ALL {
            let p = make();
            assert!(p.validate().is_empty(), "{}: {:?}", p.name, p.validate());
            for piece in &p.pieces {
                mesher::mesh_piece(piece, &Default::default()).unwrap();
            }
        }
    }

    /// Los ficheros de `examples/` son los mismos patrones que el menú.
    /// Con `ACTUALIZAR_EJEMPLOS=1` se vuelven a escribir.
    #[test]
    fn example_files_are_up_to_date() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        for (_, file, make) in ALL {
            let path = dir.join(format!("{file}.pattern.json"));
            let json = make().to_json().unwrap() + "\n";
            if std::env::var_os("ACTUALIZAR_EJEMPLOS").is_some() {
                std::fs::write(&path, &json).unwrap();
            }
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(on_disk == json, "{} está desfasado: ACTUALIZAR_EJEMPLOS=1 cargo test -p app", path.display());
        }
    }
}
