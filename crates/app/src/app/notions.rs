//! Fornituras en la etapa Montaje: cremalleras, cinchas, bieses, vivos y
//! elásticos, generados como piezas y costuras normales.

use eframe::egui::{self, RichText, Ui};
use pattern::{Notion, NotionParts, Point, SeamKind, SeamSide};

use super::ClothingMakerApp;
use crate::canvas::Selection;
use crate::icons::Icon;
use crate::theme;
use crate::widgets;

/// Largo (mm) de una cremallera o una cincha sueltas si no hay un borde
/// elegido del que tomarlo.
const LOOSE_LENGTH: [f64; 2] = [200.0, 600.0];

/// La fornitura que se está a punto de crear, con sus medidas.
#[derive(Clone, Copy, Debug)]
pub(super) struct NotionDialog {
    notion: Notion,
    /// Dónde va: en una costura, en un borde o suelta.
    target: Target,
    /// Largo (mm), si va suelta.
    length: f64,
    width: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Seam(pattern::SeamId),
    Edge(SeamSide),
    Loose,
}

impl ClothingMakerApp {
    /// Grupo de la cinta de Montaje con las fornituras.
    pub(super) fn notions_ribbon(&mut self, ui: &mut Ui) {
        widgets::ribbon_group(ui, "FORNITURAS", false, |ui, _| {
            for notion in Notion::ALL {
                let target = self.notion_target(notion);
                let icon = match notion {
                    Notion::Zipper => Icon::Zipper,
                    Notion::Webbing => Icon::Webbing,
                    Notion::Binding => Icon::Binding,
                    Notion::Piping => Icon::Piping,
                    Notion::Elastic => Icon::Elastic,
                };
                let missing = match notion {
                    Notion::Binding | Notion::Elastic => "\n\nElige antes el borde (una arista) en el que va.",
                    Notion::Piping => "\n\nElige antes la costura en la que va.",
                    _ => "",
                };
                let hint = format!("{}{}", notion.hint(), if target.is_some() { "" } else { missing });
                if widgets::ribbon_button(ui, icon, notion.label(), false, target.is_some())
                    .on_hover_text(hint)
                    .clicked()
                    && let Some(target) = target
                {
                    let length = match (target, notion) {
                        (Target::Loose, Notion::Webbing) => LOOSE_LENGTH[1],
                        _ => self.selected_edge_length().unwrap_or(LOOSE_LENGTH[0]),
                    };
                    self.notion_dialog = Some(NotionDialog { notion, target, length, width: notion.default_width() });
                }
            }
        });
    }

    /// Dónde iría una fornitura con lo que hay elegido, o `None` si le
    /// falta algo.
    fn notion_target(&self, notion: Notion) -> Option<Target> {
        let seam = match self.editor.selection {
            Selection::Seam(id) => self.pattern.seam(id),
            _ => None,
        };
        let edge = match self.editor.selection {
            Selection::Edge(piece, i) => {
                self.pattern.piece(piece).map(|p| SeamSide::new(piece, p.edges()[i].id, false))
            }
            _ => None,
        };
        match notion {
            Notion::Zipper => Some(match seam {
                Some(s) if s.sides.len() == 2 && s.kind != SeamKind::Zipper => Target::Seam(s.id),
                _ => Target::Loose,
            }),
            Notion::Webbing => Some(Target::Loose),
            Notion::Binding | Notion::Elastic => edge.map(Target::Edge),
            Notion::Piping => seam.map(|s| Target::Seam(s.id)),
        }
    }

    fn selected_edge_length(&self) -> Option<f64> {
        let Selection::Edge(piece, i) = self.editor.selection else { return None };
        let edge = self.pattern.piece(piece)?.edges().get(i)?.id;
        self.pattern.edge_length(piece, edge)
    }

    /// La ventana con las medidas de la fornitura que se va a crear.
    pub(super) fn notion_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.notion_dialog else { return };
        let mut create = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("notion_dialog")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading(format!("{}: {}", dialog.notion.label(), self.target_label(dialog.target)));
            ui.add_space(4.0);
            ui.label(RichText::new(dialog.notion.hint()).size(11.0).color(theme::TEXT_WEAK));
            ui.add_space(8.0);
            widgets::properties(ui, "notion", |ui| {
                if dialog.target == Target::Loose {
                    widgets::prop_label(ui, "Largo");
                    let mut cm = dialog.length / 10.0;
                    ui.add(egui::DragValue::new(&mut cm).range(1.0..=500.0).speed(0.5).suffix(" cm"));
                    dialog.length = cm * 10.0;
                    ui.end_row();
                }
                let label = match dialog.notion {
                    Notion::Zipper => "Ancho de cada cinta",
                    Notion::Binding | Notion::Piping => "Ancho a la vista",
                    _ => "Ancho",
                };
                widgets::prop_label(ui, label);
                ui.add(egui::DragValue::new(&mut dialog.width).range(2.0..=100.0).speed(0.5).suffix(" mm"));
                ui.end_row();
            });
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.button("Cancelar").clicked();
                let button = egui::Button::new(RichText::new("Crear").color(egui::Color32::WHITE)).fill(theme::ACCENT);
                create = ui.add(button).clicked();
            });
        });
        self.notion_dialog = if create || cancel { None } else { Some(dialog) };
        if create {
            self.create_notion(dialog);
        }
    }

    fn target_label(&self, target: Target) -> String {
        match target {
            Target::Seam(id) => format!("en «{}»", self.pattern.seam(id).map_or("?", |s| s.name.as_str())),
            Target::Edge(side) => {
                format!("en un borde de «{}»", self.pattern.piece(side.piece).map_or("?", |p| p.name.as_str()))
            }
            Target::Loose => "suelta".to_owned(),
        }
    }

    fn create_notion(&mut self, d: NotionDialog) {
        let origin = self.free_spot();
        let parts = match (d.notion, d.target) {
            (Notion::Zipper, Target::Seam(seam)) => self.pattern.zip_seam(seam, d.width),
            (Notion::Zipper, _) => Some(self.pattern.add_zipper(d.length, d.width, origin)),
            (Notion::Webbing, _) => {
                let piece = self.pattern.add_strap(Notion::Webbing, d.length, d.width, origin);
                Some(NotionParts { pieces: vec![piece], seams: Vec::new() })
            }
            (Notion::Binding, Target::Edge(side)) => self.pattern.bind_edge(side, d.width),
            (Notion::Elastic, Target::Edge(side)) => self.pattern.add_elastic(side, d.width),
            (Notion::Piping, Target::Seam(seam)) => self.pattern.pipe_seam(seam, d.width),
            _ => None,
        };
        match parts {
            Some(parts) => {
                let what = d.notion.label();
                let pieces = parts.pieces.len();
                self.status =
                    Some((format!("{what}: {pieces} pieza(s) y {} costura(s) nuevas", parts.seams.len()), false));
                self.editor.selection = match (parts.seams.last(), parts.pieces.first()) {
                    (Some(&seam), _) => Selection::Seam(seam),
                    (None, Some(&piece)) => Selection::Piece(piece),
                    _ => Selection::None,
                };
            }
            None => {
                self.status = Some((format!("No se ha podido crear: {}", d.notion.label().to_lowercase()), true));
            }
        }
    }

    /// Un sitio libre en el patrón para una pieza nueva: a la derecha de
    /// todo lo que hay.
    fn free_spot(&self) -> Point {
        let bounds = self.pattern.pieces.iter().map(|p| p.bounding_box()).reduce(|a, b| a.union(b));
        bounds.map_or(Point::ZERO, |b| Point::new(b.x1 + 50.0, b.y0))
    }
}
