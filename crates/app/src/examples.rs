//! Patrones de ejemplo para probar el editor.
//!
//! Las piezas se dibujan vistas desde fuera (con el derecho de la tela hacia
//! quien mira), así que dos piezas vecinas recorren su borde común en
//! sentidos contrarios y uno de los dos lados de cada costura va invertido.

use pattern::{
    Arrangement, BodyZone, EdgeKind, Notch, Pattern, PieceId, Point, SeamFit, SeamKind, SeamSide, Textile, TextileId,
    Vec2,
};

/// Camiseta básica: delantero y espalda con escote y sisas curvas, unidos
/// por hombros y costados.
pub fn t_shirt() -> Pattern {
    let mut p = Pattern::new("Camiseta básica");
    p.on_avatar = true;
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

/// Camiseta con mangas: la básica más dos mangas cortas, cada una colocada
/// en su brazo del maniquí.
pub fn t_shirt_with_sleeves() -> Pattern {
    let mut p = t_shirt();
    p.name = "Camiseta con mangas".into();
    let (front, back) = (p.pieces[0].id, p.pieces[1].id);
    // Sisas: aristas 2 (lado +x del dibujo) y 6. Del delantero, la 2 cae en
    // el brazo izquierdo del maniquí; de la espalda (vista desde detrás), en
    // el derecho.
    let armhole = p.piece(front).unwrap().edge_length(2);
    for (name, x, zone, front_edge, back_edge) in
        [("Manga dcha.", 0.0, BodyZone::RightArm, 6, 2), ("Manga izda.", 450.0, BodyZone::LeftArm, 2, 6)]
    {
        let sleeve = sleeve(&mut p, name, Point::new(x, 800.0), armhole);
        p.piece_mut(sleeve).unwrap().zone = zone;
        // La mitad derecha del dibujo de la copa (arista 1) queda detrás en
        // el brazo izquierdo y delante en el derecho.
        let (to_front, to_back) = if zone == BodyZone::LeftArm { (0, 1) } else { (1, 0) };
        sew(&mut p, &format!("Copa delantera ({name})"), &[(front, front_edge, false), (sleeve, to_front, true)]);
        sew(&mut p, &format!("Copa trasera ({name})"), &[(back, back_edge, false), (sleeve, to_back, true)]);
        sew(&mut p, &format!("Bajo brazo ({name})"), &[(sleeve, 2, false), (sleeve, 4, true)]);
    }
    p
}

/// Manga corta: copa en dos mitades (aristas 0 y 1) del largo de la sisa,
/// costados bajo el brazo (2 y 4) y bajo (3). La altura de la copa se
/// ajusta para que cada mitad mida `armhole`.
fn sleeve(p: &mut Pattern, name: &str, origin: Point, armhole: f64) -> PieceId {
    const WIDTH: f64 = 360.0;
    const LENGTH: f64 = 120.0;
    const TAPER: f64 = 20.0;
    let shape = |cap: f64| {
        let pts = [(0.0, cap), (WIDTH / 2.0, 0.0), (WIDTH, cap), (WIDTH - TAPER, cap + LENGTH), (TAPER, cap + LENGTH)]
            .map(|(x, y)| origin + Vec2::new(x, y));
        let mut piece = pattern::Piece::from_parts(
            PieceId(0),
            name,
            pts.to_vec(),
            (0..5).map(|i| pattern::Edge::line(pattern::EdgeId(i))).collect(),
        )
        .unwrap();
        piece.set_edge_kind(0, EdgeKind::Cubic { h1: Vec2::new(WIDTH * 0.15, 0.0), h2: Vec2::new(-WIDTH * 0.2, 0.0) });
        piece.set_edge_kind(1, EdgeKind::Cubic { h1: Vec2::new(WIDTH * 0.2, 0.0), h2: Vec2::new(-WIDTH * 0.15, 0.0) });
        (pts, piece)
    };
    // Bisección: la mitad de la copa crece con su altura.
    let (mut lo, mut hi) = (0.0, armhole);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if shape(mid).1.edge_length(0) < armhole { lo = mid } else { hi = mid }
    }
    let (pts, template) = shape((lo + hi) / 2.0);
    let id = p.add_polygon(name, pts.to_vec());
    let piece = p.piece_mut(id).unwrap();
    for i in 0..2 {
        piece.set_edge_kind(i, template.edges()[i].kind);
    }
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

/// Bolsa tote con forro aparte: el exterior de lona y un forro de algodón
/// cosidos como dos bolsas, una dentro de otra, unidos por la boca, y un
/// bolsillo de parche por dentro, cosido a tres líneas del forro delantero.
/// El forro va en la capa 1 (dentro) y con el derecho hacia dentro.
pub fn lined_tote_bag() -> Pattern {
    let (w, h) = (380.0, 420.0);
    let mut p = Pattern::new("Bolsa tote con forro");
    let named = |p: &Pattern, name: &str| p.textiles.iter().find(|t| t.name == name).expect("tela genérica").id;
    let canvas = named(&p, "Lona");
    let front = p.add_rect("Delantero", Point::ZERO, w, h);
    let back = p.add_rect("Trasero", Point::new(w + 70.0, 0.0), w, h);
    // El forro, algo más estrecho abajo y más corto: dentro del exterior
    // tiene menos sitio (la tela tiene grosor), y así sus dobleces de los
    // lados y del fondo no caen justo encima de los del exterior.
    let (taper, lh) = (10.0, h - 10.0);
    let lining = |p: &mut Pattern, name: &str, x: f64| {
        let pts = [(0.0, 0.0), (w, 0.0), (w - taper, lh), (taper, lh)];
        p.add_polygon(name, pts.map(|(px, py)| Point::new(x + px, h + 80.0 + py)).to_vec())
    };
    let lining_front = lining(&mut p, "Forro delantero", 0.0);
    let lining_back = lining(&mut p, "Forro trasero", w + 70.0);
    for id in [front, back] {
        p.piece_mut(id).unwrap().fabric = canvas;
    }
    for id in [back, lining_back] {
        p.piece_mut(id).unwrap().arrangement = Arrangement::Back;
    }
    for id in [lining_front, lining_back] {
        let piece = p.piece_mut(id).unwrap();
        piece.layer = 1;
        piece.right_side_in = true;
    }
    for (outer, inner, name) in [(front, back, ""), (lining_front, lining_back, " del forro")] {
        sew(&mut p, &format!("Fondo{name}"), &[(outer, BOTTOM, false), (inner, BOTTOM, true)]);
        sew(&mut p, &format!("Lateral dcho.{name}"), &[(outer, RIGHT, false), (inner, LEFT, true)]);
        sew(&mut p, &format!("Lateral izdo.{name}"), &[(outer, LEFT, false), (inner, RIGHT, true)]);
    }
    // La boca: cada pieza del exterior con su forro, embolsadas.
    sew(&mut p, "Boca delantera", &[(front, TOP, false), (lining_front, TOP, false)]);
    sew(&mut p, "Boca trasera", &[(back, TOP, false), (lining_back, TOP, false)]);
    for seam in p.seams.iter_mut().filter(|s| s.name.starts_with("Boca")) {
        seam.set_kind(SeamKind::Bagged);
    }

    // Bolsillo por dentro del forro delantero: sus lados y su fondo van
    // cosidos a una U de líneas interiores; la boca queda abierta.
    let (x, y, pw, ph) = (110.0, h + 80.0 + 110.0, 160.0, 160.0);
    let corner = |dx: f64, dy: f64| Point::new(x + dx, y + dy);
    let left = p.add_line(lining_front, corner(0.0, 0.0), corner(0.0, ph)).unwrap();
    let bottom = p.add_line(lining_front, corner(0.0, ph), corner(pw, ph)).unwrap();
    let right = p.add_line(lining_front, corner(pw, ph), corner(pw, 0.0)).unwrap();
    let pocket = p.add_rect("Bolsillo", Point::new(2.0 * w + 140.0, h + 80.0), pw, ph);
    let piece = p.piece_mut(pocket).unwrap();
    piece.layer = 1;
    piece.right_side_in = true;
    let edge = |p: &Pattern, i: usize| p.piece(pocket).unwrap().edges()[i].id;
    for (name, i, line) in [("izdo.", LEFT, left), ("fondo", BOTTOM, bottom), ("dcho.", RIGHT, right)] {
        let sides = vec![SeamSide::new(pocket, edge(&p, i), true), SeamSide::new(lining_front, line, false)];
        p.add_seam(format!("Bolsillo {name}"), sides).expect("costura válida");
    }
    p
}

/// Cojín: dos cuadrados cosidos por los cuatro lados y con relleno, para
/// ver la presión interior.
pub fn cushion() -> Pattern {
    let mut p = Pattern::new("Cojín");
    p.pressure = 30.0;
    let front = p.add_rect("Delantero", Point::ZERO, 400.0, 400.0);
    let back = p.add_rect("Trasero", Point::new(450.0, 0.0), 400.0, 400.0);
    p.piece_mut(back).unwrap().arrangement = Arrangement::Back;
    sew(&mut p, "Arriba", &[(front, TOP, false), (back, TOP, true)]);
    sew(&mut p, "Lado dcho.", &[(front, RIGHT, false), (back, LEFT, true)]);
    sew(&mut p, "Abajo", &[(front, BOTTOM, false), (back, BOTTOM, true)]);
    sew(&mut p, "Lado izdo.", &[(front, LEFT, false), (back, RIGHT, true)]);
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

/// Bolso de cuero con forro de lino: la misma caja que la bolsa con fondo,
/// pero cada pieza lleva dos capas que suman su firmeza, así que se sostiene
/// casi sola.
pub fn leather_bag() -> Pattern {
    let mut p = box_bag();
    p.name = "Bolso de cuero".into();
    let named = |p: &Pattern, name: &str| p.textiles.iter().find(|t| t.name == name).expect("tela genérica").id;
    let (leather, linen) = (named(&p, "Cuero"), named(&p, "Lino"));
    for piece in &mut p.pieces {
        piece.fabric = leather;
        piece.layers = vec![linen];
    }
    p
}

/// Chalk bag (bolsa de magnesio de escalada), del patrón de Christian
/// Halling. Las piezas van dibujadas por la línea de costura, con las
/// medidas del patrón menos su margen de 3/8" (9,5 mm), que es el margen
/// del patrón.
///
/// El cuerpo (1.1 de tela decorativa y 1.2 de lona de refuerzo, cortadas
/// iguales y al doblez) es una sola pieza cerrada por un costado; el forro
/// de pelo (3 y 4) va como capa suya y de la base redonda (2). La base se
/// cose en una sola costura que da la vuelta a sus cuatro aristas. El puño
/// de punto (5) se dobla por la mitad, así que lleva dos capas de punto, y
/// es más corto que la boca: se cose estirado. Los piquetes casan la base y
/// el puño con el cuerpo, y la línea de montaje cose primero los costados,
/// luego la base y por último el puño.
pub fn chalk_bag() -> Pattern {
    const SEAM_ALLOWANCE: f64 = 9.5;
    // El cuerpo mide 400 × 152,5 mm con margen (200 al doblez); la base,
    // 140 mm de diámetro; el puño, 360 × 152,5 mm antes de doblarlo.
    let (w, h) = (400.0 - 2.0 * SEAM_ALLOWANCE, 152.5 - 2.0 * SEAM_ALLOWANCE);
    let radius = (140.0 - 2.0 * SEAM_ALLOWANCE) / 2.0;
    let (cuff_w, cuff) = (360.0 - 2.0 * SEAM_ALLOWANCE, (152.5 - 2.0 * SEAM_ALLOWANCE) / 2.0);

    let mut p = Pattern::new("Chalk bag");
    p.seam_allowance = SEAM_ALLOWANCE;
    let fur = TextileId(p.textiles.iter().map(|t| t.id.0).max().unwrap_or(0) + 1);
    p.textiles.push(Textile {
        id: fur,
        name: "Pelo sintético".into(),
        firmness: 30.0,
        weight: 300.0,
        stretch: 40.0,
        friction: 0.6,
        thickness: 6.0,
    });
    let named = |p: &Pattern, name: &str| p.textiles.iter().find(|t| t.name == name).expect("tela genérica").id;
    let (cotton, canvas, knit) = (named(&p, "Algodón"), named(&p, "Lona"), named(&p, "Punto"));

    let body = p.add_rect("Cuerpo (1)", Point::new(0.0, cuff + 20.0), w, h);
    let cuff_piece = p.add_rect("Puño (5)", Point::new((w - cuff_w) / 2.0, 0.0), cuff_w, cuff);

    // Base: un círculo de cuatro cuartos. Las aristas 3 y 0 (la mitad de
    // arriba del dibujo) caen delante.
    let c = Point::new(w / 2.0, cuff + h + 40.0 + radius);
    let pts = [(0.0, -radius), (radius, 0.0), (0.0, radius), (-radius, 0.0)].map(|(x, y)| c + Vec2::new(x, y));
    let base = p.add_polygon("Base (2)", pts.to_vec());
    let piece = p.piece_mut(base).unwrap();
    piece.arrangement = Arrangement::Bottom;
    // Manejadores de una Bézier que aproxima un cuarto de círculo.
    let k = radius * 0.552_284_75;
    let quarter = |from: Point| {
        // Tangente en el sentido del contorno (horario en pantalla).
        let to_center = c - from;
        Vec2::new(to_center.y, -to_center.x).normalize() * k
    };
    for i in 0..4 {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        piece.set_edge_kind(i, EdgeKind::Cubic { h1: quarter(a), h2: -quarter(b) });
    }
    // Piquetes en el centro de cada cuarto de la base y en los octavos del
    // fondo del cuerpo, que caen en los mismos sitios al coserlos.
    for i in 0..4 {
        let edge = piece.edges()[i].id;
        piece.notches.push(Notch { edge, t: 0.5 });
    }
    let piece = p.piece_mut(body).unwrap();
    let bottom = piece.edges()[BOTTOM].id;
    let top = piece.edges()[TOP].id;
    piece.notches.extend([0.125, 0.375, 0.625, 0.875].map(|t| Notch { edge: bottom, t }));
    // Puño y boca, en los cuartos: el punto se estira igual por todo.
    piece.notches.extend([0.25, 0.5, 0.75].map(|t| Notch { edge: top, t }));
    let piece = p.piece_mut(cuff_piece).unwrap();
    let cuff_bottom = piece.edges()[BOTTOM].id;
    piece.notches.extend([0.25, 0.5, 0.75].map(|t| Notch { edge: cuff_bottom, t }));

    for id in [body, base] {
        let piece = p.piece_mut(id).unwrap();
        piece.fabric = cotton;
        piece.layers = vec![canvas, fur];
    }
    let piece = p.piece_mut(cuff_piece).unwrap();
    piece.fabric = knit;
    piece.layers = vec![knit];

    let sides = p.add_step("Costados");
    let bottom_step = p.add_step("Base");
    let cuff_step = p.add_step("Puño");
    sew(&mut p, "Costado", &[(body, RIGHT, false), (body, LEFT, true)]);
    sew(&mut p, "Costado del puño", &[(cuff_piece, RIGHT, false), (cuff_piece, LEFT, true)]);
    // La base, de atrás (el vértice 2) a atrás dando toda la vuelta: sus
    // cuatro aristas son un solo lado.
    let base_edges: Vec<_> = p.piece(base).unwrap().edges().iter().map(|e| e.id).collect();
    let around = SeamSide::new(base, base_edges[2], true).through(base_edges[1]);
    p.add_seam("Base", vec![SeamSide::new(body, bottom, false), around]).expect("costura válida");
    sew(&mut p, "Puño", &[(cuff_piece, BOTTOM, false), (body, TOP, true)]);
    for seam in &mut p.seams {
        seam.step = Some(match seam.name.as_str() {
            "Base" => bottom_step,
            "Puño" => cuff_step,
            _ => sides,
        });
        // El punto se cose con remalladora, y el puño, estirado hasta la boca.
        if seam.name.contains("uño") {
            seam.set_kind(SeamKind::Overlocked);
        }
        if seam.name == "Puño" {
            seam.fit = SeamFit::Stretched;
        }
    }
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
pub const ALL: [Example; 8] = [
    ("Camiseta básica", "camiseta-basica", t_shirt),
    ("Camiseta con mangas", "camiseta-con-mangas", t_shirt_with_sleeves),
    ("Bolsa tote", "bolsa-tote", tote_bag),
    ("Bolsa tote con forro", "bolsa-tote-con-forro", lined_tote_bag),
    ("Bolsa con fondo", "bolsa-con-fondo", box_bag),
    ("Bolso de cuero con forro", "bolso-de-cuero", leather_bag),
    ("Cojín", "cojin", cushion),
    ("Chalk bag de escalada", "chalk-bag", chalk_bag),
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
