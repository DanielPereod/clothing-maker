//! Telas en el inspector: la tela y las capas de la pieza seleccionada y la
//! biblioteca de telas del patrón, con sus ajustes.

use eframe::egui::{self, RichText, Ui};
use pattern::{PieceId, TextileId, textile};

use super::ClothingMakerApp;
use crate::theme;
use crate::view3d;
use crate::widgets;

impl ClothingMakerApp {
    /// Tela de fuera y capas de una pieza.
    pub(super) fn piece_textiles(&mut self, ui: &mut Ui, id: PieceId) {
        let Some(piece) = self.pattern.piece(id) else { return };
        let mut layers: Vec<TextileId> = piece.textiles().collect();
        let before = layers.clone();
        let mut remove = None;
        let mut edit = None;
        let textiles: Vec<(TextileId, String)> = self.pattern.textiles.iter().map(|t| (t.id, t.name.clone())).collect();
        let name_of = |id: TextileId| textiles.iter().find(|t| t.0 == id).map_or("?", |t| t.1.as_str());
        widgets::section(ui, "Tela y capas", Some(layers.len()), |ui| {
            widgets::properties(ui, "piece_textiles", |ui| {
                for (k, layer) in layers.iter_mut().enumerate() {
                    let label = if k == 0 { "Exterior".to_owned() } else { format!("Capa {}", k + 1) };
                    let hint = if k == 0 {
                        "La tela de la pieza, la que se ve por fuera"
                    } else {
                        "Otra tela unida a toda la pieza por dentro (forro, entretela...): suma su firmeza, \
                         su peso y su grosor"
                    };
                    widgets::prop_label(ui, &label).on_hover_text(hint);
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("layer", k)).width(110.0).selected_text(name_of(*layer)).show_ui(
                            ui,
                            |ui| {
                                for (t, name) in &textiles {
                                    ui.selectable_value(layer, *t, name);
                                }
                            },
                        );
                        if ui.small_button("✎").on_hover_text("Ajustar esta tela en la biblioteca").clicked() {
                            edit = Some(*layer);
                        }
                        if k > 0 && ui.small_button("✕").on_hover_text("Quitar esta capa").clicked() {
                            remove = Some(k);
                        }
                    });
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            if ui.button("Añadir capa").on_hover_text("Añade otra tela por dentro, como un forro").clicked() {
                let lining = textiles.iter().find(|t| t.1 == "Forro").map_or(TextileId::COTTON, |t| t.0);
                layers.push(lining);
            }
            ui.add_space(4.0);
            let fabric = view3d::piece_fabric(&self.pattern, self.pattern.piece(id).expect("existe"));
            let summary = format!(
                "Firmeza total: ×{} la del algodón · {:.0} g/m² · {:.1} mm",
                times(fabric.relative_firmness()),
                fabric.density * 1e3,
                fabric.thickness * 1e3
            );
            ui.label(RichText::new(summary).size(11.0).color(theme::TEXT_WEAK));
        });
        if let Some(k) = remove {
            layers.remove(k);
        }
        if layers != before
            && let Some(piece) = self.pattern.piece_mut(id)
        {
            piece.fabric = layers[0];
            piece.layers = layers[1..].to_vec();
        }
        if edit.is_some() {
            self.open_textile = edit;
        }
    }

