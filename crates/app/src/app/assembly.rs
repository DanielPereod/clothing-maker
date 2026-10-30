//! Línea de montaje: las etapas en las que se cose la prenda, en orden, con
//! sus costuras. Se ve al montar (para ordenarlas) y al simular (resalta la
//! etapa que se está cosiendo).

use eframe::egui::{self, CornerRadius, Margin, RichText, Stroke, Ui};
use pattern::{SeamId, StepId};

use super::ClothingMakerApp;
use crate::canvas::{self, Selection};
use crate::stages::Stage;
use crate::theme;

/// Lo pedido en la línea de montaje: se aplica después de dibujarla.
enum Action {
    Select(SeamId),
    /// Pasa una costura a una etapa (o a ninguna).
    Assign(SeamId, Option<StepId>),
    Move(StepId, isize),
    Remove(StepId),
    Add,
}

impl ClothingMakerApp {
    pub(super) fn assembly_line(&mut self, ui: &mut Ui) {
        let sewing = if self.stage == Stage::Simulation { self.view3d.sewing_stage() } else { None };
        let selected = match self.editor.selection {
            Selection::Seam(id) => self.pattern.seam(id).map(|s| (id, self.pattern.seam_stage(s), s.name.clone())),
            _ => None,
        };
        // Costuras de cada etapa (0 = sin etapa), en el orden del patrón.
        let count = self.pattern.steps.len();
        let mut stages: Vec<Vec<(usize, SeamId, String)>> = vec![Vec::new(); count + 1];
        for (k, seam) in self.pattern.seams.iter().enumerate() {
            stages[self.pattern.seam_stage(seam)].push((k, seam.id, seam.name.clone()));
        }
        let mut action = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(120.0);
                ui.label(RichText::new("LÍNEA DE MONTAJE").size(10.5).strong().color(theme::TEXT_WEAK));
                ui.label(
                    RichText::new("Cada etapa se cose cuando se han cerrado las anteriores")
                        .size(10.5)
                        .color(theme::TEXT_WEAK),
                );
            });
            egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (stage, seams) in stages.iter().enumerate() {
                        // «Sin etapa» solo si tiene costuras o no hay etapas.
                        if stage == 0 && seams.is_empty() && count > 0 {
                            continue;
                        }
                        let step = stage.checked_sub(1).map(|i| self.pattern.steps[i].id);
                        let lit = sewing == Some(stage)
                            || (sewing.is_none() && selected.as_ref().is_some_and(|s| s.1 == stage));
                        let stroke =
                            if lit { Stroke::new(1.5, theme::ACCENT) } else { Stroke::new(1.0, theme::BORDER) };
                        let fill =
                            if sewing == Some(stage) { theme::ACCENT.gamma_multiply(0.25) } else { theme::PANEL };
                        egui::Frame::NONE
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(Margin::same(6))
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.set_min_width(150.0);
                                    ui.horizontal(|ui| match stage.checked_sub(1) {
                                        None => {
                                            ui.label(RichText::new("Sin etapa").color(theme::TEXT_WEAK))
                                                .on_hover_text("Se cosen las primeras, antes que cualquier etapa");
                                        }
                                        Some(i) => {
                                            ui.label(RichText::new(format!("{}.", i + 1)).strong());
                                            ui.add(
                                                egui::TextEdit::singleline(&mut self.pattern.steps[i].name)
                                                    .desired_width(100.0),
                                            );
                                            let id = self.pattern.steps[i].id;
                                            if ui
                                                .add_enabled(i > 0, egui::Button::new("◀").small())
                                                .on_hover_text("Coser antes")
                                                .clicked()
                                            {
                                                action = Some(Action::Move(id, -1));
                                            }
                                            if ui
                                                .add_enabled(i + 1 < count, egui::Button::new("▶").small())
                                                .on_hover_text("Coser después")
                                                .clicked()
                                            {
                                                action = Some(Action::Move(id, 1));
                                            }
                                            if ui
                                                .small_button("✕")
                                                .on_hover_text("Borrar la etapa (sus costuras se quedan sin etapa)")
                                                .clicked()
                                            {
                                                action = Some(Action::Remove(id));
                                            }
                                        }
                                    });
                                    for (k, id, name) in seams {
                                        let text = RichText::new(format!("● {name}")).color(canvas::seam_color(*k));
                                        let chosen = selected.as_ref().is_some_and(|s| s.0 == *id);
                                        if ui.selectable_label(chosen, text).clicked() {
                                            action = Some(Action::Select(*id));
                                        }
                                    }
                                    if let Some((id, from, name)) = &selected
                                        && *from != stage
                                        && ui
                                            .small_button("+ la costura elegida")
                                            .on_hover_text(format!("Coser «{name}» en esta etapa"))
                                            .clicked()
                                    {
                                        action = Some(Action::Assign(*id, step));
                                    }
                                })
                            });
                    }
                    if ui
                        .button("+ Nueva etapa")
                        .on_hover_text("Añade una etapa al final de la línea de montaje")
                        .clicked()
                    {
                        action = Some(Action::Add);
                    }
                });
            });
        });
        match action {
            Some(Action::Select(id)) => self.editor.selection = Selection::Seam(id),
            Some(Action::Assign(id, step)) => {
                if let Some(seam) = self.pattern.seam_mut(id) {
                    seam.step = step;
                }
            }
            Some(Action::Move(id, delta)) => self.pattern.move_step(id, delta),
            Some(Action::Remove(id)) => {
                self.pattern.remove_step(id);
            }
            Some(Action::Add) => {
                let step = self.pattern.add_step(format!("Etapa {}", count + 1));
                // La costura elegida que aún no tiene etapa va a la nueva.
                if let Some((id, 0, _)) = selected
                    && let Some(seam) = self.pattern.seam_mut(id)
                {
                    seam.step = Some(step);
                }
            }
            None => {}
        }
    }
}
