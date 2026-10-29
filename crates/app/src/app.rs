//! Ventana principal: barra superior, cinta de herramientas, navegador,
//! inspector, vistas 2D y 3D, ficheros y deshacer.

mod panels;
mod textiles;

use std::path::PathBuf;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText, Ui};
use pattern::{Pattern, Point, SeamKind};

use crate::canvas::{self, Editor, Selection, Tool};
use crate::examples;
use crate::history::History;
use crate::icons::{self, Icon};
use crate::theme;
use crate::view3d::View3D;
use crate::widgets::{self, Part};

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
    view3d: View3D,
    tab: Tab,
    layout: ViewLayout,
    /// Resaltar las costuras en 3D en la pestaña Costuras.
    highlight_seams: bool,
    status: Option<(String, bool)>,
    pending: Option<Pending>,
    allow_close: bool,
    title: String,
    /// Tela de la biblioteca que hay que desplegar en el inspector.
    open_textile: Option<pattern::TextileId>,
}

impl ClothingMakerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let pattern = Pattern::new("Sin título");
        let mut app = Self {
            history: History::new(&pattern),
            saved: pattern.clone(),
            pattern,
            path: None,
            editor: Editor::default(),
            view3d: View3D::new(cc),
            tab: Tab::Pattern,
            layout: ViewLayout::Split,
            highlight_seams: true,
            status: None,
            pending: None,
            allow_close: false,
            title: String::new(),
            open_textile: None,
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

    fn undo(&mut self) {
        self.history.undo(&mut self.pattern);
    }

    fn redo(&mut self) {
        self.history.redo(&mut self.pattern);
    }

    fn can_undo(&self) -> bool {
        self.history.can_undo() || self.is_dirty()
    }

    fn set_layout(&mut self, layout: ViewLayout) {
        self.layout = layout;
        if !layout.shows_3d() && self.tab == Tab::Simulation {
            self.tab = Tab::Pattern;
        }
    }

    /// Barra superior: menús, accesos rápidos, el documento abierto y la
    /// disposición de las vistas.
    fn app_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            self.menus(ui);
            ui.spacing_mut().item_spacing.x = 2.0;
            widgets::separator_v(ui, 26.0);
            if widgets::icon_button(ui, Icon::New, "Nuevo (Ctrl+N)", false).clicked() {
                self.request(Pending::New);
            }
            if widgets::icon_button(ui, Icon::Open, "Abrir… (Ctrl+O)", false).clicked() {
                self.request(Pending::Open);
            }
            if widgets::icon_button_enabled(ui, Icon::Save, "Guardar (Ctrl+S)", false, self.is_dirty()).clicked() {
                self.save();
            }
            widgets::separator_v(ui, 26.0);
            if widgets::icon_button_enabled(ui, Icon::Undo, "Deshacer (Ctrl+Z)", false, self.can_undo()).clicked() {
                self.undo();
            }
            if widgets::icon_button_enabled(ui, Icon::Redo, "Rehacer (Ctrl+Mayús+Z)", false, self.history.can_redo())
                .clicked()
            {
                self.redo();
            }

            // El documento abierto, centrado, como una pestaña.
            let bar = ui.max_rect();
            let name = self.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned());
            let name = name.unwrap_or_else(|| self.pattern.name.clone());
            let galley = ui.painter().layout_no_wrap(name, egui::FontId::proportional(12.5), theme::TEXT);
            let tab = egui::Rect::from_center_size(bar.center(), egui::vec2(galley.size().x + 56.0, bar.height()));
            let painter = ui.painter();
            painter.rect_filled(tab, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, theme::RIBBON);
            icons::paint(
                painter,
                egui::Rect::from_center_size(tab.left_center() + egui::vec2(16.0, 0.0), egui::vec2(14.0, 14.0)),
                Icon::Pattern,
                theme::TEXT,
            );
            painter.galley(tab.left_center() + egui::vec2(30.0, -galley.size().y / 2.0), galley, theme::TEXT);
            if self.is_dirty() {
                painter.circle_filled(tab.right_center() - egui::vec2(13.0, 0.0), 3.5, theme::ACCENT);
            }
            if let Some(path) = &self.path {
                ui.interact(tab, ui.scope_id().with("document_tab"), egui::Sense::hover())
                    .on_hover_text(path.display().to_string());
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for layout in ViewLayout::ALL.into_iter().rev() {
                    if widgets::icon_button(ui, layout.icon(), layout.label(), self.layout == layout).clicked() {
                        self.set_layout(layout);
                    }
                }
            });
        });
    }

    fn menus(&mut self, ui: &mut Ui) {
        ui.menu_button("Archivo", |ui| {
            if ui.add(egui::Button::new("Nuevo").shortcut_text(ui.ctx().format_shortcut(&NEW))).clicked() {
                self.request(Pending::New);
            }
            if ui.add(egui::Button::new("Abrir…").shortcut_text(ui.ctx().format_shortcut(&OPEN))).clicked() {
                self.request(Pending::Open);
            }
            ui.menu_button("Ejemplos", |ui| self.examples_menu(ui));
            ui.separator();
            if ui.add(egui::Button::new("Guardar").shortcut_text(ui.ctx().format_shortcut(&SAVE))).clicked() {
                self.save();
            }
            if ui.add(egui::Button::new("Guardar como…").shortcut_text(ui.ctx().format_shortcut(&SAVE_AS))).clicked()
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
            if ui.add_enabled(self.can_undo(), undo).clicked() {
                self.undo();
            }
            let redo = egui::Button::new("Rehacer").shortcut_text(ui.ctx().format_shortcut(&REDO));
            if ui.add_enabled(self.history.can_redo(), redo).clicked() {
                self.redo();
            }
        });
        ui.menu_button("Ver", |ui| {
            if ui.add(egui::Button::new("Encuadrar el patrón").shortcut_text("F")).clicked() {
                self.editor.fit_view();
            }
            ui.checkbox(&mut self.editor.show_mesh, "Mostrar malla de simulación");
            ui.separator();
            for layout in ViewLayout::ALL {
                if ui.radio(self.layout == layout, layout.label()).clicked() {
                    self.set_layout(layout);
                }
            }
        });
    }

    fn examples_menu(&mut self, ui: &mut Ui) {
        for (name, _, make) in examples::ALL {
            if ui.button(name).clicked() {
                self.request(Pending::Example(make));
            }
        }
    }

    /// Cinta de herramientas: pestañas de los espacios de trabajo y, debajo,
    /// los grupos de órdenes del espacio activo.
    fn ribbon(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (tab, label) in [(Tab::Pattern, "Patrón"), (Tab::Seams, "Costuras"), (Tab::Simulation, "Simulación")]
            {
                if widgets::tab(ui, label, self.tab == tab).clicked() {
                    self.tab = tab;
                    if tab == Tab::Simulation && !self.layout.shows_3d() {
                        self.layout = ViewLayout::Split;
                    }
                    if tab == Tab::Seams && self.layout == ViewLayout::Garment {
                        self.layout = ViewLayout::Split;
                    }
                }
            }
        });
        let rect = ui.max_rect();
        ui.painter().hline(rect.x_range(), ui.min_rect().bottom(), egui::Stroke::new(1.0, theme::BORDER));
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            // La misma altura en todas las pestañas, para que las vistas no salten.
            ui.set_min_height(80.0);
            match self.tab {
                Tab::Pattern => self.pattern_ribbon(ui),
                Tab::Seams => self.seams_ribbon(ui),
                Tab::Simulation => {
                    self.view3d.ribbon(ui, &self.pattern, canvas::piece_color);
                    self.fill_group(ui);
                }
            }
        });
    }

    fn tool_button(&mut self, ui: &mut Ui, tool: Tool, icon: Icon, label: &str, key: &str) {
        let response = widgets::ribbon_button(ui, icon, label, self.editor.tool == tool, true)
            .on_hover_text(format!("{label} ({key})\n\n{}", tool.hint()));
        if response.clicked() {
            self.editor.set_tool(tool);
        }
    }

    fn pattern_ribbon(&mut self, ui: &mut Ui) {
        widgets::ribbon_group(ui, "SELECCIONAR", false, |ui, _| {
            self.tool_button(ui, Tool::Select, Icon::Select, "Seleccionar", "V");
        });
        widgets::ribbon_group(ui, "CREAR", true, |ui, part| match part {
            Part::Buttons => {
                self.tool_button(ui, Tool::Pen, Icon::Pen, "Pluma", "P");
                self.tool_button(ui, Tool::Rect, Icon::Rect, "Rectángulo", "R");
            }
            Part::Menu => {
                if ui.button("Rectángulo de 30 × 40 cm en el centro").clicked() {
                    self.add_rect();
                    ui.close();
                }
                ui.separator();
                ui.menu_button("Empezar desde un ejemplo", |ui| self.examples_menu(ui));
            }
        });
        let selection = self.editor.selection;
        widgets::ribbon_group(ui, "MODIFICAR", true, |ui, part| match part {
            Part::Buttons => {
                let piece = selection.piece().filter(|_| !matches!(selection, Selection::Seam(_)));
                if widgets::ribbon_button(ui, Icon::Duplicate, "Duplicar", false, piece.is_some())
                    .on_hover_text("Duplicar la pieza seleccionada (Ctrl+D)")
                    .clicked()
                    && let Some(id) = piece
                    && let Some(new) = self.pattern.duplicate_piece(id, pattern::Vec2::new(20.0, 20.0))
                {
                    self.editor.selection = Selection::Piece(new);
                }
                if widgets::ribbon_button(ui, Icon::Delete, "Borrar", false, selection != Selection::None)
                    .on_hover_text("Borrar la selección (Supr)")
                    .clicked()
                {
                    self.editor.delete_selection(&mut self.pattern);
                }
            }
            Part::Menu => match selection {
                Selection::Edge(id, i) => {
                    let Some(piece) = self.pattern.piece_mut(id) else { return };
                    if piece.edges()[i].is_curve() {
                        if ui.add(egui::Button::new("Convertir en recta").shortcut_text("L")).clicked() {
                            piece.make_line(i);
                        }
                    } else if ui.add(egui::Button::new("Convertir en curva").shortcut_text("C")).clicked() {
                        piece.make_curve(i);
                    }
                    if ui.button("Añadir punto en el centro").clicked()
                        && let Some(v) = self.pattern.split_edge(id, i, 0.5)
                    {
                        self.editor.selection = Selection::Vertex(id, v);
                    }
                }
                _ => {
                    ui.weak("Selecciona una arista para convertirla en curva o recta, o partirla.");
                }
            },
        });
        widgets::ribbon_group(ui, "COSER", false, |ui, _| {
            self.tool_button(ui, Tool::Seam, Icon::Seam, "Costura", "S");
        });
        widgets::ribbon_group(ui, "INSPECCIONAR", true, |ui, part| match part {
            Part::Buttons => {
                if widgets::ribbon_button(ui, Icon::Grid, "Rejilla", self.editor.snap, true)
                    .on_hover_text("Ajustar los puntos a la rejilla (Alt para soltarlos)")
                    .clicked()
                {
                    self.editor.snap = !self.editor.snap;
                }
                if widgets::ribbon_button(ui, Icon::Mesh, "Malla", self.editor.show_mesh, true)
                    .on_hover_text("Mostrar la malla de simulación sobre las piezas")
                    .clicked()
                {
                    self.editor.show_mesh = !self.editor.show_mesh;
                }
            }
            Part::Menu => {
                widgets::properties(ui, "inspect", |ui| {
                    widgets::prop_label(ui, "Paso de rejilla");
                    ui.add(
                        egui::DragValue::new(&mut self.editor.snap_step).range(0.5..=100.0).speed(0.5).suffix(" mm"),
                    );
                    ui.end_row();
                    widgets::prop_label(ui, "Triángulos");
                    ui.add(
                        egui::DragValue::new(&mut self.editor.mesh_size).range(3.0..=200.0).speed(0.5).suffix(" mm"),
                    );
                    ui.end_row();
                });
            }
        });
        widgets::ribbon_group(ui, "PRENDA", false, |ui, _| {
            if widgets::ribbon_button(ui, Icon::Mannequin, "Maniquí", self.pattern.on_avatar, true)
                .on_hover_text(
                    "Prenda para el maniquí: la vista 3D la viste sobre él. Quítalo para objetos como bolsas.",
                )
                .clicked()
            {
                self.pattern.on_avatar = !self.pattern.on_avatar;
            }
        });
        widgets::ribbon_group(ui, "VISTA", false, |ui, _| {
            if widgets::ribbon_button(ui, Icon::Fit, "Encuadrar", false, true)
                .on_hover_text("Encuadrar el patrón (F)")
                .clicked()
            {
                self.editor.fit_view();
            }
        });
    }

    /// Cinta de la pestaña Costuras: crear costuras, su tipo, el margen, el
    /// orden de montaje y dar la vuelta a la prenda.
    fn seams_ribbon(&mut self, ui: &mut Ui) {
        widgets::ribbon_group(ui, "CREAR", false, |ui, _| {
            self.tool_button(ui, Tool::Select, Icon::Select, "Seleccionar", "V");
            self.tool_button(ui, Tool::Seam, Icon::Seam, "Costura", "S");
        });
        let selected = match self.editor.selection {
            Selection::Seam(id) => self.pattern.seams.iter().position(|s| s.id == id),
            _ => None,
        };
        widgets::ribbon_group(ui, "TIPO", true, |ui, part| match part {
            Part::Buttons => {
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    let Some(k) = selected else {
                        ui.add_enabled(false, egui::Button::new("Selecciona una costura"));
                        return;
                    };
                    let seam = &mut self.pattern.seams[k];
                    let mut kind = seam.kind;
                    egui::ComboBox::from_id_salt("ribbon_seam_kind").width(130.0).selected_text(kind.label()).show_ui(
                        ui,
                        |ui| {
                            for option in SeamKind::ALL {
                                ui.selectable_value(&mut kind, option, option.label()).on_hover_text(option.hint());
                            }
                        },
                    );
                    if kind != seam.kind {
                        seam.set_kind(kind);
                    }
                });
            }
            Part::Menu => {
                let kind = selected.map(|k| self.pattern.seams[k].kind);
                let label = kind.map_or("Aplica un tipo a todas las costuras".to_owned(), |k| {
                    format!("Hacer todas las costuras de tipo «{}»", k.label())
                });
                if ui.add_enabled(kind.is_some(), egui::Button::new(label)).clicked()
                    && let Some(kind) = kind
                {
                    for seam in &mut self.pattern.seams {
                        seam.set_kind(kind);
                    }
                    ui.close();
                }
            }
        });
        widgets::ribbon_group(ui, "MARGEN", true, |ui, part| match part {
            Part::Buttons => {
                if widgets::ribbon_button(ui, Icon::Allowance, "Línea de corte", self.editor.show_allowance, true)
                    .on_hover_text("Dibujar el margen de costura alrededor de las piezas")
                    .clicked()
                {
                    self.editor.show_allowance = !self.editor.show_allowance;
                }
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    widgets::prop_label(ui, "Por defecto");
                    ui.add(
                        egui::DragValue::new(&mut self.pattern.seam_allowance)
                            .range(0.0..=50.0)
                            .speed(0.5)
                            .suffix(" mm"),
                    )
                    .on_hover_text(
                        "Margen de los bordes que no tienen uno propio (selecciona una arista para darle otro)",
                    );
                });
            }
            Part::Menu => {
                if ui.button("Quitar los márgenes propios de todos los bordes").clicked() {
                    for piece in &mut self.pattern.pieces {
                        for i in 0..piece.len() {
                            piece.set_edge_allowance(i, None);
                        }
                    }
                    ui.close();
                }
                if ui.button("Usar en cada costura el margen típico de su tipo").clicked() {
                    let typical: Vec<_> = self
                        .pattern
                        .seams
                        .iter()
                        .flat_map(|s| {
                            s.sides.iter().map(move |side| (side.piece, side.edge, s.kind.typical_allowance()))
                        })
                        .collect();
                    for (piece, edge, allowance) in typical {
                        if let Some(piece) = self.pattern.piece_mut(piece)
                            && let Some(i) = piece.edge_index(edge)
                        {
                            piece.set_edge_allowance(i, Some(allowance));
                        }
                    }
                    ui.close();
                }
            }
        });
        widgets::ribbon_group(ui, "ORDEN", false, |ui, _| {
            let up = selected.is_some_and(|k| k > 0);
            let down = selected.is_some_and(|k| k + 1 < self.pattern.seams.len());
            ui.vertical(|ui| {
                ui.add_space(4.0);
                if ui.add_enabled(up, egui::Button::new("▲ Antes")).on_hover_text("Coser antes esta costura").clicked()
                    && let Selection::Seam(id) = self.editor.selection
                {
                    self.pattern.move_seam(id, -1);
                }
                if ui
                    .add_enabled(down, egui::Button::new("▼ Después"))
                    .on_hover_text("Coser después esta costura")
                    .clicked()
                    && let Selection::Seam(id) = self.editor.selection
                {
                    self.pattern.move_seam(id, 1);
                }
            });
            let in_order = self.view3d.settings.sew_in_order;
            if widgets::ribbon_button(ui, Icon::Order, "Coser por orden", in_order, true)
                .on_hover_text(
                    "En la vista 3D, cerrar las costuras de una en una en el orden de la lista \
                     (si no, se cierran todas a la vez)",
                )
                .clicked()
            {
                self.view3d.set_sew_in_order(!in_order, &self.pattern, canvas::piece_color);
            }
        });
        widgets::ribbon_group(ui, "PRENDA", false, |ui, _| {
            if widgets::ribbon_button(ui, Icon::Flip, "Voltear", self.pattern.inside_out, true)
                .on_hover_text(
                    "Dar la vuelta a la prenda: el derecho de la tela queda dentro y los márgenes de las \
                     costuras, fuera (como al coserla)",
                )
                .clicked()
            {
                self.pattern.inside_out = !self.pattern.inside_out;
            }
        });
        widgets::ribbon_group(ui, "VISTA", false, |ui, _| {
            if widgets::ribbon_button(ui, Icon::Highlight, "Resaltar", self.highlight_seams, true)
                .on_hover_text("Marcar en 3D cada costura con su color")
                .clicked()
            {
                self.highlight_seams = !self.highlight_seams;
            }
            if widgets::ribbon_button(ui, Icon::Fit, "Encuadrar", false, true)
                .on_hover_text("Encuadrar el patrón (F)")
                .clicked()
            {
                self.editor.fit_view();
            }
        });
    }

    /// Relleno (presión interior) del patrón, también a mano en la cinta de simulación.
    fn fill_group(&mut self, ui: &mut Ui) {
        widgets::ribbon_group(ui, "RELLENO", false, |ui, _| {
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.add(egui::Slider::new(&mut self.pattern.pressure, 0.0..=60.0).suffix(" Pa").max_decimals(0))
                    .on_hover_text(
                        "Presión interior: hincha la prenda como si estuviera rellena. 0 = vacía; \
                         unos 10 Pa llenan una bolsa y 30 dejan firme un cojín.",
                    );
            });
        });
    }

    fn add_rect(&mut self) {
        let c = self.editor.view_center();
        let name = format!("Pieza {}", self.pattern.pieces.len() + 1);
        let id = self.pattern.add_rect(name, Point::new(c.x - 150.0, c.y - 200.0), 300.0, 400.0);
        self.editor.selection = Selection::Piece(id);
    }

    fn status_bar(&self, ui: &mut Ui) {
        // Primero lo de la derecha, para que el mensaje se recorte si no cabe.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if self.layout.shows_2d() {
                ui.monospace(format!("{:.0} %", self.editor.zoom_percent()));
                if let Some(c) = self.editor.cursor {
                    ui.monospace(format!("x {:>7.1}  y {:>7.1} cm", c.x / 10.0, c.y / 10.0));
                }
            }
            if self.layout.shows_3d() {
                widgets::separator_v(ui, 18.0);
                ui.horizontal(|ui| self.view3d.status(ui, &self.pattern));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let text = match &self.status {
                    Some((msg, true)) => RichText::new(msg).color(theme::ERROR),
                    Some((msg, false)) => RichText::new(msg).color(theme::OK),
                    None => RichText::new(self.tool_hint()).color(theme::TEXT_WEAK),
                };
                ui.add(egui::Label::new(text).truncate());
            });
        });
    }

    /// El lienzo 2D, con su barra de navegación.
    fn pattern_view(&mut self, ui: &mut Ui) {
        self.editor.ui(ui, &mut self.pattern);
        let rect = self.editor.rect();
        widgets::viewport_label(ui, rect, "Patrón 2D");
        widgets::nav_bar(ui, rect, "2d", |ui| {
            if widgets::icon_button(ui, Icon::Fit, "Encuadrar (F)", false).clicked() {
                self.editor.fit_view();
            }
            widgets::separator_v(ui, 24.0);
            if widgets::icon_button(ui, Icon::ZoomOut, "Alejar", false).clicked() {
                self.editor.zoom_by(1.0 / 1.25);
            }
            ui.add_sized(
                egui::vec2(46.0, 22.0),
                egui::Label::new(RichText::new(format!("{:.0} %", self.editor.zoom_percent())).size(11.5)),
            );
            if widgets::icon_button(ui, Icon::ZoomIn, "Acercar", false).clicked() {
                self.editor.zoom_by(1.25);
            }
            widgets::separator_v(ui, 24.0);
            if widgets::icon_button(ui, Icon::Grid, "Ajustar a la rejilla", self.editor.snap).clicked() {
                self.editor.snap = !self.editor.snap;
            }
            if widgets::icon_button(ui, Icon::Mesh, "Mostrar la malla de simulación", self.editor.show_mesh).clicked()
            {
                self.editor.show_mesh = !self.editor.show_mesh;
            }
        });
    }

    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("unsaved_changes")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                icons::paint(ui.painter(), rect, Icon::Warning, theme::WARN);
                ui.heading("Hay cambios sin guardar");
            });
            ui.add_space(4.0);
            ui.label(format!("¿Quieres guardar «{}» antes de continuar?", self.pattern.name));
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancelar").clicked() {
                    self.pending = None;
                }
                if ui.button("No guardar").clicked() {
                    choice = Some(false);
                }
                let save = egui::Button::new(RichText::new("Guardar").color(egui::Color32::WHITE)).fill(theme::ACCENT);
                if ui.add(save).clicked() {
                    choice = Some(true);
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

        let bar = |fill| egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(6, 3));
        egui::Panel::top("app_bar").frame(bar(theme::APP_BAR)).show(ui, |ui| self.app_bar(ui));
        egui::Panel::top("ribbon")
            .frame(egui::Frame::NONE.fill(theme::RIBBON).inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 0,
                bottom: 2,
            }))
            .show(ui, |ui| self.ribbon(ui));
        egui::Panel::bottom("status").frame(bar(theme::HEADER)).show(ui, |ui| self.status_bar(ui));
        let side = egui::Frame::NONE.fill(theme::PANEL);
        egui::Panel::left("browser")
            .resizable(true)
            .default_size(230.0)
            .size_range(170.0..=420.0)
            .frame(side)
            .show(ui, |ui| self.browser(ui));
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(290.0)
            .size_range(220.0..=480.0)
            .frame(side)
            .show(ui, |ui| self.inspector(ui));

        self.editor.focus_seams = self.tab == Tab::Seams;
        self.view3d.highlight_seams = self.tab == Tab::Seams && self.highlight_seams;
        if self.layout.shows_3d() {
            self.view3d.sync(&ctx, &self.pattern, canvas::piece_color);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| match self.layout {
            ViewLayout::Pattern => self.pattern_view(ui),
            ViewLayout::Garment => self.view3d.viewport(ui, &self.pattern, canvas::piece_color),
            ViewLayout::Split => {
                let half = ui.available_width() / 2.0;
                egui::Panel::right("view3d")
                    .resizable(true)
                    .default_size(half)
                    .size_range(200.0..=ui.available_width() - 200.0)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.view3d.viewport(ui, &self.pattern, canvas::piece_color));
                self.pattern_view(ui);
            }
        });

        self.unsaved_dialog(&ctx);

        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        if !busy {
            self.history.commit(&self.pattern);
        }
        self.update_title(&ctx);
    }
}

/// Espacio de trabajo de la cinta.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Pattern,
    Seams,
    Simulation,
}

/// Qué vistas se ven en el centro.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewLayout {
    Pattern,
    Split,
    Garment,
}

impl ViewLayout {
    const ALL: [ViewLayout; 3] = [ViewLayout::Pattern, ViewLayout::Split, ViewLayout::Garment];

    fn label(self) -> &'static str {
        match self {
            ViewLayout::Pattern => "Solo el patrón 2D",
            ViewLayout::Split => "Patrón 2D y vista 3D",
            ViewLayout::Garment => "Solo la vista 3D",
        }
    }

    fn icon(self) -> Icon {
        match self {
            ViewLayout::Pattern => Icon::Layout2D,
            ViewLayout::Split => Icon::LayoutSplit,
            ViewLayout::Garment => Icon::Layout3D,
        }
    }

    fn shows_2d(self) -> bool {
        self != ViewLayout::Garment
    }

    fn shows_3d(self) -> bool {
        self != ViewLayout::Pattern
    }
}
