//! Lienzo 2D del editor: vista (zoom y desplazamiento), dibujo e
//! interacción con las herramientas.

use std::collections::HashMap;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Key, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Ui,
    epaint::CubicBezierShape,
};
use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathSeg};
use mesher::{Mesh2D, MeshOptions};
use pattern::{Pattern, Piece, PieceId, Point, SeamId, SeamSide, TextileId, Vec2};

/// Distancia (px) a la que se "engancha" el cursor a puntos y aristas.
const HIT_PX: f64 = 8.0;
const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 50.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Rect,
    Seam,
}

impl Tool {
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => {
                "Arrastra puntos, manejadores o piezas · doble clic en una arista añade un punto · C/L: curva/recta · Supr: borrar · F: encuadrar"
            }
            Tool::Pen => {
                "Clic para añadir puntos · clic en el primero o Intro para cerrar · Retroceso: quitar el último · Esc: cancelar"
            }
            Tool::Rect => "Arrastra para crear un rectángulo",
            Tool::Seam => {
                "Clic en cada arista, cerca del extremo por el que empieza la costura · Mayús+clic: añadir más lados · Esc: cancelar"
            }
        }
    }
}

/// Qué se puede hacer en el lienzo y qué destaca, según la etapa de trabajo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Dibujar y cambiar la forma de las piezas.
    #[default]
    Draw,
    /// Elegir telas: se selecciona la pieza entera y se ve de qué tela es.
    Fabrics,
    /// Coser: las piezas se atenúan y destacan las costuras; se eligen
    /// aristas y costuras, pero la forma no se toca.
    Seams,
    /// Colocar: se ve dónde va cada pieza en 3D. Las piezas se pueden mover
    /// (su sitio en el patrón cuenta al colocarlas), pero no su forma.
    Placement,
}

impl Mode {
    /// Se pueden mover puntos y manejadores.
    fn edits_shape(self) -> bool {
        self == Mode::Draw
    }

