//! Paneles laterales: el navegador (árbol de piezas, costuras y
//! comprobaciones de cada etapa) a la izquierda y el inspector de
//! propiedades de la selección a la derecha, con lo que importa en la etapa
//! activa.

use eframe::egui::{self, Color32, RichText, Ui};
use pattern::{Arrangement, BodyZone, EdgeKind, Point, SeamId, SeamKind, Topstitch};

use super::ClothingMakerApp;
use crate::canvas::{self, Selection};
use crate::icons::Icon;
use crate::section;
use crate::stages::{self, Stage};
use crate::theme;
use crate::widgets;

/// Algo que se puede hacer desde una fila del navegador.
enum TreeAction {
    Select(Selection),
    /// Ir a una etapa y seleccionar lo que tiene pendiente.
    Check(Stage, Option<Selection>),
    Duplicate(pattern::PieceId),
    DeletePiece(pattern::PieceId),
    DeleteSeam(SeamId),
}

impl ClothingMakerApp {
    pub(super) fn browser(&mut self, ui: &mut Ui) {
        widgets::panel_title(ui, "Navegador");
        let mut action = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let kind = if self.pattern.on_avatar { "prenda" } else { "objeto" };
            ui.add_space(4.0);
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                widgets::tree_icon_row(ui, Icon::Pattern, &self.pattern.name, Some(kind));
            });
            ui.add_space(4.0);

            self.checks_section(ui, &mut action);

            let selected = self.editor.selection;
            widgets::section(ui, "Piezas", Some(self.pattern.pieces.len()), |ui| {
                for piece in &self.pattern.pieces {
                    // Lo que importa de cada pieza en la etapa.
                    let (color, detail) = match self.stage {
                        _ if piece.is_hole() => (canvas::piece_color(piece.id), Some("agujero".to_owned())),
                        Stage::Fabrics => {
                            let name = self.pattern.textile(piece.fabric).map_or("?", |t| t.name.as_str());
                            let detail = match piece.layers.len() {
                                0 => name.to_owned(),
                                n => format!("{name} +{n}"),
                            };
                            (canvas::textile_color(piece.fabric), Some(detail))
                        }
                        Stage::Placement | Stage::Simulation => {
                            (canvas::piece_color(piece.id), Some(stages::slot_label(&self.pattern, piece)))
                        }
                        _ => (
                            canvas::piece_color(piece.id),
                            self.pattern.on_avatar.then(|| piece.zone.label().to_owned()),
                        ),
                    };
                    let response = widgets::tree_row(
                        ui,
                        color,
                        &piece.name,
                        detail.as_deref(),
                        selected.piece() == Some(piece.id) && !matches!(selected, Selection::Seam(_)),
                        0,
                    );
                    if response.clicked() {
                        action = Some(TreeAction::Select(Selection::Piece(piece.id)));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Duplicar").clicked() {
                            action = Some(TreeAction::Duplicate(piece.id));
                        }
                        if ui.button("Borrar").clicked() {
                            action = Some(TreeAction::DeletePiece(piece.id));
                        }
                    });
                }
                if self.pattern.pieces.is_empty() {
                    ui.add_space(4.0);
                    ui.weak("Aún no hay piezas. Usa la pluma (P) o el rectángulo (R).");
                }
            });

            widgets::section(ui, "Costuras", Some(self.pattern.seams.len()), |ui| {
                for (k, seam) in self.pattern.seams.iter().enumerate() {
                    // Lo normal son dos lados: solo se indica cuando hay más.
                    let detail = match seam.sides.len() {
                        2 => seam.kind.label().to_owned(),
                        n => format!("{} · {n} lados", seam.kind.label()),
                    };
                    let detail = Some(detail);
                    let response = widgets::tree_row(
                        ui,
                        canvas::seam_color(k),
                        &format!("{}. {}", k + 1, seam.name),
                        detail.as_deref(),
                        selected == Selection::Seam(seam.id),
                        0,
                    );
                    let pieces: Vec<&str> = seam
                        .sides
                        .iter()
                        .filter_map(|s| self.pattern.piece(s.piece).map(|p| p.name.as_str()))
                        .collect();
                    let response = response.on_hover_text(pieces.join(" + "));
                    if response.clicked() {
                        action = Some(TreeAction::Select(Selection::Seam(seam.id)));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Borrar costura").clicked() {
                            action = Some(TreeAction::DeleteSeam(seam.id));
                        }
                    });
                }
                if self.pattern.seams.is_empty() {
                    ui.add_space(4.0);
                    ui.weak("Aún no hay costuras. Usa la herramienta Costura (S).");
                }
            });
        });

        match action {
            Some(TreeAction::Select(selection)) => self.editor.selection = selection,
            Some(TreeAction::Check(stage, target)) => {
                self.set_stage(stage);
                if let Some(target) = target {
                    self.editor.selection = target;
                }
            }
            Some(TreeAction::Duplicate(id)) => {
                if let Some(new) = self.pattern.duplicate_piece(id, pattern::Vec2::new(20.0, 20.0)) {
                    self.editor.selection = Selection::Piece(new);
                }
            }
            Some(TreeAction::DeletePiece(id)) => {
                self.pattern.remove_piece(id);
                self.editor.selection = Selection::None;
            }
            Some(TreeAction::DeleteSeam(id)) => {
                self.pattern.remove_seam(id);
                self.editor.selection = Selection::None;
            }
            None => {}
        }
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        widgets::panel_title(ui, "Propiedades");
        let stage = self.stage;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add_space(4.0);
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 0)).show(ui, |ui| {
                ui.label(RichText::new(format!("{}. {}", stage.number(), stage.label())).strong());
                ui.label(RichText::new(stage.hint()).size(11.0).color(theme::TEXT_WEAK));
            });
            ui.add_space(4.0);
            self.selection_details(ui);
            if stage == Stage::Fabrics {
                self.textile_library(ui);
            }
            widgets::section(ui, "Patrón", None, |ui| {
                widgets::properties(ui, "pattern", |ui| {
                    widgets::prop_label(ui, "Nombre");
                    ui.text_edit_singleline(&mut self.pattern.name);
                    ui.end_row();
                    // El tipo de prenda se decide al dibujar o al colocar.
                    if matches!(stage, Stage::Pattern | Stage::Placement) {
                        widgets::prop_label(ui, "Tipo");
                        ui.checkbox(&mut self.pattern.on_avatar, "Prenda para el maniquí").on_hover_text(
                            "La vista 3D la viste sobre el maniquí y cada pieza se coloca en su zona del cuerpo. \
                             Quítalo para objetos como bolsas o mochilas.",
                        );
                        ui.end_row();
                    }
                    if stage == Stage::Assembly {
                        widgets::prop_label(ui, "Margen");
                        ui.add(
                            egui::DragValue::new(&mut self.pattern.seam_allowance)
                                .range(0.0..=50.0)
                                .speed(0.5)
                                .suffix(" mm"),
                        )
                        .on_hover_text("Margen de costura de los bordes que no tienen uno propio");
                        ui.end_row();
                        widgets::prop_label(ui, "Volteada");
                        ui.checkbox(&mut self.pattern.inside_out, "Del revés").on_hover_text(
                            "La prenda está del revés: el derecho de la tela queda dentro y los márgenes, fuera",
                        );
                        ui.end_row();
                    }
                    if stage == Stage::Simulation {
                        widgets::prop_label(ui, "Relleno");
                        ui.add(egui::Slider::new(&mut self.pattern.pressure, 0.0..=60.0).suffix(" Pa").max_decimals(0))
                            .on_hover_text(
                                "Presión interior en la vista 3D: hincha la prenda como si estuviera rellena. \
                             0 = vacía (una bolsa sin nada dentro queda plana). Con unos 10 Pa una bolsa se llena \
                             y con 30 un cojín queda firme; mucho más y se pone redonda como un globo. Las bocas \
                             sin coser se abren.",
                            );
                        ui.end_row();
                    }
                    widgets::prop_label(ui, "Piezas");
                    ui.label(self.pattern.pieces.len().to_string());
                    ui.end_row();
                    widgets::prop_label(ui, "Costuras");
                    ui.label(self.pattern.seams.len().to_string());
                    ui.end_row();
                });
            });
        });
    }

    fn selection_details(&mut self, ui: &mut Ui) {
        let selection = self.editor.selection;
        if let Selection::Seam(id) = selection {
            self.seam_details(ui, id);
            return;
        }
        let on_avatar = self.pattern.on_avatar;
        let stage = self.stage;
        let Some(id) = selection.piece() else {
            let hint = match stage {
                Stage::Pattern => "Selecciona una pieza, un punto o una arista para ver sus propiedades.",
                Stage::Fabrics => "Selecciona una pieza para elegir su tela y sus capas.",
                Stage::Assembly => "Selecciona una costura, o una arista para ver su margen y sus costuras.",
                Stage::Placement => {
                    "Selecciona una pieza para decidir dónde empieza en 3D: su lado, su zona y su cara."
                }
                Stage::Simulation => "Selecciona una pieza o una costura para ver sus datos.",
            };
            widgets::section(ui, "Selección", None, |ui| {
                ui.add_space(2.0);
                ui.weak(hint);
            });
            return;
        };
        let places = stage == Stage::Placement;
        // Piezas de tela de las que esta podría ser un agujero.
        let parents: Vec<(pattern::PieceId, String)> =
            self.pattern.pieces.iter().filter(|p| p.id != id && !p.is_hole()).map(|p| (p.id, p.name.clone())).collect();
        let has_holes = self.pattern.holes_of(id).next().is_some();
        let Some(piece) = self.pattern.piece_mut(id) else { return };
        let mut duplicate = false;
        let mut delete = false;
        widgets::section(ui, "Pieza", None, |ui| {
            widgets::properties(ui, "piece", |ui| {
                widgets::prop_label(ui, "Nombre");
                ui.text_edit_singleline(&mut piece.name);
                ui.end_row();
                widgets::prop_label(ui, "Área");
                ui.label(format!("{:.1} cm²", piece.area().abs() / 100.0));
                ui.end_row();
                widgets::prop_label(ui, "Perímetro");
                ui.label(format!("{:.1} cm", piece.perimeter() / 10.0));
                ui.end_row();
                widgets::prop_label(ui, "Puntos");
                ui.label(piece.len().to_string());
                ui.end_row();
                if stage == Stage::Pattern && !has_holes {
                    widgets::prop_label(ui, "Agujero en").on_hover_text(
                        "Convierte la pieza en un agujero recortado de otra (una ventana para una cremallera, un \
                         asa troquelada). Tiene que quedar del todo dentro de ella.",
                    );
                    let name = |id: Option<pattern::PieceId>| match id {
                        None => "Ninguna: es tela".to_owned(),
                        Some(id) => parents.iter().find(|p| p.0 == id).map_or("?".to_owned(), |p| p.1.clone()),
                    };
                    egui::ComboBox::from_id_salt("hole_in").selected_text(name(piece.hole_in)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut piece.hole_in, None, name(None));
                        for (id, label) in &parents {
                            ui.selectable_value(&mut piece.hole_in, Some(*id), label);
                        }
                    });
                    ui.end_row();
                }
                if !places {
                    return;
                }
                widgets::prop_label(ui, "Colocación 3D").on_hover_text(
                    "Dónde va la pieza al coser en la vista 3D (modo Alrededor), mirando la prenda de frente. \
                     Dibuja cada pieza como se ve desde fuera.",
                );
                egui::ComboBox::from_id_salt("arrangement").selected_text(piece.arrangement.label()).show_ui(
                    ui,
                    |ui| {
                        for a in Arrangement::ALL {
                            ui.selectable_value(&mut piece.arrangement, a, a.label());
                        }
                    },
                );
                ui.end_row();
                widgets::prop_label(ui, "Cara");
                ui.checkbox(&mut piece.right_side_in, "Derecho hacia dentro").on_hover_text(
                    "Para forros y vistas: la pieza se dibuja igual (vista desde fuera), pero lo que queda hacia \
                     fuera es el revés de la tela.",
                );
                ui.end_row();
                if on_avatar {
                    widgets::prop_label(ui, "Zona del cuerpo").on_hover_text(
                        "Alrededor de qué parte del maniquí se coloca: el torso, un brazo (mangas) o una pierna. \
                         Derecha e izquierda son las del maniquí.",
                    );
                    egui::ComboBox::from_id_salt("zone").selected_text(piece.zone.label()).show_ui(ui, |ui| {
                        for zone in BodyZone::ALL {
                            ui.selectable_value(&mut piece.zone, zone, zone.label());
                        }
                    });
                    ui.end_row();
                }
            });
            if stage == Stage::Pattern {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    duplicate = ui.button("Duplicar").on_hover_text("Ctrl+D").clicked();
                    delete = ui.button("Borrar pieza").clicked();
                });
            }
        });

        match selection {
            Selection::Vertex(_, i) if stage == Stage::Pattern => {
                let mut remove = false;
                widgets::section(ui, &format!("Punto {}", i + 1), None, |ui| {
                    widgets::properties(ui, "vertex", |ui| {
                        widgets::prop_label(ui, "Posición");
                        let mut p = piece.vertices()[i];
                        if point_editor(ui, &mut p) {
                            piece.set_vertex(i, p);
                        }
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    remove = ui.add_enabled(piece.len() > 3, egui::Button::new("Borrar punto")).clicked();
                });
                if remove {
                    self.editor.delete_selection(&mut self.pattern);
                    return;
                }
            }
            Selection::Edge(_, i) if matches!(stage, Stage::Pattern | Stage::Assembly) => {
                let shape = stage == Stage::Pattern;
                let mut split = false;
                let mut go_to = None;
                let edge = piece.edges()[i].id;
                let default_allowance = self.pattern.seam_allowance;
                let Some(piece) = self.pattern.piece_mut(id) else { return };
                widgets::section(ui, &format!("Arista {}", i + 1), None, |ui| {
                    widgets::properties(ui, "edge", |ui| {
                        widgets::prop_label(ui, "Longitud");
                        ui.label(format!("{:.1} cm", piece.edge_length(i) / 10.0));
                        ui.end_row();
                        if !shape {
                            widgets::prop_label(ui, "Margen");
                            allowance_editor(ui, piece, i, default_allowance);
                            ui.end_row();
                            return;
                        }
                        widgets::prop_label(ui, "Tipo");
                        let mut curve = piece.edges()[i].is_curve();
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut curve, false, "Recta");
                            ui.selectable_value(&mut curve, true, "Curva");
                        });
                        if curve != piece.edges()[i].is_curve() {
                            if curve { piece.make_curve(i) } else { piece.make_line(i) }
                        }
                        ui.end_row();
                        if let Some((mut c1, mut c2)) = piece.control_points(i) {
                            widgets::prop_label(ui, "Manejador 1");
                            if point_editor(ui, &mut c1) {
                                piece.set_control_point(i, 0, c1);
                            }
                            ui.end_row();
                            widgets::prop_label(ui, "Manejador 2");
                            if point_editor(ui, &mut c2) {
                                piece.set_control_point(i, 1, c2);
                            }
                            ui.end_row();
                        }
                    });
                    if !shape {
                        return;
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if piece.edges()[i].is_curve() && ui.button("Enderezar manejadores").clicked() {
                            piece.set_edge_kind(i, EdgeKind::Line);
                            piece.make_curve(i);
                        }
                        split = ui.button("Añadir punto en el centro").clicked();
                    });
                });
                let seams: Vec<(usize, SeamId, String)> = self
                    .pattern
                    .seams
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.sides.iter().any(|side| side.piece == id && side.edge == edge))
                    .map(|(k, s)| (k, s.id, s.name.clone()))
                    .collect();
                if !seams.is_empty() && !shape {
                    widgets::section(ui, "Costuras de la arista", Some(seams.len()), |ui| {
                        for (k, seam, name) in seams {
                            if widgets::tree_row(ui, canvas::seam_color(k), &name, None, false, 0).clicked() {
                                go_to = Some(seam);
                            }
                        }
                    });
                }
                if split && let Some(v) = self.pattern.split_edge(id, i, 0.5) {
                    self.editor.selection = Selection::Vertex(id, v);
                }
                if let Some(seam) = go_to {
                    self.editor.selection = Selection::Seam(seam);
                }
            }
            Selection::Line(_, k) if matches!(stage, Stage::Pattern | Stage::Assembly) => {
                self.line_details(ui, id, k);
            }
            _ => {}
        }

        if stage == Stage::Fabrics {
            self.piece_textiles(ui, id);
        }

        if duplicate && let Some(new) = self.pattern.duplicate_piece(id, pattern::Vec2::new(20.0, 20.0)) {
            self.editor.selection = Selection::Piece(new);
        } else if delete {
            self.pattern.remove_piece(id);
            self.editor.selection = Selection::None;
        }
    }

    /// Una línea interior: su largo, sus extremos (al dibujar) y las
    /// costuras que van sobre ella (al montar).
    fn line_details(&mut self, ui: &mut Ui, id: pattern::PieceId, k: usize) {
        let shape = self.stage == Stage::Pattern;
        let Some(line) = self.pattern.piece(id).and_then(|p| p.lines.get(k)).copied() else { return };
        let seams: Vec<(usize, SeamId, String)> = self
            .pattern
            .seams
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sides.iter().any(|side| side.piece == id && side.edge == line.id))
            .map(|(n, s)| (n, s.id, s.name.clone()))
            .collect();
        let mut edited = line;
        let mut delete = false;
        let mut go_to = None;
        widgets::section(ui, "Línea interior", None, |ui| {
            widgets::properties(ui, "line", |ui| {
                widgets::prop_label(ui, "Longitud");
                ui.label(format!("{:.1} cm", line.length() / 10.0));
                ui.end_row();
                if shape {
                    widgets::prop_label(ui, "Desde");
                    point_editor(ui, &mut edited.from);
                    ui.end_row();
                    widgets::prop_label(ui, "Hasta");
                    point_editor(ui, &mut edited.to);
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Cose encima otra pieza (un bolsillo, un asa) eligiendo esta línea con la herramienta Costura.",
                )
                .size(11.0)
                .color(theme::TEXT_WEAK),
            );
            if shape {
                ui.add_space(6.0);
                delete = ui.button("Borrar línea").clicked();
            }
        });
        if !shape && !seams.is_empty() {
            widgets::section(ui, "Costuras de la línea", Some(seams.len()), |ui| {
                for (n, seam, name) in &seams {
                    if widgets::tree_row(ui, canvas::seam_color(*n), name, None, false, 0).clicked() {
                        go_to = Some(*seam);
                    }
                }
            });
        }
        if edited != line
            && let Some(slot) = self.pattern.piece_mut(id).and_then(|p| p.lines.get_mut(k))
        {
            *slot = edited;
        }
        if delete {
            self.pattern.remove_line(id, line.id);
            self.editor.selection = Selection::Piece(id);
        }
        if let Some(seam) = go_to {
            self.editor.selection = Selection::Seam(seam);
        }
    }

    fn seam_details(&mut self, ui: &mut Ui, id: SeamId) {
        let Some(k) = self.pattern.seams.iter().position(|s| s.id == id) else { return };
        let sides = self.pattern.seams[k].sides.clone();
        let names: Vec<String> =
            sides.iter().map(|s| self.pattern.piece(s.piece).map_or("?".to_owned(), |p| p.name.clone())).collect();
        let count = self.pattern.seams.len();
        let mut flip = None;
        let mut remove = None;
        let mut delete = false;
        let mut move_by = 0;
        widgets::section(ui, &format!("Costura {}", k + 1), None, |ui| {
            let seam = &mut self.pattern.seams[k];
            widgets::properties(ui, "seam", |ui| {
                widgets::prop_label(ui, "Nombre");
                ui.text_edit_singleline(&mut seam.name);
                ui.end_row();
                widgets::prop_label(ui, "Color");
                let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, canvas::seam_color(k));
                ui.end_row();
                widgets::prop_label(ui, "Tipo");
                let mut kind = seam.kind;
                egui::ComboBox::from_id_salt("seam_kind").selected_text(kind.label()).show_ui(ui, |ui| {
                    for option in SeamKind::ALL {
                        ui.selectable_value(&mut kind, option, option.label()).on_hover_text(option.hint());
                    }
                });
                if kind != seam.kind {
                    seam.set_kind(kind);
                }
                ui.end_row();
                widgets::prop_label(ui, "Margen hacia");
                let pressed_label = |p: Option<usize>| match p {
                    None => "Abierto (cada uno a su pieza)".to_owned(),
                    Some(j) => format!("«{}»", names.get(j).map_or("?", |n| n.as_str())),
                };
                let together = seam.kind.pressed_together();
                egui::ComboBox::from_id_salt("seam_pressed")
                    .selected_text(pressed_label(seam.pressed_side()))
                    .show_ui(ui, |ui| {
                        if !together {
                            ui.selectable_value(&mut seam.pressed_to, None, pressed_label(None));
                        }
                        for j in 0..names.len() {
                            ui.selectable_value(&mut seam.pressed_to, Some(j), pressed_label(Some(j)));
                        }
                    })
                    .response
                    .on_hover_text("Hacia qué pieza se plancha el margen (por el revés)");
                ui.end_row();
                widgets::prop_label(ui, "Pespunte");
                egui::ComboBox::from_id_salt("seam_topstitch").selected_text(seam.topstitch.label()).show_ui(
                    ui,
                    |ui| {
                        for option in Topstitch::ALL {
                            ui.selectable_value(&mut seam.topstitch, option, option.label());
                        }
                    },
                );
                ui.end_row();
                widgets::prop_label(ui, "Puntada");
                ui.add(egui::DragValue::new(&mut seam.stitch_length).range(0.5..=8.0).speed(0.1).suffix(" mm"));
                ui.end_row();
                widgets::prop_label(ui, "Orden");
                ui.horizontal(|ui| {
                    ui.label(format!("{} de {count}", k + 1));
                    if ui.add_enabled(k > 0, egui::Button::new("▲").small()).on_hover_text("Coser antes").clicked() {
                        move_by = -1;
                    }
                    if ui
                        .add_enabled(k + 1 < count, egui::Button::new("▼").small())
                        .on_hover_text("Coser después")
                        .clicked()
                    {
                        move_by = 1;
                    }
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            let colors = [0, 1].map(|j| sides.get(j).map_or(theme::TEXT_WEAK, |s| canvas::piece_color(s.piece)));
            section::show(ui, seam.kind, seam.pressed_side(), seam.topstitch, colors);
            ui.label(RichText::new(seam.kind.hint()).size(11.0).color(theme::TEXT_WEAK));
            ui.add_space(6.0);
            ui.label(RichText::new("LADOS").size(10.5).color(theme::TEXT_WEAK));
            let default_allowance = self.pattern.seam_allowance;
            egui::Grid::new("seam_sides").num_columns(4).striped(true).show(ui, |ui| {
                for (j, side) in sides.iter().enumerate() {
                    let length = self.pattern.edge_length(side.piece, side.edge).unwrap_or(0.0);
                    let Some(piece) = self.pattern.piece_mut(side.piece) else { continue };
                    let edge = piece.edge_index(side.edge);
                    ui.label(format!("{} · arista {}", names[j], edge.map_or(0, |i| i + 1)));
                    ui.label(RichText::new(format!("{:.1} cm", length / 10.0)).color(theme::TEXT_WEAK));
                    match edge {
                        Some(i) => allowance_editor(ui, piece, i, default_allowance),
                        None => {
                            ui.label("");
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("⇄").on_hover_text("Invertir el sentido de este lado").clicked() {
                            flip = Some(j);
                        }
                        if sides.len() > 2 && ui.small_button("✕").on_hover_text("Quitar este lado").clicked() {
                            remove = Some(j);
                        }
                    });
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Todos los lados se cosen a la vez desde el punto gordo, en el sentido de la flecha.")
                    .size(11.0)
                    .color(theme::TEXT_WEAK),
            );
            ui.add_space(6.0);
            delete = ui.button("Borrar costura").clicked();
        });
        if move_by != 0 {
            self.pattern.move_seam(id, move_by);
            return;
        }
        let seam = &mut self.pattern.seams[k];
        if let Some(j) = flip {
            seam.sides[j].reversed = !seam.sides[j].reversed;
        }
        if let Some(j) = remove {
            seam.sides.remove(j);
            if seam.pressed_to.is_some_and(|p| p >= j) {
                seam.pressed_to = seam.pressed_to.and_then(|p| p.checked_sub(1));
            }
        }
        if delete {
            self.pattern.remove_seam(id);
            self.editor.selection = Selection::None;
        }
    }

    /// Lo que falta o está mal, por etapas: la etapa activa primero y
    /// desplegada. Un clic lleva a la etapa y selecciona lo que hay que
    /// arreglar.
    fn checks_section(&self, ui: &mut Ui, action: &mut Option<TreeAction>) {
        let count = self.checks.len();
        widgets::section(ui, "Comprobaciones", Some(count), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_space(2.0);
            if count == 0 {
                ui.label(RichText::new("✔ Todo en orden en todas las etapas").color(theme::OK));
                return;
            }
            let order = std::iter::once(self.stage).chain(Stage::ALL.into_iter().filter(|&s| s != self.stage));
            for stage in order {
                let checks: Vec<_> = self.checks.iter().filter(|c| c.stage == stage).collect();
                if checks.is_empty() {
                    continue;
                }
                let title = format!("{} · {} ({})", stage.number(), stage.label(), checks.len());
                egui::CollapsingHeader::new(RichText::new(title).size(11.5))
                    // Al cambiar de etapa, se despliega solo la nueva.
                    .id_salt(("checks", stage.number(), self.stage.number()))
                    .default_open(stage == self.stage)
                    .show(ui, |ui| {
                        for check in checks {
                            let text = RichText::new(format!("⚠ {}", check.text)).color(theme::WARN);
                            let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                            if response.on_hover_text("Clic para ir a la etapa y seleccionarlo").clicked() {
                                *action = Some(TreeAction::Check(check.stage, check.target));
                            }
                        }
                    });
            }
        });
    }

    /// Pista de la herramienta activa en la etapa, para la barra de estado.
    pub(super) fn tool_hint(&self) -> &'static str {
        match (self.editor.tool, self.stage) {
            (canvas::Tool::Select, Stage::Fabrics) => {
                "Clic en una pieza para ver y cambiar su tela y sus capas · F: encuadrar"
            }
            (canvas::Tool::Select, Stage::Assembly) => {
                "Clic en una costura o una arista para ver su tipo, su margen y su orden · S: nueva costura · F: encuadrar"
            }
            (canvas::Tool::Select, Stage::Placement | Stage::Simulation) => {
                "Clic en una pieza para elegir su lado, su zona y su cara · arrastra para moverla · F: encuadrar"
            }
            (tool, _) => tool.hint(),
        }
    }
}

/// Margen de costura de la arista `i`: el suyo o, si no tiene, el del
/// patrón (en gris), con un botón para volver a él.
fn allowance_editor(ui: &mut Ui, piece: &mut pattern::Piece, i: usize, default: f64) {
    ui.horizontal(|ui| {
        let own = piece.edges()[i].allowance;
        let mut value = own.unwrap_or(default);
        let drag = egui::DragValue::new(&mut value).range(0.0..=50.0).speed(0.5).suffix(" mm");
        let response = if own.is_some() { ui.add(drag) } else { ui.add(drag).on_hover_text("El del patrón") };
        if response.changed() {
            piece.set_edge_allowance(i, Some(value));
        }
        if own.is_some()
            && ui.small_button("↺").on_hover_text(format!("Usar el del patrón ({default:.0} mm)")).clicked()
        {
            piece.set_edge_allowance(i, None);
        }
    });
}

/// Editor de un punto en cm. Devuelve `true` si ha cambiado.
fn point_editor(ui: &mut Ui, p: &mut Point) -> bool {
    let (mut x, mut y) = (p.x / 10.0, p.y / 10.0);
    let changed = ui
        .horizontal(|ui| {
            ui.label(RichText::new("X").color(Color32::from_rgb(0xe0, 0x6c, 0x6c)));
            let a = ui.add(egui::DragValue::new(&mut x).speed(0.1).fixed_decimals(1).suffix(" cm")).changed();
            ui.label(RichText::new("Y").color(Color32::from_rgb(0x7c, 0xc4, 0x6c)));
            let b = ui.add(egui::DragValue::new(&mut y).speed(0.1).fixed_decimals(1).suffix(" cm")).changed();
            a || b
        })
        .inner;
    if changed {
        *p = Point::new(x * 10.0, y * 10.0);
    }
    changed
}
