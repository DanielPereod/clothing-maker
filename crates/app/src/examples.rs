//! Patrones de ejemplo para probar el editor.
//!
//! Las piezas se dibujan vistas desde fuera (con el derecho de la tela hacia
//! quien mira), así que dos piezas vecinas recorren su borde común en
//! sentidos contrarios y uno de los dos lados de cada costura va invertido.

use pattern::{
    Arrangement, BodyZone, EdgeKind, Notch, Pattern, PieceId, Point, SeamFit, SeamKind, SeamSide, StepId, Textile,
    TextileId, Vec2,
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

/// Bolso con cremallera y asas: dos paneles de lona cosidos por el fondo y
/// los lados, una cremallera en la boca (sus dos cintas, cosidas cada una a
/// un panel, y los dientes, que se abren desde la izquierda del delantero
/// con «Abierta» en la costura «Boca») y un asa de cincha en cada
/// panel, cosida por sus extremos a dos líneas interiores. Como al coserlo
/// de verdad, primero van las asas y la cremallera sobre los paneles
/// planos, y al final se cierra la bolsa.
pub fn zipper_bag() -> Pattern {
    let (w, h) = (360.0, 280.0);
    let mut p = Pattern::new("Bolso con cremallera y asas");
    let canvas = p.textiles.iter().find(|t| t.name == "Lona").expect("tela genérica").id;
    let front = p.add_rect("Delantero", Point::ZERO, w, h);
    let back = p.add_rect("Trasero", Point::new(w + 70.0, 0.0), w, h);
    p.piece_mut(back).unwrap().arrangement = Arrangement::Back;
    for id in [front, back] {
        p.piece_mut(id).unwrap().fabric = canvas;
    }
    let handles = p.add_step("Asas");
    let zipper = p.add_step("Cremallera");
    let body = p.add_step("Bolsa");
    sew(&mut p, "Fondo", &[(front, BOTTOM, false), (back, BOTTOM, true)]);
    sew(&mut p, "Lado dcho.", &[(front, RIGHT, false), (back, LEFT, true)]);
    sew(&mut p, "Lado izdo.", &[(front, LEFT, false), (back, RIGHT, true)]);
    for seam in &mut p.seams {
        seam.step = Some(body);
    }
    sew(&mut p, "Boca", &[(front, TOP, false), (back, TOP, true)]);
    let mouth = p.seams.last().unwrap().id;
    p.seam_mut(mouth).unwrap().step = Some(zipper);
    p.zip_seam(mouth, pattern::ZIPPER_TAPE_WIDTH).expect("la boca tiene dos lados");

    add_handles(&mut p, [(front, "delantera"), (back, "trasera")], Point::new(0.0, h + 60.0), handles);
    p
}

/// Un asa de cincha en cada panel, cortas, de mano: cada extremo sobre una
/// línea vertical cerca de la boca; el asa sale de una hacia la otra y
/// empieza en arco, fuera del panel. Las asas sueltas, en el patrón, desde
/// `origin` hacia la derecha.
fn add_handles(p: &mut Pattern, panels: [(PieceId, &str); 2], origin: Point, step: StepId) {
    let (strap, width, gap) = (300.0, 25.0, 200.0);
    for (k, (panel, name)) in panels.into_iter().enumerate() {
        let bounds = p.piece(panel).unwrap().bounding_box();
        let (x0, y0, w) = (bounds.x0, bounds.y0 + 30.0, bounds.width());
        let lines = [(w - gap) / 2.0, (w + gap) / 2.0]
            .map(|x| p.add_line(panel, Point::new(x0 + x, y0), Point::new(x0 + x, y0 + width)).unwrap());
        let origin = origin + Vec2::new(k as f64 * (strap + 40.0), 0.0);
        let handle = p.add_strap(pattern::Notion::Webbing, strap, width, origin);
        p.piece_mut(handle).unwrap().name = format!("Asa {name}");
        let edge = |p: &Pattern, i: usize| p.piece(handle).unwrap().edges()[i].id;
        // Los extremos, de arriba abajo como las líneas.
        for (end, i, reversed, line) in [("izdo.", LEFT, true, lines[0]), ("dcho.", RIGHT, false, lines[1])] {
            let sides = vec![SeamSide::new(handle, edge(p, i), reversed), SeamSide::new(panel, line, false)];
            let id = p.add_seam(format!("Asa {name} ({end})"), sides).expect("costura válida");
            let seam = p.seam_mut(id).unwrap();
            seam.set_kind(SeamKind::Lapped);
            seam.step = Some(step);
        }
    }
}

/// Bolso con forro aparte, bolsillo y cremallera, como se cose de verdad:
///
/// - Exterior de lona: delantero, trasero y una base que da fondo al bolso.
/// - Forro de algodón con las mismas piezas, algo menores, en la capa de
///   dentro y con el derecho hacia dentro.
/// - Un bolsillo de parche cosido por tres lados a una U de líneas del
///   forro delantero.
/// - Una cremallera en la boca: cada cinta, cosida entre la boca del
///   exterior y la del forro, en la misma costura. Se abre desde la
///   izquierda del delantero con «Abierta» en la costura «Boca».
/// - Un asa de cincha en cada panel del exterior.
pub fn lined_zipper_bag() -> Pattern {
    let (w, h, d) = (360.0, 280.0, 100.0);
    let mut p = Pattern::new("Bolso con forro, bolsillo y cremallera");
    let named = |p: &Pattern, name: &str| p.textiles.iter().find(|t| t.name == name).expect("tela genérica").id;
    let canvas = named(&p, "Lona");
    // La base, de largo `w - d` y ancho `d`, con un vértice en el centro de
    // cada lado corto: el bajo de cada panel se cose a media vuelta de ella
    // (su lado largo y medio lado corto a cada lado), y los costados del
    // bolso caen en el centro de los lados cortos.
    let base_piece = |p: &mut Pattern, name: &str, origin: Point, w: f64, d: f64| {
        let l = w - d;
        let pts = [(0.0, 0.0), (l, 0.0), (l, d / 2.0), (l, d), (0.0, d), (0.0, d / 2.0)];
        let id = p.add_polygon(name, pts.map(|(x, y)| origin + Vec2::new(x, y)).to_vec());
        p.piece_mut(id).unwrap().arrangement = Arrangement::Bottom;
        id
    };
    let front = p.add_rect("Delantero", Point::ZERO, w, h);
    let back = p.add_rect("Trasero", Point::new(w + 70.0, 0.0), w, h);
    let base = base_piece(&mut p, "Base", Point::new(2.0 * w + 140.0, 0.0), w, d);
    // El forro, algo más corto y más estrecho que el exterior para que
    // quepa dentro: tan ancho como la cremallera arriba, pero sus costados
    // se cosen con más margen justo debajo (`inset` mm más dentro) y aún más
    // abajo (`taper`). Sus aristas: 0 la boca, 1 y 2 el costado derecho, 3
    // el bajo y 4 y 5 el izquierdo.
    let (inset, drop, taper, lh, ly) = (6.0, 25.0, 12.0, h - 10.0, h + 80.0);
    let lining = |p: &mut Pattern, name: &str, x: f64| {
        let pts = [(0.0, 0.0), (w, 0.0), (w - inset, drop), (w - taper, lh), (taper, lh), (inset, drop)];
        p.add_polygon(name, pts.map(|(px, py)| Point::new(x + px, ly + py)).to_vec())
    };
    let lining_front = lining(&mut p, "Forro delantero", 0.0);
    let lining_back = lining(&mut p, "Forro trasero", w + 70.0);
    let lining_d = d - 8.0;
    let lining_base = base_piece(&mut p, "Base del forro", Point::new(2.0 * w + 140.0, ly), w - 2.0 * taper, lining_d);
    for id in [front, back, base] {
        p.piece_mut(id).unwrap().fabric = canvas;
    }
    for id in [back, lining_back] {
        p.piece_mut(id).unwrap().arrangement = Arrangement::Back;
    }
    for id in [lining_front, lining_back, lining_base] {
        let piece = p.piece_mut(id).unwrap();
        piece.layer = 1;
        piece.right_side_in = true;
    }

    let handles = p.add_step("Asas");
    let pocket_step = p.add_step("Bolsillo");
    let zipper = p.add_step("Cremallera");
    // El forro se cierra antes que el exterior: así el exterior se cierra
    // por fuera de él, y no se mete por sus costados aún abiertos.
    let inside = p.add_step("Forro");
    let outside = p.add_step("Exterior");

    // Costados y base de cada bolsa. La base se ve desde abajo, con su lado
    // largo de arriba hacia delante: el bajo del delantero casa con ella de
    // su vértice 2 al 5 por arriba, y el del trasero, del 5 al 2 por abajo
    // (los dos, al revés de su contorno).
    // Lados de cada panel por (primera, última arista): derecho, izquierdo
    // y bajo.
    let panel_sides = [((RIGHT, RIGHT), (LEFT, LEFT), BOTTOM), ((1, 2), (4, 5), 3)];
    for ((outer, inner, bottom, name, step), (right, left, hem)) in
        [(front, back, base, "", outside), (lining_front, lining_back, lining_base, " del forro", inside)]
            .into_iter()
            .zip(panel_sides)
    {
        let first = p.seams.len();
        let side = |p: &Pattern, id: PieceId, (from, to): (usize, usize), reversed: bool| {
            let edges = p.piece(id).unwrap().edges();
            SeamSide::new(id, edges[from].id, reversed).through(edges[to].id)
        };
        let sides = vec![side(&p, outer, right, false), side(&p, inner, left, true)];
        p.add_seam(format!("Lado dcho.{name}"), sides).expect("costura válida");
        let sides = vec![side(&p, outer, left, false), side(&p, inner, right, true)];
        p.add_seam(format!("Lado izdo.{name}"), sides).expect("costura válida");
        let edges: Vec<_> = p.piece(bottom).unwrap().edges().iter().map(|e| e.id).collect();
        for (panel, from, to, side) in [(outer, 5, 1, "delantera"), (inner, 2, 4, "trasera")] {
            let hem = p.piece(panel).unwrap().edges()[hem].id;
            let around = SeamSide::new(bottom, edges[from], true).through(edges[to]);
            p.add_seam(format!("Base {side}{name}"), vec![SeamSide::new(panel, hem, false), around])
                .expect("costura válida");
        }
        for seam in &mut p.seams[first..] {
            seam.step = Some(step);
        }
    }

    // La boca: una cinta por fuera de cada panel y, en la misma costura,
    // la boca del forro de ese lado, como el exterior.
    sew(&mut p, "Boca", &[(front, TOP, false), (back, TOP, true)]);
    let mouth = p.seams.last().unwrap().id;
    p.seam_mut(mouth).unwrap().step = Some(zipper);
    let parts = p.zip_seam(mouth, pattern::ZIPPER_TAPE_WIDTH).expect("la boca tiene dos lados");
    for (attach, lining, reversed) in [(parts.seams[0], lining_front, false), (parts.seams[1], lining_back, true)] {
        let top = p.piece(lining).unwrap().edges()[TOP].id;
        p.seam_mut(attach).unwrap().sides.push(SeamSide::new(lining, top, reversed));
    }

    // Bolsillo por dentro del forro delantero: sus lados y su fondo van
    // cosidos a una U de líneas interiores; la boca queda abierta.
    let (x, y, pw, ph) = (110.0, ly + 70.0, 140.0, 120.0);
    let corner = |dx: f64, dy: f64| Point::new(x + dx, y + dy);
    let left = p.add_line(lining_front, corner(0.0, 0.0), corner(0.0, ph)).unwrap();
    let bottom = p.add_line(lining_front, corner(0.0, ph), corner(pw, ph)).unwrap();
    let right = p.add_line(lining_front, corner(pw, ph), corner(pw, 0.0)).unwrap();
    let pocket = p.add_rect("Bolsillo", Point::new(2.0 * w + 140.0, ly + d + 40.0), pw, ph);
    let piece = p.piece_mut(pocket).unwrap();
    piece.layer = 1;
    piece.right_side_in = true;
    let edge = |p: &Pattern, i: usize| p.piece(pocket).unwrap().edges()[i].id;
    for (name, i, line) in [("izdo.", LEFT, left), ("fondo", BOTTOM, bottom), ("dcho.", RIGHT, right)] {
        let sides = vec![SeamSide::new(pocket, edge(&p, i), true), SeamSide::new(lining_front, line, false)];
        let id = p.add_seam(format!("Bolsillo {name}"), sides).expect("costura válida");
        p.seam_mut(id).unwrap().step = Some(pocket_step);
    }

    add_handles(&mut p, [(front, "delantera"), (back, "trasera")], Point::new(2.0 * w + 140.0, d + 40.0), handles);
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
/// Halling, sin simplificar. Las piezas van dibujadas por la línea de
/// costura: las medidas del patrón menos su margen de 3/8" (9,5 mm).
///
/// - Cuerpo (1.1 + 1.2, cortado al doblez): una pieza de 381 × 133,5 mm de
///   tela decorativa con lona pegada como entretela, cerrada por un
///   costado.
/// - Base (2): un círculo de 121 mm cosido al cuerpo en una sola costura
///   que da la vuelta a sus cuatro aristas.
/// - Forro de pelo (4.1 + 4.2 y su base 3): una bolsa aparte, de 341 mm de
///   contorno y 95,5 de alto, con el derecho hacia dentro y en la capa de
///   forro, dentro del cuerpo.
/// - Puño de punto (5.1 + 5.2): 341 × 133,5 mm, doblado por la mitad revés
///   con revés. Sus dos bordes se cosen a la boca junto con el cuerpo y el
///   forro, en una costura de cuatro lados, estirados de 341 a 381 mm.
///
/// Los piquetes casan las bases y la boca, y la línea de montaje cierra a
/// la vez los tres tubos (los costados del cuerpo, del puño y del forro, y
/// la base del forro), luego la boca y por último la base. Los tubos, a la
/// vez para que encojan juntos: con el forro cerrado antes, tira de la
/// boca del cuerpo y del puño, que esperan abiertos, y los inclina. Y la
/// boca antes que la base para que el puño, mientras espera, no tire de
/// lado del cuerpo ya cerrado por abajo y lo vuelque.
pub fn chalk_bag() -> Pattern {
    const SEAM_ALLOWANCE: f64 = 9.5;
    let less = |mm: f64| mm - 2.0 * SEAM_ALLOWANCE;
    // Con margen: cuerpo 400 × 152,5, base Ø 140, forro 360 × 114,5 y su
    // base Ø 127, puño 360 × 152,5 antes de doblarlo.
    let (w, h) = (less(400.0), less(152.5));
    let (lining_w, lining_h) = (less(360.0), less(114.5));
    let (cuff_w, cuff_h) = (less(360.0), less(152.5));

    let mut p = Pattern::new("Chalk bag");
    p.seam_allowance = SEAM_ALLOWANCE;
    // El pelo sintético lleva un fondo de punto: cede casi como el puño,
    // que mide lo mismo y se estira con él hasta la boca.
    let fur = TextileId(p.textiles.iter().map(|t| t.id.0).max().unwrap_or(0) + 1);
    p.textiles.push(Textile {
        id: fur,
        name: "Pelo sintético".into(),
        firmness: 30.0,
        weight: 300.0,
        stretch: 65.0,
        friction: 0.6,
        thickness: 4.0,
    });
    let named = |p: &Pattern, name: &str| p.textiles.iter().find(|t| t.name == name).expect("tela genérica").id;
    let (cotton, canvas, knit) = (named(&p, "Algodón"), named(&p, "Lona"), named(&p, "Punto"));

    // El puño, desplegado, encima del cuerpo; el forro, a su derecha.
    let cuff = p.add_rect("Puño (5)", Point::new((w - cuff_w) / 2.0, 0.0), cuff_w, cuff_h);
    let body = p.add_rect("Cuerpo (1)", Point::new(0.0, cuff_h + 20.0), w, h);
    let base = round_piece(&mut p, "Base (2)", Point::new(w / 2.0, cuff_h + h + 40.0 + less(140.0) / 2.0), less(140.0));
    let lining_x = w + 60.0;
    let lining = p.add_rect("Forro (4)", Point::new(lining_x, cuff_h + 20.0), lining_w, lining_h);
    let lining_base = round_piece(
        &mut p,
        "Base del forro (3)",
        Point::new(lining_x + lining_w / 2.0, cuff_h + lining_h + 40.0 + less(127.0) / 2.0),
        less(127.0),
    );
    for (id, fabric, layers) in [
        (body, cotton, vec![canvas]),
        (base, cotton, vec![canvas]),
        (lining, fur, vec![]),
        (lining_base, fur, vec![]),
        (cuff, knit, vec![]),
    ] {
        let piece = p.piece_mut(id).unwrap();
        piece.fabric = fabric;
        piece.layers = layers;
    }
    for id in [lining, lining_base] {
        let piece = p.piece_mut(id).unwrap();
        piece.layer = 1;
        piece.right_side_in = true;
    }

    // Piquetes: en los octavos del fondo de los cuerpos (las bases los
    // llevan en el centro de cada cuarto) y en los cuartos de la boca, para
    // que el punto se estire igual por todo.
    for id in [body, lining] {
        let piece = p.piece_mut(id).unwrap();
        let (bottom, top) = (piece.edges()[BOTTOM].id, piece.edges()[TOP].id);
        piece.notches.extend([0.125, 0.375, 0.625, 0.875].map(|t| Notch { edge: bottom, t }));
        piece.notches.extend([0.25, 0.5, 0.75].map(|t| Notch { edge: top, t }));
    }
    let piece = p.piece_mut(cuff).unwrap();
    let (top, bottom) = (piece.edges()[TOP].id, piece.edges()[BOTTOM].id);
    for edge in [top, bottom] {
        piece.notches.extend([0.25, 0.5, 0.75].map(|t| Notch { edge, t }));
    }

    let tubes = p.add_step("Costados y forro");
    let mouth = p.add_step("Boca");
    let bases = p.add_step("Base");
    for (name, id) in [("Costado", body), ("Costado del forro", lining), ("Costado del puño", cuff)] {
        sew(&mut p, name, &[(id, RIGHT, false), (id, LEFT, true)]);
    }
    // Cada base, de atrás (su vértice 2) a atrás dando toda la vuelta: sus
    // cuatro aristas son un solo lado.
    for (name, tube, round) in [("Base", body, base), ("Base del forro", lining, lining_base)] {
        let edges: Vec<_> = p.piece(round).unwrap().edges().iter().map(|e| e.id).collect();
        let bottom = p.piece(tube).unwrap().edges()[BOTTOM].id;
        let around = SeamSide::new(round, edges[2], true).through(edges[1]);
        p.add_seam(name, vec![SeamSide::new(tube, bottom, false), around]).expect("costura válida");
    }
    // La boca: el cuerpo, los dos bordes del puño y el forro, juntos.
    let edge = |p: &Pattern, id: PieceId, i: usize| p.piece(id).unwrap().edges()[i].id;
    let around_mouth = vec![
        SeamSide::new(cuff, edge(&p, cuff, BOTTOM), false),
        SeamSide::new(body, edge(&p, body, TOP), true),
        SeamSide::new(cuff, edge(&p, cuff, TOP), true),
        SeamSide::new(lining, edge(&p, lining, TOP), true),
    ];
    p.add_seam("Boca", around_mouth).expect("costura válida");
    for seam in &mut p.seams {
        seam.step = Some(match seam.name.as_str() {
            "Base" => bases,
            "Boca" => mouth,
            _ => tubes,
        });
        match seam.name.as_str() {
            // El punto se cose con remalladora, y el puño y el forro, estirados
            // hasta la boca.
            "Boca" => {
                seam.set_kind(SeamKind::Overlocked);
                seam.fit = SeamFit::Stretched;
            }
            "Costado del puño" => seam.set_kind(SeamKind::Overlocked),
            // La base del forro es algo más corta que su contorno (339 mm
            // por 341): se embebe.
            "Base del forro" => seam.fit = SeamFit::Eased,
            _ => {}
        }
    }
    // El doblez del puño, después de su costado: al partir las aristas de
    // los lados, la costura las sigue enteras.
    let fold = p
        .add_fold(cuff, Point::new(-1.0, cuff_h / 2.0), Point::new(w + 1.0, cuff_h / 2.0))
        .expect("el doblez cruza el puño");
    let piece = p.piece_mut(cuff).unwrap();
    let i = piece.line_index(fold).unwrap();
    // La solapa es la mitad de arriba: se dobla hacia dentro sobre la de
    // abajo, revés con revés.
    let line = &mut piece.lines[i];
    if !line.in_flap(Point::new(w / 2.0, 1.0)) {
        line.fold.as_mut().unwrap().flip ^= true;
    }
    p
}

/// Una pieza redonda de diámetro `diameter` (mm) con centro en `center`:
/// cuatro cuartos de Bézier y un piquete en el centro de cada uno.
fn round_piece(p: &mut Pattern, name: &str, center: Point, diameter: f64) -> PieceId {
    let radius = diameter / 2.0;
    let pts = [(0.0, -radius), (radius, 0.0), (0.0, radius), (-radius, 0.0)].map(|(x, y)| center + Vec2::new(x, y));
    let id = p.add_polygon(name, pts.to_vec());
    let piece = p.piece_mut(id).unwrap();
    piece.arrangement = Arrangement::Bottom;
    // Manejadores de una Bézier que aproxima un cuarto de círculo, en el
    // sentido del contorno (horario en pantalla).
    let k = radius * 0.552_284_75;
    let quarter = |from: Point| {
        let to_center = center - from;
        Vec2::new(to_center.y, -to_center.x).normalize() * k
    };
    for i in 0..4 {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        piece.set_edge_kind(i, EdgeKind::Cubic { h1: quarter(a), h2: -quarter(b) });
    }
    for i in 0..4 {
        let edge = piece.edges()[i].id;
        piece.notches.push(Notch { edge, t: 0.5 });
    }
    id
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
pub const ALL: [Example; 10] = [
    ("Camiseta básica", "camiseta-basica", t_shirt),
    ("Camiseta con mangas", "camiseta-con-mangas", t_shirt_with_sleeves),
    ("Bolsa tote", "bolsa-tote", tote_bag),
    ("Bolsa tote con forro", "bolsa-tote-con-forro", lined_tote_bag),
    ("Bolsa con fondo", "bolsa-con-fondo", box_bag),
    ("Bolso de cuero con forro", "bolso-de-cuero", leather_bag),
    ("Bolso con cremallera y asas", "bolso-con-cremallera", zipper_bag),
    ("Bolso con forro, bolsillo y cremallera", "bolso-con-forro", lined_zipper_bag),
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