    /// Se pueden elegir aristas.
    fn picks_edges(self) -> bool {
        matches!(self, Mode::Draw | Mode::Seams)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Selection {
    #[default]
    None,
    Piece(PieceId),
    Vertex(PieceId, usize),
    Edge(PieceId, usize),
    Seam(SeamId),
}

impl Selection {
    pub fn piece(self) -> Option<PieceId> {
        match self {
            Selection::None | Selection::Seam(_) => None,
            Selection::Piece(id) | Selection::Vertex(id, _) | Selection::Edge(id, _) => Some(id),
        }
    }

    /// Descarta la selección si ya no existe en el patrón.
    fn sanitize(self, pattern: &Pattern) -> Self {
        if let Selection::Seam(id) = self {
            return if pattern.seam(id).is_some() { self } else { Selection::None };
        }
        let Some(piece) = self.piece().and_then(|id| pattern.piece(id)) else {
            return Selection::None;
        };
        match self {
            Selection::Vertex(_, i) | Selection::Edge(_, i) if i >= piece.len() => Selection::Piece(piece.id),
            s => s,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Hit {
    Handle(PieceId, usize, usize),
    Vertex(PieceId, usize),
    Edge(PieceId, usize, f64),
    Interior(PieceId),
}

enum Drag {
    None,
    Pan,
    Vertex(PieceId, usize),
    Handle(PieceId, usize, usize),
    Piece { original: Piece, start: Point },
    Rect { start: Point, current: Point },
}

/// Transformación mundo (mm) ↔ pantalla (px).
#[derive(Clone, Copy, Debug)]
struct View {
    /// Desplazamiento (px) del origen del mundo respecto al centro del lienzo.
    pan: egui::Vec2,
    /// Píxeles por milímetro.
    zoom: f32,
}

impl View {
    fn to_screen(self, rect: Rect, p: Point) -> Pos2 {
        rect.center() + self.pan + egui::vec2(p.x as f32, p.y as f32) * self.zoom
    }

    fn to_world(self, rect: Rect, pos: Pos2) -> Point {
        let v = (pos - rect.center() - self.pan) / self.zoom;
        Point::new(v.x as f64, v.y as f64)
    }
}

struct CachedMesh {
    piece: Piece,
    key: f64,
    mesh: Option<Mesh2D>,
}

pub struct Editor {
    pub tool: Tool,
    pub selection: Selection,
    pub snap: bool,
    /// Paso de la rejilla de ajuste (mm).
    pub snap_step: f64,
    pub show_mesh: bool,
    /// Dibujar la línea de corte (el contorno más el margen de costura).
    pub show_allowance: bool,
    /// Qué se hace en el lienzo según la etapa de trabajo.
    pub mode: Mode,
    /// Tamaño de los triángulos de la vista previa de malla (mm).
    pub mesh_size: f64,
    /// Posición del cursor en el mundo (mm), si está sobre el lienzo.
    pub cursor: Option<Point>,
    view: View,
    fit_requested: bool,
    canvas_rect: Rect,
    pen_points: Vec<Point>,
    /// Lados elegidos para la costura que se está creando.
    seam_sides: Vec<SeamSide>,
    drag: Drag,
    fill_cache: HashMap<PieceId, CachedMesh>,
    mesh_cache: HashMap<PieceId, CachedMesh>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            selection: Selection::None,
            snap: true,
            snap_step: 5.0,
            show_mesh: false,
            show_allowance: true,
            mode: Mode::Draw,
            mesh_size: 20.0,
            cursor: None,
            view: View { pan: egui::Vec2::ZERO, zoom: 1.0 },
            fit_requested: true,
            canvas_rect: Rect::NOTHING,
            pen_points: Vec::new(),
            seam_sides: Vec::new(),
            drag: Drag::None,
            fill_cache: HashMap::new(),
            mesh_cache: HashMap::new(),
        }
    }
}

impl Editor {
    /// Deja el editor listo para un patrón nuevo.
    pub fn reset(&mut self) {
        let keep = (self.snap, self.snap_step, self.show_mesh, self.mesh_size, self.show_allowance, self.mode);
        *self = Self::default();
        (self.snap, self.snap_step, self.show_mesh, self.mesh_size, self.show_allowance, self.mode) = keep;
    }

    pub fn fit_view(&mut self) {
        self.fit_requested = true;
    }

    pub fn zoom_percent(&self) -> f32 {
        self.view.zoom * 100.0
    }

    /// Acerca (`factor` > 1) o aleja la vista, alrededor del centro del lienzo.
    pub fn zoom_by(&mut self, factor: f32) {
        let rect = self.canvas_rect;
        if !rect.is_positive() {
            return;
        }
        let anchor = self.view.to_world(rect, rect.center());
        self.view.zoom = (self.view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let moved = self.view.to_screen(rect, anchor);
        self.view.pan += rect.center() - moved;
    }

    /// Zona de la pantalla que ocupa el lienzo.
    pub fn rect(&self) -> Rect {
        self.canvas_rect
    }

    /// Centro del lienzo en coordenadas del mundo.
    pub fn view_center(&self) -> Point {
        if self.canvas_rect.is_positive() {
            self.view.to_world(self.canvas_rect, self.canvas_rect.center())
        } else {
            Point::ZERO
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.pen_points.clear();
        self.seam_sides.clear();
        self.drag = Drag::None;
    }

    fn snap_point(&self, p: Point, ui: &Ui) -> Point {
        if !self.snap || ui.input(|i| i.modifiers.alt) {
            return p;
        }
        let s = self.snap_step;
        Point::new((p.x / s).round() * s, (p.y / s).round() * s)
    }

    pub fn ui(&mut self, ui: &mut Ui, pattern: &mut Pattern) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        self.canvas_rect = rect;
        self.selection = self.selection.sanitize(pattern);

        if self.fit_requested {
            self.fit_requested = false;
            self.fit(pattern, rect);
        }

        self.handle_zoom(ui, &response);
        self.cursor = response.hover_pos().map(|p| self.view.to_world(rect, p));
        if !ui.ctx().egui_wants_keyboard_input() {
            self.handle_keys(ui, pattern);
        }
        self.handle_pointer(ui, &response, pattern);
        self.context_menu(&response, pattern);

        self.draw(ui, &painter, pattern);
        self.update_cursor_icon(ui, &response, pattern);
    }

    // ---------------------------------------------------------------- vista

    fn fit(&mut self, pattern: &Pattern, rect: Rect) {
        let bbox = pattern.pieces.iter().map(|p| p.bounding_box()).reduce(|a, b| a.union(b));
        match bbox {
            Some(b) if b.width() > 0.0 && b.height() > 0.0 => {
                let zoom = ((rect.width() / b.width() as f32).min(rect.height() / b.height() as f32) * 0.85)
                    .clamp(MIN_ZOOM, MAX_ZOOM);
                let c = b.center();
                self.view = View { zoom, pan: -egui::vec2(c.x as f32, c.y as f32) * zoom };
            }
            _ => {
                // Patrón vacío: el origen arriba a la izquierda, a 1 px/mm.
                self.view = View { zoom: 1.0, pan: -rect.size() / 2.0 + egui::vec2(60.0, 60.0) };
            }
        }
    }

    fn handle_zoom(&mut self, ui: &Ui, response: &egui::Response) {
        let Some(hover) = response.hover_pos() else { return };
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = pinch * (scroll * 0.0015).exp();
        if factor == 1.0 {
            return;
        }
        let rect = response.rect;
        let anchor = self.view.to_world(rect, hover);
        self.view.zoom = (self.view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        // Mantén el punto bajo el cursor en su sitio.
        let moved = self.view.to_screen(rect, anchor);
        self.view.pan += hover - moved;
    }

    // ------------------------------------------------------------- teclado

    fn handle_keys(&mut self, ui: &Ui, pattern: &mut Pattern) {
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        let command = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        if command {
            if pressed(Key::D)
                && let Some(id) = self.selection.piece()
                && let Some(new) = pattern.duplicate_piece(id, Vec2::new(20.0, 20.0))
            {
                self.selection = Selection::Piece(new);
            }
            return;
        }
        if pressed(Key::V) {
            self.set_tool(Tool::Select);
        }
        if pressed(Key::P) {
            self.set_tool(Tool::Pen);
        }
        if pressed(Key::R) {
            self.set_tool(Tool::Rect);
        }
        if pressed(Key::S) {
            self.set_tool(Tool::Seam);
        }
        if pressed(Key::F) {
            self.fit_view();
        }

        if self.tool == Tool::Pen {
            if pressed(Key::Enter) {
                self.finish_pen(pattern);
            }
            if pressed(Key::Escape) {
                self.pen_points.clear();
            }
            if pressed(Key::Backspace) {
                self.pen_points.pop();
            }
            return;
        }
        if self.tool == Tool::Seam && pressed(Key::Escape) {
            if self.seam_sides.is_empty() {
                self.set_tool(Tool::Select);
            }
            self.seam_sides.clear();
            return;
        }

        if pressed(Key::Escape) {
            self.selection = Selection::None;
        }
        if pressed(Key::Delete) || pressed(Key::Backspace) {
            self.delete_selection(pattern);
        }
        if let Selection::Edge(id, i) = self.selection
            && self.mode.edits_shape()
            && let Some(piece) = pattern.piece_mut(id)
        {
            if pressed(Key::C) {
                piece.make_curve(i);
            }
            if pressed(Key::L) {
                piece.make_line(i);
            }
        }
    }

    pub fn delete_selection(&mut self, pattern: &mut Pattern) {
        match self.selection {
            Selection::Vertex(id, i) => {
                if pattern.remove_vertex(id, i) {
                    self.selection = Selection::Piece(id);
                }
            }
            Selection::Piece(id) => {
                pattern.remove_piece(id);
                self.selection = Selection::None;
            }
            Selection::Seam(id) => {
                pattern.remove_seam(id);
                self.selection = Selection::None;
            }
            Selection::Edge(..) | Selection::None => {}
        }
    }

    /// Añade a la costura en curso la arista bajo el cursor. El sentido sale
    /// del extremo más cercano al clic: la costura empieza por ahí. Sin
    /// Mayús, a partir del segundo lado la costura se crea.
    fn seam_click(&mut self, pattern: &mut Pattern, pos: Pos2, keep_open: bool) {
        let Some((piece, i, t)) = self.edge_hit(pattern, pos) else { return };
        let edge = pattern.piece(piece).unwrap().edges()[i].id;
        let side = SeamSide::new(piece, edge, t > 0.5);
        match self.seam_sides.iter_mut().find(|s| s.same_edge(&side)) {
            // Otra vez la misma arista: se le da la vuelta.
            Some(existing) => existing.reversed = !existing.reversed,
            None => self.seam_sides.push(side),
        }
        if !keep_open && self.seam_sides.len() >= 2 {
            let name = format!("Costura {}", pattern.seams.len() + 1);
            if let Some(id) = pattern.add_seam(name, std::mem::take(&mut self.seam_sides)) {
                self.selection = Selection::Seam(id);
            }
        }
    }

    /// Arista más cercana al cursor (pieza, índice, parámetro) dentro del
    /// margen de enganche, sin contar los vértices.
    fn edge_hit(&self, pattern: &Pattern, pos: Pos2) -> Option<(PieceId, usize, f64)> {
        let w = self.view.to_world(self.canvas_rect, pos);
        let tol = HIT_PX / self.view.zoom as f64;
        pattern
            .pieces
            .iter()
            .rev()
            .filter_map(|p| p.nearest_edge(w).map(|(i, t, d)| (p.id, i, t, d)))
            .filter(|h| h.3 < tol)
            .min_by(|a, b| a.3.total_cmp(&b.3))
            .map(|(id, i, t, _)| (id, i, t))
    }

    fn finish_pen(&mut self, pattern: &mut Pattern) {
        if self.pen_points.len() < 3 {
            return;
        }
        let name = format!("Pieza {}", pattern.pieces.len() + 1);
        let id = pattern.add_polygon(name, std::mem::take(&mut self.pen_points));
        self.selection = Selection::Piece(id);
        self.tool = Tool::Select;
    }

    // -------------------------------------------------------------- ratón

    fn hit_test(&self, pattern: &Pattern, pos: Pos2) -> Option<Hit> {
        let w = self.view.to_world(self.canvas_rect, pos);
        let tol = HIT_PX / self.view.zoom as f64;
        let selected = self.selection.piece();
        let (shape, edges) = (self.mode.edits_shape(), self.mode.picks_edges());

        if shape && let Some(piece) = selected.and_then(|id| pattern.piece(id)) {
            for i in 0..piece.len() {
                if let Some((c1, c2)) = piece.control_points(i) {
                    if c1.distance(w) < tol {
                        return Some(Hit::Handle(piece.id, i, 0));
                    }
                    if c2.distance(w) < tol {
                        return Some(Hit::Handle(piece.id, i, 1));
                    }
                }
            }
        }

        // La pieza seleccionada primero; luego de arriba (última) a abajo.
        let mut order: Vec<&Piece> = pattern.pieces.iter().rev().collect();
        if let Some(pos) = order.iter().position(|p| Some(p.id) == selected) {
            let p = order.remove(pos);
            order.insert(0, p);
        }

        for piece in order.iter().filter(|_| shape) {
            if let Some(i) = piece.vertices().iter().position(|v| v.distance(w) < tol) {
                return Some(Hit::Vertex(piece.id, i));
            }
        }
        for piece in &order {
            if let Some((i, t, d)) = piece.nearest_edge(w)
                && d < tol
            {
                // Sin elegir aristas, pinchar en el borde elige la pieza.
                return Some(if edges { Hit::Edge(piece.id, i, t) } else { Hit::Interior(piece.id) });
            }
        }
        order.iter().find(|p| p.contains(w)).map(|p| Hit::Interior(p.id))
    }

    fn select_hit(&mut self, hit: Option<Hit>) {
        self.selection = match hit {
            None => Selection::None,
            Some(Hit::Handle(id, e, _)) | Some(Hit::Edge(id, e, _)) => Selection::Edge(id, e),
            Some(Hit::Vertex(id, i)) => Selection::Vertex(id, i),
            Some(Hit::Interior(id)) => Selection::Piece(id),
        };
    }

    fn handle_pointer(&mut self, ui: &Ui, response: &egui::Response, pattern: &mut Pattern) {
        let rect = response.rect;
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let world = pointer.map(|p| self.view.to_world(rect, p));
        let space = ui.input(|i| i.key_down(Key::Space));

        // Desplazar con el botón central/derecho, o con espacio + arrastre.
        if response.dragged_by(PointerButton::Middle)
            || response.dragged_by(PointerButton::Secondary)
            || (space && response.dragged_by(PointerButton::Primary))
        {
            self.view.pan += response.drag_delta();
            return;
        }

        if response.drag_started_by(PointerButton::Primary) {
            let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
            self.start_drag(ui, pattern, origin);
        }

        if response.dragged_by(PointerButton::Primary)
            && let Some(w) = world
        {
            self.update_drag(ui, pattern, w, response.drag_delta());
        }

        if response.drag_stopped() {
            if let Drag::Rect { start, current } = std::mem::replace(&mut self.drag, Drag::None) {
                let r = kurbo::Rect::from_points(start, current);
                if r.width() >= 1.0 && r.height() >= 1.0 {
                    let name = format!("Pieza {}", pattern.pieces.len() + 1);
                    let id = pattern.add_rect(name, r.origin(), r.width(), r.height());
                    self.selection = Selection::Piece(id);
                    self.tool = Tool::Select;
                }
            }
            self.drag = Drag::None;
        }

        if response.clicked_by(PointerButton::Primary)
            && let Some(pos) = pointer
        {
            match self.tool {
                Tool::Select => self.select_hit(self.hit_test(pattern, pos)),
                Tool::Pen => {
                    let close = self.pen_points.len() >= 3
                        && self.view.to_screen(rect, self.pen_points[0]).distance(pos) < HIT_PX as f32;
                    if close {
                        self.finish_pen(pattern);
                    } else {
                        let p = self.snap_point(self.view.to_world(rect, pos), ui);
                        if self.pen_points.last() != Some(&p) {
                            self.pen_points.push(p);
                        }
                    }
                }
                Tool::Rect => {}
                Tool::Seam => {
                    let shift = ui.input(|i| i.modifiers.shift);
                    self.seam_click(pattern, pos, shift);
                }
            }
        }

        if response.double_clicked()
            && self.tool == Tool::Select
            && self.mode.picks_edges()
            && let Some(pos) = pointer
            && let Some(Hit::Edge(id, i, t)) = self.hit_test(pattern, pos)
            && let Some(v) = pattern.split_edge(id, i, t)
        {
            self.selection = Selection::Vertex(id, v);
        }

        if response.secondary_clicked()
            && self.tool == Tool::Select
            && let Some(pos) = pointer
        {
            self.select_hit(self.hit_test(pattern, pos));
        }
    }

    fn start_drag(&mut self, ui: &Ui, pattern: &Pattern, origin: Option<Pos2>) {
        let Some(origin) = origin else { return };
        let w = self.view.to_world(self.canvas_rect, origin);
        self.drag = match self.tool {
            Tool::Rect => {
                let p = self.snap_point(w, ui);
                Drag::Rect { start: p, current: p }
            }
            Tool::Pen | Tool::Seam => Drag::Pan,
            Tool::Select => {
                let hit = self.hit_test(pattern, origin);
                self.select_hit(hit);
                let moves = matches!(self.mode, Mode::Draw | Mode::Placement);
                match hit {
                    None => Drag::Pan,
                    Some(Hit::Handle(id, e, h)) => Drag::Handle(id, e, h),
                    Some(Hit::Vertex(id, i)) => Drag::Vertex(id, i),
                    Some(Hit::Edge(id, ..)) | Some(Hit::Interior(id)) if moves => {
                        Drag::Piece { original: pattern.piece(id).unwrap().clone(), start: w }
                    }
                    Some(_) => Drag::Pan,
                }
            }
        };
    }

    fn update_drag(&mut self, ui: &Ui, pattern: &mut Pattern, w: Point, delta_px: egui::Vec2) {
        match &mut self.drag {
            Drag::None => {}
            Drag::Pan => self.view.pan += delta_px,
            Drag::Vertex(id, i) => {
                let (id, i) = (*id, *i);
                let p = self.snap_point(w, ui);
                if let Some(piece) = pattern.piece_mut(id) {
                    piece.set_vertex(i, p);
                }
            }
            Drag::Handle(id, e, h) => {
                if let Some(piece) = pattern.piece_mut(*id) {
                    piece.set_control_point(*e, *h, w);
                }
            }
            Drag::Piece { original, start } => {
                let mut delta = w - *start;
                if self.snap && !ui.input(|i| i.modifiers.alt) {
                    let s = self.snap_step;
                    delta = Vec2::new((delta.x / s).round() * s, (delta.y / s).round() * s);
                }
                let mut moved = original.clone();
                moved.translate(delta);
                if let Some(piece) = pattern.piece_mut(moved.id) {
                    *piece = moved;
                }
            }
            Drag::Rect { current, .. } => {
                let p = if self.snap && !ui.input(|i| i.modifiers.alt) {
                    let s = self.snap_step;
                    Point::new((w.x / s).round() * s, (w.y / s).round() * s)
                } else {
                    w
                };
                *current = p;
            }
        }
    }

    fn context_menu(&mut self, response: &egui::Response, pattern: &mut Pattern) {
        if self.tool != Tool::Select {
            return;
        }
        response.context_menu(|ui| match self.selection {
            Selection::None => {
                ui.label("Nada seleccionado");
            }
            Selection::Edge(id, i) => {
                let is_curve = pattern.piece(id).is_some_and(|p| p.edges()[i].is_curve());
                let shape = self.mode.edits_shape();
                if shape && !is_curve && ui.button("Convertir en curva (C)").clicked() {
                    pattern.piece_mut(id).unwrap().make_curve(i);
                }
                if shape && is_curve && ui.button("Convertir en recta (L)").clicked() {
                    pattern.piece_mut(id).unwrap().make_line(i);
                }
                if ui.button("Añadir punto en el centro").clicked()
                    && let Some(v) = pattern.split_edge(id, i, 0.5)
                {
                    self.selection = Selection::Vertex(id, v);
                }
            }
            Selection::Vertex(..) => {
                if ui.button("Borrar punto (Supr)").clicked() {
                    self.delete_selection(pattern);
                }
            }
            Selection::Seam(..) => {
                if ui.button("Borrar costura (Supr)").clicked() {
                    self.delete_selection(pattern);
                }
            }
            Selection::Piece(id) => {
                if ui.button("Duplicar (Ctrl+D)").clicked()
                    && let Some(new) = pattern.duplicate_piece(id, Vec2::new(20.0, 20.0))
                {
                    self.selection = Selection::Piece(new);
                }
                if ui.button("Borrar pieza (Supr)").clicked() {
                    self.delete_selection(pattern);
                }
            }
        });
    }

    fn update_cursor_icon(&self, ui: &Ui, response: &egui::Response, pattern: &Pattern) {
        let icon = match (&self.drag, self.tool) {
            (Drag::Pan, _) => CursorIcon::Grabbing,
            (Drag::Piece { .. }, _) => CursorIcon::Move,
            (Drag::Vertex(..) | Drag::Handle(..), _) => CursorIcon::Grabbing,
            (_, Tool::Pen | Tool::Rect) => CursorIcon::Crosshair,
            (_, Tool::Seam) => match response.hover_pos().and_then(|p| self.edge_hit(pattern, p)) {
                Some(_) => CursorIcon::PointingHand,
                None => CursorIcon::Default,
            },
            (_, Tool::Select) => match response.hover_pos().and_then(|p| self.hit_test(pattern, p)) {
                Some(Hit::Vertex(..) | Hit::Handle(..)) => CursorIcon::Grab,
                Some(Hit::Edge(..) | Hit::Interior(..)) => CursorIcon::PointingHand,
                None => CursorIcon::Default,
            },
        };
        if response.hovered() || !matches!(self.drag, Drag::None) {
            ui.ctx().set_cursor_icon(icon);
        }
    }

    // --------------------------------------------------------------- dibujo

    fn draw(&mut self, ui: &Ui, painter: &Painter, pattern: &Pattern) {
        let rect = self.canvas_rect;
        let visuals = ui.visuals();
        let dark = visuals.dark_mode;
        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);
        self.draw_grid(painter, dark);

        let text_color = visuals.text_color();
        let selected = self.selection.piece();

        self.refresh_caches(pattern);
        let focus = self.mode == Mode::Seams;
        for piece in &pattern.pieces {
            let color = piece_color(piece.id);
            // En la etapa Telas, cada pieza se pinta del color de su tela.
            let fill = match self.mode {
                Mode::Fabrics => textile_color(piece.fabric),
                _ => color,
            };
            let is_sel = Some(piece.id) == selected;

            if self.show_allowance || focus {
                self.draw_allowance(painter, pattern, piece, color);
            }
            if let Some(mesh) = self.fill_cache.get(&piece.id).and_then(|c| c.mesh.as_ref()) {
                let alpha = match (focus, is_sel, self.mode) {
                    (true, _, _) => 25,
                    (false, true, Mode::Fabrics) => 150,
                    (false, false, Mode::Fabrics) => 110,
                    (false, true, _) => 70,
                    (false, false, _) => 45,
                };
                // Si lo que se ve desde fuera es el revés, un relleno gris.
                let fill =
                    if piece.right_side_in && self.mode != Mode::Fabrics { Color32::from_gray(150) } else { fill };
                painter.add(self.egui_mesh(mesh, fill.gamma_multiply_u8(alpha)));
            }
            if self.show_mesh
                && let Some(mesh) = self.mesh_cache.get(&piece.id).and_then(|c| c.mesh.as_ref())
            {
                self.draw_wireframe(painter, mesh, color.gamma_multiply(0.6));
            }

            let broken = self.fill_cache.get(&piece.id).is_some_and(|c| c.mesh.is_none());
            let outline = match (broken, focus) {
                (true, _) => Color32::RED,
                (false, true) => color.gamma_multiply(0.55),
                (false, false) => color,
            };
            let width = match (is_sel, focus) {
                (true, _) => 2.5,
                (false, true) => 1.0,
                (false, false) => 1.5,
            };
            for i in 0..piece.len() {
                let edge_sel = self.selection == Selection::Edge(piece.id, i);
                let stroke = if edge_sel {
                    Stroke::new(4.0, visuals.selection.stroke.color)
                } else {
                    Stroke::new(width, outline)
                };
                self.draw_segment(painter, piece.segment(i), stroke);
            }
            for (i, v) in piece.vertices().iter().enumerate() {
                let pos = self.view.to_screen(rect, *v);
                let vsel = self.selection == Selection::Vertex(piece.id, i);
                let r = if vsel {
                    6.0
                } else if is_sel {
                    4.0
                } else {
                    2.5
                };
                painter.circle_filled(pos, r, if vsel { visuals.selection.stroke.color } else { outline });
            }
        }

        self.draw_seams(painter, pattern);
        self.draw_seam_tool(painter, ui, pattern);

        // Nombres encima de todo lo demás y, debajo, lo que importa en la
        // etapa: las telas de la pieza o dónde va en 3D.
        for piece in &pattern.pieces {
            let c = piece.bounding_box().center();
            let pos = self.view.to_screen(rect, c);
            painter.text(pos, Align2::CENTER_CENTER, &piece.name, FontId::proportional(14.0), text_color);
            let note = match self.mode {
                Mode::Fabrics => Some(textile_names(pattern, piece)),
                Mode::Placement if piece.right_side_in => {
                    Some(format!("{} · revés fuera", crate::stages::slot_label(pattern, piece)))
                }
                Mode::Placement => Some(crate::stages::slot_label(pattern, piece)),
                _ => piece.right_side_in.then(|| "revés hacia fuera".to_owned()),
            };
            if let Some(note) = note {
                let below = pos + egui::vec2(0.0, 15.0);
                painter.text(
                    below,
                    Align2::CENTER_CENTER,
                    note,
                    FontId::proportional(11.0),
                    text_color.gamma_multiply(0.7),
                );
            }
        }

        if let Some(piece) = selected.and_then(|id| pattern.piece(id)) {
            self.draw_selected_details(painter, piece, text_color, visuals.selection.stroke.color);
        }

        self.draw_tool_preview(painter, ui);
    }

    fn refresh_caches(&mut self, pattern: &Pattern) {
        let fill_opts = MeshOptions { target_edge_length: f64::MAX / 4.0, curve_tolerance: 0.3 };
        let sim_opts = MeshOptions { target_edge_length: self.mesh_size, ..Default::default() };
        let ids: Vec<PieceId> = pattern.pieces.iter().map(|p| p.id).collect();
        self.fill_cache.retain(|id, _| ids.contains(id));
        self.mesh_cache.retain(|id, _| ids.contains(id));
        for piece in &pattern.pieces {
            update_cache(&mut self.fill_cache, piece, 0.0, &fill_opts);
            if self.show_mesh {
                update_cache(&mut self.mesh_cache, piece, self.mesh_size, &sim_opts);
            }
        }
    }

    fn egui_mesh(&self, mesh: &Mesh2D, color: Color32) -> Shape {
        let mut m = egui::Mesh::default();
        for p in &mesh.positions {
            m.colored_vertex(self.view.to_screen(self.canvas_rect, Point::new(p[0], p[1])), color);
        }
        for t in &mesh.triangles {
            m.add_triangle(t[0], t[1], t[2]);
        }
        Shape::mesh(m)
    }

    fn draw_wireframe(&self, painter: &Painter, mesh: &Mesh2D, color: Color32) {
        let pts: Vec<Pos2> =
            mesh.positions.iter().map(|p| self.view.to_screen(self.canvas_rect, Point::new(p[0], p[1]))).collect();
        let stroke = Stroke::new(0.7, color);
        let shapes = mesh
            .edges()
            .into_iter()
            .map(|[a, b]| Shape::line_segment([pts[a as usize], pts[b as usize]], stroke))
            .collect::<Vec<_>>();
        painter.extend(shapes);
    }

    fn draw_segment(&self, painter: &Painter, seg: PathSeg, stroke: Stroke) {
        let s = |p: Point| self.view.to_screen(self.canvas_rect, p);
        match seg {
            PathSeg::Line(l) => {
                painter.line_segment([s(l.p0), s(l.p1)], stroke);
            }
            PathSeg::Cubic(c) => {
                painter.add(CubicBezierShape::from_points_stroke(
                    [s(c.p0), s(c.p1), s(c.p2), s(c.p3)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            }
            PathSeg::Quad(q) => {
                painter.line_segment([s(q.p0), s(q.p2)], stroke);
            }
        }
    }

    fn draw_grid(&self, painter: &Painter, dark: bool) {
        let rect = self.canvas_rect;
        let zoom = self.view.zoom as f64;
        // Paso menor: el primero que deje al menos 10 px entre líneas.
        let minor = [1.0, 5.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 5000.0]
            .into_iter()
            .find(|s| s * zoom >= 10.0)
            .unwrap_or(10_000.0);
        let major = minor * 10.0;
        let base = if dark { 255 } else { 0 };
        let minor_c = Color32::from_rgba_unmultiplied(base, base, base, 12);
        let major_c = Color32::from_rgba_unmultiplied(base, base, base, 30);
        let axis_c = Color32::from_rgba_unmultiplied(base, base, base, 70);

        let tl = self.view.to_world(rect, rect.left_top());
        let br = self.view.to_world(rect, rect.right_bottom());
        let mut shapes = Vec::new();
        let mut x = (tl.x / minor).floor() * minor;
        while x <= br.x {
            let c = if x.abs() < 1e-9 {
                axis_c
            } else if (x / major).round() * major == x {
                major_c
            } else {
                minor_c
            };
            let sx = self.view.to_screen(rect, Point::new(x, 0.0)).x;
            shapes.push(Shape::line_segment([Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())], (1.0, c)));
            x += minor;
        }
        let mut y = (tl.y / minor).floor() * minor;
        while y <= br.y {
            let c = if y.abs() < 1e-9 {
                axis_c
            } else if (y / major).round() * major == y {
                major_c
            } else {
                minor_c
            };
            let sy = self.view.to_screen(rect, Point::new(0.0, y)).y;
            shapes.push(Shape::line_segment([Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)], (1.0, c)));
            y += minor;
        }
        painter.extend(shapes);
    }

    fn draw_seams(&self, painter: &Painter, pattern: &Pattern) {
        for (k, seam) in pattern.seams.iter().enumerate() {
            let color = seam_color(k);
            let selected = self.selection == Selection::Seam(seam.id);
            let width = match (selected, self.mode == Mode::Seams) {
                (true, _) => 3.5,
                (false, true) => 2.8,
                (false, false) => 2.0,
            };
            // En el modo costuras, la seleccionada dice también su tipo.
            let label = if self.mode == Mode::Seams && selected {
                format!("{} · {}", k + 1, seam.kind.label())
            } else {
                format!("{}", k + 1)
            };
            for side in &seam.sides {
                self.draw_seam_side(painter, pattern, side, color, width, &label);
            }
            if selected {
                self.draw_seam_matches(painter, pattern, seam, color);
            }
        }
    }

    /// Líneas finas entre los puntos que se cosen juntos de cada lado de la
    /// costura: si se cruzan, algún lado va al revés.
    fn draw_seam_matches(&self, painter: &Painter, pattern: &Pattern, seam: &pattern::Seam, color: Color32) {
        const STEPS: usize = 8;
        let points: Vec<Vec<Pos2>> = seam
            .sides
            .iter()
            .filter_map(|side| {
                let piece = pattern.piece(side.piece)?;
                let seg = piece.segment(piece.edge_index(side.edge)?);
                let length = seg.arclen(1e-3);
                Some(
                    (0..=STEPS)
                        .map(|k| {
                            let s = k as f64 / STEPS as f64;
                            let s = if side.reversed { 1.0 - s } else { s };
                            let t = seg.inv_arclen(s * length, 1e-3);
                            self.view.to_screen(self.canvas_rect, seg.eval(t))
                        })
                        .collect(),
                )
            })
            .collect();
        let Some((first, rest)) = points.split_first() else { return };
        let stroke = Stroke::new(1.0, color.gamma_multiply(0.6));
        for other in rest {
            for (a, b) in first.iter().zip(other) {
                painter.line_segment([*a, *b], stroke);
            }
        }
    }

    /// Línea de corte: el contorno más el margen de cada arista, discontinua.
    fn draw_allowance(&self, painter: &Painter, pattern: &Pattern, piece: &Piece, color: Color32) {
        let outline = piece.allowance_outline(&pattern.allowances(piece));
        if outline.is_empty() {
            return;
        }
        let mut pts: Vec<Pos2> = outline.iter().map(|p| self.view.to_screen(self.canvas_rect, *p)).collect();
        pts.push(pts[0]);
        let stroke = Stroke::new(1.0, color.gamma_multiply(if self.mode == Mode::Seams { 0.8 } else { 0.5 }));
        painter.extend(Shape::dashed_line(&pts, stroke, 5.0, 3.0));
    }

    /// Trazo discontinuo por dentro de la arista, con una flecha que marca
    /// el sentido de la costura y su número.
    fn draw_seam_side(
        &self,
        painter: &Painter,
        pattern: &Pattern,
        side: &SeamSide,
        color: Color32,
        width: f32,
        label: &str,
    ) {
        let Some(piece) = pattern.piece(side.piece) else { return };
        let Some(i) = piece.edge_index(side.edge) else { return };
        let seg = piece.segment(i);
        let inset = 6.0 / self.view.zoom as f64;
        let at = |t: f64| self.view.to_screen(self.canvas_rect, seg.eval(t) + inward_normal(piece, &seg, t) * inset);
        let pts: Vec<Pos2> = (0..=24).map(|j| at(j as f64 / 24.0)).collect();
        painter.extend(Shape::dashed_line(&pts, Stroke::new(width, color), 6.0, 4.0));

        // Flecha en el centro, hacia donde avanza la costura.
        let (from, to) = if side.reversed { (at(0.55), at(0.45)) } else { (at(0.45), at(0.55)) };
        let dir = (to - from).normalized();
        if dir.is_finite() {
            let tip = at(0.5) + dir * 7.0;
            let back = tip - dir * 12.0;
            let normal = egui::vec2(-dir.y, dir.x) * 5.0;
            painter.add(Shape::convex_polygon(vec![tip, back + normal, back - normal], color, Stroke::NONE));
        }
        // Punto en el extremo donde empieza.
        painter.circle_filled(at(if side.reversed { 1.0 } else { 0.0 }), width + 1.5, color);

        let label_at = seg.eval(0.5) + inward_normal(piece, &seg, 0.5) * (20.0 / self.view.zoom as f64);
        painter.text(
            self.view.to_screen(self.canvas_rect, label_at),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(12.0),
            color,
        );
    }

    /// Lados ya elegidos de la costura en curso y, bajo el cursor, la arista
    /// que se añadiría.
    fn draw_seam_tool(&self, painter: &Painter, ui: &Ui, pattern: &Pattern) {
        if self.tool != Tool::Seam {
            return;
        }
        let accent = ui.visuals().selection.stroke.color;
        for (k, side) in self.seam_sides.iter().enumerate() {
            self.draw_seam_side(painter, pattern, side, accent, 3.5, &format!("{}", k + 1));
        }
        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| self.canvas_rect.contains(*p));
        if let Some((piece, i, t)) = hover.and_then(|p| self.edge_hit(pattern, p)) {
            let edge = pattern.piece(piece).unwrap().edges()[i].id;
            let side = SeamSide::new(piece, edge, t > 0.5);
            if !self.seam_sides.iter().any(|s| s.same_edge(&side)) {
                let label = format!("{}", self.seam_sides.len() + 1);
                self.draw_seam_side(painter, pattern, &side, accent.gamma_multiply(0.6), 3.0, &label);
            }
        }
    }

    fn draw_selected_details(&self, painter: &Painter, piece: &Piece, text: Color32, accent: Color32) {
        let s = |p: Point| self.view.to_screen(self.canvas_rect, p);
        // Manejadores de las curvas.
        for i in 0..piece.len() {
            let Some((c1, c2)) = piece.control_points(i) else { continue };
            let (a, b) = piece.edge_endpoints(i);
            let stroke = Stroke::new(1.0, accent.gamma_multiply(0.8));
            painter.line_segment([s(piece.vertices()[a]), s(c1)], stroke);
            painter.line_segment([s(piece.vertices()[b]), s(c2)], stroke);
            for c in [c1, c2] {
                painter.circle(s(c), 4.0, Color32::WHITE, Stroke::new(1.5, accent));
            }
        }
        // Longitud de cada arista (cm), por fuera de la pieza.
        for i in 0..piece.len() {
            let seg = piece.segment(i);
            let at = seg.eval(0.5) - inward_normal(piece, &seg, 0.5) * (14.0 / self.view.zoom as f64);
            painter.text(
                s(at),
                Align2::CENTER_CENTER,
                format!("{:.1}", piece.edge_length(i) / 10.0),
                FontId::monospace(11.0),
                text.gamma_multiply(0.8),
            );
        }
    }

    fn draw_tool_preview(&self, painter: &Painter, ui: &Ui) {
        let rect = self.canvas_rect;
        let accent = ui.visuals().selection.stroke.color;
        let stroke = Stroke::new(1.5, accent);
        match (&self.drag, self.tool) {
            (Drag::Rect { start, current }, _) => {
                let r = Rect::from_two_pos(self.view.to_screen(rect, *start), self.view.to_screen(rect, *current));
                painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
                let size =
                    format!("{:.1} × {:.1} cm", (current.x - start.x).abs() / 10.0, (current.y - start.y).abs() / 10.0);
                painter.text(
                    r.right_bottom() + egui::vec2(6.0, 6.0),
                    Align2::LEFT_TOP,
                    size,
                    FontId::monospace(11.0),
                    accent,
                );
            }
            (_, Tool::Pen) if !self.pen_points.is_empty() => {
                let mut pts: Vec<Pos2> = self.pen_points.iter().map(|p| self.view.to_screen(rect, *p)).collect();
                if let Some(c) = self.cursor {
                    pts.push(self.view.to_screen(rect, self.snap_point(c, ui)));
                }
                painter.add(Shape::line(pts.clone(), stroke));
                for (i, p) in pts.iter().take(self.pen_points.len()).enumerate() {
                    let r = if i == 0 && self.pen_points.len() >= 3 { 6.0 } else { 3.5 };
                    painter.circle_filled(*p, r, accent);
                }
            }
            _ => {}
        }
    }
}

fn update_cache(cache: &mut HashMap<PieceId, CachedMesh>, piece: &Piece, key: f64, opts: &MeshOptions) {
    let fresh = cache.get(&piece.id).is_some_and(|c| c.key == key && c.piece == *piece);
    if !fresh {
        let mesh = mesher::mesh_piece(piece, opts).ok();
        cache.insert(piece.id, CachedMesh { piece: piece.clone(), key, mesh });
    }
}

/// Normal unitaria hacia el interior de la pieza en el punto `t` de `seg`.
fn inward_normal(piece: &Piece, seg: &PathSeg, t: f64) -> Vec2 {
    let d = match seg {
        PathSeg::Line(l) => l.p1 - l.p0,
        PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
        PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
    };
    let len = d.hypot();
    if len < 1e-12 {
        return Vec2::ZERO;
    }
    // Con Y hacia abajo, área positiva = sentido horario en pantalla, y el
    // interior queda a la derecha del avance: (-dy, dx).
    let n = Vec2::new(-d.y, d.x) / len;
    if piece.area() >= 0.0 { n } else { -n }
}

/// Las telas de una pieza, de fuera a dentro («Lona + Entretela»).
fn textile_names(pattern: &Pattern, piece: &Piece) -> String {
    let names: Vec<&str> = piece.textiles().map(|t| pattern.textile(t).map_or("?", |t| t.name.as_str())).collect();
    names.join(" + ")
}

pub fn piece_color(id: PieceId) -> Color32 {
    let hue = (id.0 as f32 * 0.618_034).fract();
    egui::ecolor::Hsva::new(hue, 0.55, 0.85, 1.0).into()
}

/// Color con el que se pinta una tela en la etapa Telas: el mismo para
/// todas las piezas de esa tela.
pub fn textile_color(id: TextileId) -> Color32 {
    let hue = (id.0 as f32 * 0.381_966 + 0.08).fract();
    egui::ecolor::Hsva::new(hue, 0.45, 0.75, 1.0).into()
}

pub fn seam_color(index: usize) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(230, 120, 30),
        Color32::from_rgb(40, 160, 220),
        Color32::from_rgb(200, 60, 160),
        Color32::from_rgb(60, 180, 90),
        Color32::from_rgb(220, 190, 40),
        Color32::from_rgb(130, 100, 230),
    ];
    PALETTE[index % PALETTE.len()]
}
