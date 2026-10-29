//! Ventana principal: menús, panel de propiedades, ficheros y deshacer.

use std::path::PathBuf;

use eframe::egui::{self, Color32, Key, KeyboardShortcut, Modifiers, RichText, Ui};
use pattern::{EdgeKind, Issue, Pattern, Point};

use crate::canvas::{Editor, Selection, Tool};
use crate::examples;
use crate::history::History;

const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const REDO_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);

/// Acción que espera a que el usuario decida qué hacer con los cambios sin guardar.
#[derive(Clone, Copy)]
enum Pending {
    New,
    Open,
    Example(fn() -> Pattern),
    Quit,
}

pub struct ClothingMakerApp {
    pattern: Pattern,
    path: Option<PathBuf>,
    /// Última versión guardada, para saber si hay cambios.
    saved: Pattern,
    history: History,
    editor: Editor,
    status: Option<(String, bool)>,
    pending: Option<Pending>,
    allow_close: bool,
    title: String,
}

impl ClothingMakerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        let pattern = Pattern::new("Sin título");
        let mut app = Self {
            history: History::new(&pattern),
            saved: pattern.clone(),
            pattern,
            path: None,
            editor: Editor::default(),
            status: None,
            pending: None,
            allow_close: false,
            title: String::new(),
        };
        if let Some(path) = file {
            app.load_from(path);
        }
        app
    }

    fn is_dirty(&self) -> bool {
        self.pattern != self.saved
    }

    fn set_pattern(&mut self, pattern: Pattern, path: Option<PathBuf>) {
        self.history.reset(&pattern);
        self.saved = if path.is_some() { pattern.clone() } else { Pattern::new("Sin título") };
        self.pattern = pattern;
        self.path = path;
        self.editor.reset();
    }

    // ------------------------------------------------------------- ficheros

    fn file_dialog(&self) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new().add_filter("Patrón (JSON)", &["json"]);
        match self.path.as_ref().and_then(|p| p.parent()) {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        }
    }

    fn open(&mut self) {
        if let Some(path) = self.file_dialog().pick_file() {
            self.load_from(path);
        }
    }

    fn load_from(&mut self, path: PathBuf) {
        match Pattern::load(&path) {
            Ok(pattern) => {
                self.status = Some((format!("Abierto {}", path.display()), false));
                self.set_pattern(pattern, Some(path));
            }
            Err(e) => self.status = Some((format!("No se pudo abrir {}: {e}", path.display()), true)),
        }
    }

    /// Guarda. Devuelve `true` si se ha guardado.
    fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(path) => self.save_to(path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) -> bool {
        let name = format!("{}.pattern.json", self.pattern.name);
        match self.file_dialog().set_file_name(name).save_file() {
            Some(path) => self.save_to(path),
            None => false,
        }
    }

    fn save_to(&mut self, path: PathBuf) -> bool {
        match self.pattern.save(&path) {
            Ok(()) => {
                self.status = Some((format!("Guardado en {}", path.display()), false));
                self.saved = self.pattern.clone();
                self.path = Some(path);
                true
            }
            Err(e) => {
                self.status = Some((format!("No se pudo guardar: {e}"), true));
                false
            }
        }
    }

    /// Ejecuta una acción que descarta el patrón actual, preguntando antes
    /// si hay cambios sin guardar.
    fn request(&mut self, action: Pending) {
        if self.is_dirty() {
            self.pending = Some(action);
        } else {
            self.run(action);
        }
    }

    fn run(&mut self, action: Pending) {
        match action {
            Pending::New => self.set_pattern(Pattern::new("Sin título"), None),
            Pending::Open => self.open(),
            Pending::Example(make) => self.set_pattern(make(), None),
            Pending::Quit => self.allow_close = true,
        }
    }

    // ------------------------------------------------------------------- ui

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let consume = |s: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(s));
        // Primero las que llevan Mayús, que si no las «come» la versión sin ella.
        if consume(&SAVE_AS) {
            self.save_as();
        } else if consume(&SAVE) {
            self.save();
        }
        if consume(&OPEN) {
            self.request(Pending::Open);
        }
        if consume(&NEW) {
            self.request(Pending::New);
        }
        if ctx.egui_wants_keyboard_input() {
            return; // deja que los campos de texto gestionen su propio deshacer
        }
        if consume(&REDO) || consume(&REDO_ALT) {
            self.history.redo(&mut self.pattern);
        } else if consume(&UNDO) {
            self.history.undo(&mut self.pattern);
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Archivo", |ui| {
                if ui.add(egui::Button::new("Nuevo").shortcut_text(ui.ctx().format_shortcut(&NEW))).clicked() {
                    self.request(Pending::New);
                }
                if ui.add(egui::Button::new("Abrir…").shortcut_text(ui.ctx().format_shortcut(&OPEN))).clicked() {
                    self.request(Pending::Open);
                }
                ui.menu_button("Ejemplos", |ui| {
                    if ui.button("Camiseta básica").clicked() {
                        self.request(Pending::Example(examples::t_shirt));
                    }
                    if ui.button("Bolsa tote").clicked() {
                        self.request(Pending::Example(examples::tote_bag));
                    }
                });
                ui.separator();
                if ui.add(egui::Button::new("Guardar").shortcut_text(ui.ctx().format_shortcut(&SAVE))).clicked() {
                    self.save();
                }
                if ui
                    .add(egui::Button::new("Guardar como…").shortcut_text(ui.ctx().format_shortcut(&SAVE_AS)))
                    .clicked()
                {
                    self.save_as();
                }
                ui.separator();
                if ui.button("Salir").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                let undo = egui::Button::new("Deshacer").shortcut_text(ui.ctx().format_shortcut(&UNDO));
                if ui.add_enabled(self.history.can_undo() || self.is_dirty(), undo).clicked() {
                    self.history.undo(&mut self.pattern);
                }
                let redo = egui::Button::new("Rehacer").shortcut_text(ui.ctx().format_shortcut(&REDO));
                if ui.add_enabled(self.history.can_redo(), redo).clicked() {
                    self.history.redo(&mut self.pattern);
                }
            });
            ui.menu_button("Ver", |ui| {
                if ui.button("Encuadrar todo (F)").clicked() {
                    self.editor.fit_view();
                }
                ui.checkbox(&mut self.editor.show_mesh, "Mostrar malla de simulación");
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            for tool in [Tool::Select, Tool::Pen, Tool::Rect] {
                if ui.selectable_label(self.editor.tool == tool, tool.label()).clicked() {
                    self.editor.set_tool(tool);
                }
            }
            ui.separator();
            ui.checkbox(&mut self.editor.snap, "Ajustar a rejilla");
            ui.add_enabled(
                self.editor.snap,
                egui::DragValue::new(&mut self.editor.snap_step).range(0.5..=100.0).speed(0.5).suffix(" mm"),
            );
            ui.separator();
            ui.checkbox(&mut self.editor.show_mesh, "Malla");
            ui.add_enabled(
                self.editor.show_mesh,
                egui::DragValue::new(&mut self.editor.mesh_size).range(3.0..=200.0).speed(0.5).suffix(" mm"),
            );
        });
    }

    fn status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            match &self.status {
                Some((msg, true)) => {
                    ui.label(RichText::new(msg).color(Color32::from_rgb(230, 80, 80)));
                }
                Some((msg, false)) => {
                    ui.label(msg);
                }
                None => {
                    ui.label(self.editor.tool.hint());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{:.0} %", self.editor.zoom_percent()));
                if let Some(c) = self.editor.cursor {
                    ui.monospace(format!("x {:>7.1}  y {:>7.1} cm", c.x / 10.0, c.y / 10.0));
                }
            });
        });
    }

    fn properties(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Patrón");
            ui.text_edit_singleline(&mut self.pattern.name);
            ui.add_space(8.0);

            ui.heading("Piezas");
            let selected = self.editor.selection.piece();
            for piece in &self.pattern.pieces {
                if ui.selectable_label(selected == Some(piece.id), &piece.name).clicked() {
                    self.editor.selection = Selection::Piece(piece.id);
                }
            }
            if self.pattern.pieces.is_empty() {
                ui.weak("Aún no hay piezas. Usa la pluma (P) o el rectángulo (R).");
            }
            ui.horizontal(|ui| {
                if ui.button("＋ Rectángulo").clicked() {
                    let c = self.editor.view_center();
                    let name = format!("Pieza {}", self.pattern.pieces.len() + 1);
                    let id = self.pattern.add_rect(name, Point::new(c.x - 150.0, c.y - 200.0), 300.0, 400.0);
                    self.editor.selection = Selection::Piece(id);
                }
                if let Some(id) = selected {
                    if ui.button("Duplicar").clicked()
                        && let Some(new) = self.pattern.duplicate_piece(id, pattern::Vec2::new(20.0, 20.0))
                    {
                        self.editor.selection = Selection::Piece(new);
                    }
                    if ui.button("Borrar").clicked() {
                        self.pattern.remove_piece(id);
                        self.editor.selection = Selection::None;
                    }
                }
            });
            ui.separator();

            self.selection_details(ui);

            let issues = self.pattern.validate();
            if !issues.is_empty() {
                ui.separator();
                ui.heading("Avisos");
                for issue in issues {
                    ui.label(RichText::new(self.describe(&issue)).color(Color32::from_rgb(230, 150, 40)));
                }
            }

            if !self.pattern.seams.is_empty() {
                ui.separator();
                ui.heading("Costuras");
                for (k, seam) in self.pattern.seams.iter().enumerate() {
                    let pieces: Vec<&str> = seam
                        .sides
                        .iter()
                        .filter_map(|s| self.pattern.piece(s.piece).map(|p| p.name.as_str()))
                        .collect();
                    ui.label(format!("{}. {} — {}", k + 1, seam.name, pieces.join(" + ")));
                }
                ui.weak("La edición de costuras llega en la siguiente fase.");
            }
        });
    }

    fn selection_details(&mut self, ui: &mut Ui) {
        let selection = self.editor.selection;
        let Some(piece) = selection.piece().and_then(|id| self.pattern.piece_mut(id)) else {
            ui.weak("Selecciona una pieza, un punto o una arista para ver sus propiedades.");
            return;
        };

        ui.heading("Pieza");
        ui.text_edit_singleline(&mut piece.name);
        egui::Grid::new("piece_info").num_columns(2).show(ui, |ui| {
            ui.label("Área");
            ui.label(format!("{:.1} cm²", piece.area().abs() / 100.0));
            ui.end_row();
            ui.label("Perímetro");
            ui.label(format!("{:.1} cm", piece.perimeter() / 10.0));
            ui.end_row();
            ui.label("Puntos");
            ui.label(piece.len().to_string());
            ui.end_row();
        });

        match selection {
            Selection::Vertex(_, i) => {
                ui.add_space(8.0);
                ui.heading(format!("Punto {}", i + 1));
                let mut p = piece.vertices()[i];
                if point_editor(ui, &mut p) {
                    piece.set_vertex(i, p);
                }
                if ui.add_enabled(piece.len() > 3, egui::Button::new("Borrar punto")).clicked() {
                    self.editor.delete_selection(&mut self.pattern);
                }
            }
            Selection::Edge(_, i) => {
                ui.add_space(8.0);
                ui.heading(format!("Arista {}", i + 1));
                ui.label(format!("Longitud: {:.1} cm", piece.edge_length(i) / 10.0));
                let mut curve = piece.edges()[i].is_curve();
                ui.horizontal(|ui| {
                    ui.radio_value(&mut curve, false, "Recta");
                    ui.radio_value(&mut curve, true, "Curva");
                });
                if curve != piece.edges()[i].is_curve() {
                    if curve { piece.make_curve(i) } else { piece.make_line(i) }
                }
                if let Some((mut c1, mut c2)) = piece.control_points(i) {
                    ui.label("Manejador 1");
                    if point_editor(ui, &mut c1) {
                        piece.set_control_point(i, 0, c1);
                    }
                    ui.label("Manejador 2");
                    if point_editor(ui, &mut c2) {
                        piece.set_control_point(i, 1, c2);
                    }
                    if ui.button("Enderezar manejadores").clicked() {
                        piece.set_edge_kind(i, EdgeKind::Line);
                        piece.make_curve(i);
                    }
                }
                if ui.button("Añadir punto en el centro").clicked() {
                    let id = piece.id;
                    if let Some(v) = self.pattern.split_edge(id, i, 0.5) {
                        self.editor.selection = Selection::Vertex(id, v);
                    }
                }
            }
            Selection::Piece(_) | Selection::None => {}
        }
    }

    fn describe(&self, issue: &Issue) -> String {
        let piece = |id| self.pattern.piece(id).map_or("?".to_owned(), |p| p.name.clone());
        let seam = |id| self.pattern.seams.iter().find(|s| s.id == id).map_or("?".to_owned(), |s| s.name.clone());
        match *issue {
            Issue::TooFewVertices { piece: id } => format!("«{}» tiene menos de 3 puntos", piece(id)),
            Issue::DegeneratePiece { piece: id } => format!("«{}» no tiene área", piece(id)),
            Issue::SelfIntersecting { piece: id } => format!("El contorno de «{}» se cruza consigo mismo", piece(id)),
            Issue::DanglingSeam { seam: id } => format!("La costura «{}» apunta a una arista que no existe", seam(id)),
            Issue::SeamTooFewSides { seam: id } => format!("La costura «{}» tiene menos de dos lados", seam(id)),
            Issue::SeamLengthMismatch { seam: id, min, max } => format!(
                "Los lados de la costura «{}» no miden lo mismo ({:.1}–{:.1} cm)",
                seam(id),
                min / 10.0,
                max / 10.0
            ),
        }
    }

    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("unsaved_changes")).show(ctx, |ui| {
            ui.heading("Hay cambios sin guardar");
            ui.label(format!("¿Quieres guardar «{}» antes de continuar?", self.pattern.name));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Guardar").clicked() {
                    choice = Some(true);
                }
                if ui.button("No guardar").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancelar").clicked() {
                    self.pending = None;
                }
            });
        });
        match choice {
            Some(true) => {
                self.pending = None;
                if self.save() {
                    self.run(action);
                }
            }
            Some(false) => {
                self.pending = None;
                self.run(action);
            }
            None => {}
        }
        if matches!(action, Pending::Quit) && self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let file = self.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned());
        let title = format!(
            "{}{} — clothing-maker",
            if self.is_dirty() { "• " } else { "" },
            file.unwrap_or_else(|| self.pattern.name.clone())
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Editor de un punto en cm. Devuelve `true` si ha cambiado.
fn point_editor(ui: &mut Ui, p: &mut Point) -> bool {
    let (mut x, mut y) = (p.x / 10.0, p.y / 10.0);
    let changed = ui
        .horizontal(|ui| {
            ui.label("x");
            let a = ui.add(egui::DragValue::new(&mut x).speed(0.1).fixed_decimals(1).suffix(" cm")).changed();
            ui.label("y");
            let b = ui.add(egui::DragValue::new(&mut y).speed(0.1).fixed_decimals(1).suffix(" cm")).changed();
            a || b
        })
        .inner;
    if changed {
        *p = Point::new(x * 10.0, y * 10.0);
    }
    changed
}

impl eframe::App for ClothingMakerApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);

        if ctx.input(|i| i.viewport().close_requested()) && self.is_dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }
        // Cualquier clic borra el último mensaje de estado.
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.status = None;
        }

        egui::Panel::top("menu").show(ui, |ui| {
            self.menu_bar(ui);
            self.toolbar(ui);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("properties").resizable(true).default_size(280.0).show(ui, |ui| {
            self.properties(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            self.editor.ui(ui, &mut self.pattern);
        });

        self.unsaved_dialog(&ctx);

        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        if !busy {
            self.history.commit(&self.pattern);
        }
        self.update_title(&ctx);
    }
}