    /// La biblioteca de telas del patrón.
    pub(super) fn textile_library(&mut self, ui: &mut Ui) {
        let open = self.open_textile.take();
        let mut duplicate = None;
        let mut delete = None;
        let mut use_everywhere = None;
        let ids: Vec<TextileId> = self.pattern.textiles.iter().map(|t| t.id).collect();
        widgets::section(ui, "Biblioteca de telas", Some(ids.len()), |ui| {
            for id in ids {
                let uses = self.pattern.textile_uses(id);
                let Some(t) = self.pattern.textile_mut(id) else { continue };
                let title = match uses {
                    0 => t.name.clone(),
                    1 => format!("{} · 1 pieza", t.name),
                    n => format!("{} · {n} piezas", t.name),
                };
                let response = egui::CollapsingHeader::new(title)
                    .id_salt(("textile", id.0))
                    .open(open.and_then(|o| (o == id).then_some(true)))
                    .show(ui, |ui| {
                        widgets::properties(ui, &format!("textile{}", id.0), |ui| {
                            widgets::prop_label(ui, "Nombre");
                            ui.text_edit_singleline(&mut t.name);
                            ui.end_row();
                            widgets::prop_label(ui, "Firmeza").on_hover_text(
                                "Cuánto se resiste a doblarse. Con poca cae como un velo; con mucha se sostiene \
                                 sola, como el cuero de un bolso. Cada 25 puntos es diez veces más firme.",
                            );
                            ui.add(egui::Slider::new(&mut t.firmness, textile::FIRMNESS_RANGE).max_decimals(0));
                            ui.end_row();
                            widgets::prop_label(ui, "Peso")
                                .on_hover_text("Gramaje de la tela. Cuanto más pesa, más cae y más se descuelga.");
                            ui.add(
                                egui::Slider::new(&mut t.weight, textile::WEIGHT_RANGE)
                                    .logarithmic(true)
                                    .suffix(" g/m²")
                                    .max_decimals(0),
                            );
                            ui.end_row();
                            widgets::prop_label(ui, "Elasticidad")
                                .on_hover_text("Cuánto cede al estirarla: poca en el vaquero, mucha en el punto.");
                            ui.add(egui::Slider::new(&mut t.stretch, textile::STRETCH_RANGE).max_decimals(0));
                            ui.end_row();
                            widgets::prop_label(ui, "Rozamiento")
                                .on_hover_text("Lo que se agarra al cuerpo o a la mesa en vez de resbalar.");
                            ui.add(egui::Slider::new(&mut t.friction, textile::FRICTION_RANGE).max_decimals(2));
                            ui.end_row();
                            widgets::prop_label(ui, "Grosor");
                            ui.add(
                                egui::Slider::new(&mut t.thickness, textile::THICKNESS_RANGE)
                                    .suffix(" mm")
                                    .max_decimals(1),
                            );
                            ui.end_row();
                        });
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button("Usar en todas")
                                .on_hover_text("Pone esta tela por fuera en todas las piezas")
                                .clicked()
                            {
                                use_everywhere = Some(id);
                            }
                            if ui.button("Duplicar").on_hover_text("Una copia para ajustarla aparte").clicked() {
                                duplicate = Some(id);
                            }
                            if let Some(generic) = t.generic_version()
                                && ui.add_enabled(generic != *t, egui::Button::new("Valores de fábrica")).clicked()
                            {
                                *t = generic;
                            }
                            let hint = if uses > 0 { "Hay piezas que la usan" } else { "Quitarla de la biblioteca" };
                            if ui.add_enabled(uses == 0, egui::Button::new("Borrar")).on_hover_text(hint).clicked() {
                                delete = Some(id);
                            }
                        });
                    });
                if open == Some(id) {
                    response.header_response.scroll_to_me(Some(egui::Align::TOP));
                }
            }
        });
        if let Some(id) = use_everywhere {
            for piece in &mut self.pattern.pieces {
                piece.fabric = id;
            }
        }
        if let Some(id) = duplicate {
            self.open_textile = self.pattern.duplicate_textile(id);
        }
        if let Some(id) = delete {
            self.pattern.remove_textile(id);
        }
    }
}

/// Un factor con coma decimal y las cifras justas («0,15», «2,5», «120»).
fn times(ratio: f32) -> String {
    let decimals = if ratio < 1.0 {
        2
    } else if ratio < 10.0 {
        1
    } else {
        0
    };
    format!("{ratio:.decimals$}").replace('.', ",")
}
