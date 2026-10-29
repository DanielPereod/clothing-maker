//! Controles propios de la interfaz: botones de la cinta con icono y
//! texto, grupos de herramientas, secciones plegables de los paneles y la
//! barra flotante de navegación de las vistas.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, Popup, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, vec2,
};

use crate::icons::{self, Icon};
use crate::theme;

/// Botón grande de la cinta: icono arriba y nombre debajo.
pub fn ribbon_button(ui: &mut Ui, icon: Icon, label: &str, active: bool, enabled: bool) -> Response {
    let font = FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, theme::TEXT);
    let size = vec2(galley.size().x.max(36.0) + 12.0, 54.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = enabled && response.hovered();
        if active {
            painter.rect(
                rect,
                3.0,
                theme::ACCENT.gamma_multiply(0.28),
                Stroke::new(1.0, theme::ACCENT),
                StrokeKind::Inside,
            );
        } else if hovered {
            painter.rect(
                rect,
                3.0,
                Color32::from_white_alpha(14),
                Stroke::new(1.0, Color32::from_white_alpha(28)),
                StrokeKind::Inside,
            );
        }
        let color = match (enabled, active || hovered) {
            (false, _) => theme::TEXT_WEAK.gamma_multiply(0.6),
            (true, true) => Color32::WHITE,
            (true, false) => theme::TEXT,
        };
        let icon_rect = Rect::from_center_size(rect.center_top() + vec2(0.0, 19.0), vec2(26.0, 26.0));
        icons::paint(painter, icon_rect, icon, if active { Color32::from_rgb(0x7c, 0xcc, 0xf5) } else { color });
        let text_pos = rect.center_bottom() - vec2(galley.size().x / 2.0, galley.size().y + 5.0);
        painter.galley(text_pos, galley, color);
    }
    response
}

/// Qué parte de un grupo de la cinta se está dibujando.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Los botones del grupo.
    Buttons,
    /// El menú que despliega el nombre del grupo.
    Menu,
}

/// Grupo de la cinta: sus botones en fila y, debajo, el nombre del grupo.
/// Con `has_menu`, el nombre despliega más órdenes, como en los programas de
/// CAD. `contents` dibuja los botones y, si se despliega, el menú.
pub fn ribbon_group(ui: &mut Ui, title: &str, has_menu: bool, mut contents: impl FnMut(&mut Ui, Part)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        ui.horizontal(|ui| {
            ui.set_min_height(56.0);
            contents(ui, Part::Buttons);
        });
        let text = RichText::new(if has_menu { format!("{title}  ⏷") } else { title.to_owned() })
            .size(10.0)
            .color(theme::TEXT_WEAK);
        let width = ui.min_rect().width();
        ui.allocate_ui_with_layout(vec2(width, 16.0), Layout::top_down(Align::Center), |ui| {
            if has_menu {
                let response = ui.add(egui::Button::new(text).frame(false).min_size(vec2(width, 16.0)));
                Popup::menu(&response).show(|ui| {
                    ui.set_min_width(200.0);
                    contents(ui, Part::Menu)
                });
            } else {
                ui.add_sized(vec2(width, 16.0), egui::Label::new(text));
            }
        });
    });
    separator_v(ui, 72.0);
}

/// Separador vertical de la cinta.
pub fn separator_v(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment(
        [egui::pos2(x, rect.top() + 6.0), egui::pos2(x, rect.bottom() - 6.0)],
        Stroke::new(1.0, Color32::from_white_alpha(22)),
    );
}

/// Botón pequeño de solo icono, para barras compactas.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str, active: bool) -> Response {
    icon_button_enabled(ui, icon, tooltip, active, true)
}

pub fn icon_button_enabled(ui: &mut Ui, icon: Icon, tooltip: &str, active: bool, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = enabled && response.hovered();
        if active {
            painter.rect_filled(rect, 3.0, theme::ACCENT.gamma_multiply(0.45));
        } else if hovered {
            painter.rect_filled(rect, 3.0, Color32::from_white_alpha(22));
        }
        let color = if !enabled {
            theme::TEXT_WEAK.gamma_multiply(0.5)
        } else if hovered || active {
            Color32::WHITE
        } else {
            theme::TEXT
        };
        icons::paint(painter, rect.shrink(5.0), icon, color);
    }
    response.on_hover_text(tooltip)
}

/// Sección plegable de un panel, con una cabecera oscura de ancho completo.
pub fn section<R>(ui: &mut Ui, title: &str, count: Option<usize>, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let id = ui.make_persistent_id(("section", title));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    if response.clicked() {
        state.toggle(ui);
    }
    let painter = ui.painter();
    let fill = if response.hovered() { Color32::from_rgb(0x2b, 0x2e, 0x31) } else { theme::HEADER };
    painter.rect_filled(rect, 0.0, fill);
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::BORDER));
    let openness = state.openness(ui.ctx());
    let arrow = Rect::from_center_size(rect.left_center() + vec2(8.0, 0.0), vec2(8.0, 8.0));
    let points = if openness > 0.5 {
        vec![arrow.left_top(), arrow.right_top(), arrow.center_bottom()]
    } else {
        vec![arrow.left_top(), arrow.right_center(), arrow.left_bottom()]
    };
    painter.add(egui::Shape::convex_polygon(points, theme::TEXT_WEAK, Stroke::NONE));
    painter.text(
        rect.left_center() + vec2(20.0, 0.0),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        FontId::proportional(11.0),
        theme::TEXT,
    );
    if let Some(n) = count {
        painter.text(
            rect.right_center() - vec2(6.0, 0.0),
            Align2::RIGHT_CENTER,
            n.to_string(),
            FontId::proportional(11.0),
            theme::TEXT_WEAK,
        );
    }
    state.store(ui.ctx());
    let inner = state.show_body_unindented(ui, |ui| {
        ui.add_space(4.0);
        let r = Frame::NONE.inner_margin(Margin::symmetric(6, 0)).show(ui, add).inner;
        ui.add_space(6.0);
        r
    });
    inner.map(|r| r.inner)
}

/// Fila del árbol del navegador: icono de color, nombre y, si se pasa, un
/// texto secundario alineado a la derecha.
pub fn tree_row(
    ui: &mut Ui,
    swatch: Color32,
    label: &str,
    detail: Option<&str>,
    selected: bool,
    depth: usize,
) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 2.0, theme::ACCENT.gamma_multiply(0.35));
            painter.rect_filled(Rect::from_min_size(rect.min, vec2(2.0, rect.height())), 0.0, theme::ACCENT);
        } else if response.hovered() {
            painter.rect_filled(rect, 2.0, Color32::from_white_alpha(12));
        }
        let x = rect.left() + 8.0 + depth as f32 * 14.0;
        let sw = Rect::from_center_size(egui::pos2(x + 5.0, rect.center().y), vec2(10.0, 10.0));
        painter.rect(sw, 2.0, swatch.gamma_multiply(0.85), Stroke::new(1.0, swatch), StrokeKind::Inside);
        let right = match detail {
            Some(detail) => {
                let r = painter.text(
                    rect.right_center() - vec2(6.0, 0.0),
                    Align2::RIGHT_CENTER,
                    detail,
                    FontId::proportional(11.0),
                    theme::TEXT_WEAK,
                );
                r.left() - 6.0
            }
            None => rect.right(),
        };
        let text_rect = Rect::from_min_max(egui::pos2(x + 16.0, rect.top()), egui::pos2(right, rect.bottom()));
        painter.with_clip_rect(text_rect).text(
            text_rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            if selected { Color32::WHITE } else { theme::TEXT },
        );
    }
    response
}

/// Rejilla de propiedades «nombre — valor» del inspector.
pub fn properties<R>(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(Id::unique(("props", id)))
        .num_columns(2)
        .spacing(vec2(10.0, 6.0))
        .min_col_width(84.0)
        .striped(false)
        .show(ui, add)
        .inner
}

/// Nombre de una propiedad, en gris.
pub fn prop_label(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).color(theme::TEXT_WEAK))
}

/// Barra flotante centrada abajo de una vista, como la de navegación de los
/// programas de CAD.
pub fn nav_bar(ui: &mut Ui, viewport: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    let width = ui.ctx().memory(|m| m.data.get_temp::<f32>(Id::unique(("nav_bar", id)))).unwrap_or(200.0);
    let rect = Rect::from_center_size(viewport.center_bottom() - vec2(0.0, 24.0), vec2(width, 34.0));
    let inner = ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)), |ui| {
        Frame::NONE
            .fill(Color32::from_rgba_unmultiplied(0x26, 0x28, 0x2b, 235))
            .stroke(Stroke::new(1.0, Color32::from_white_alpha(20)))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(4, 3))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                add(ui)
            })
            .response
            .rect
    });
    let measured = inner.inner.width();
    if (measured - width).abs() > 0.5 {
        ui.ctx().memory_mut(|m| m.data.insert_temp(Id::unique(("nav_bar", id)), measured));
        ui.ctx().request_repaint();
    }
}

/// Etiqueta de la vista, arriba a la izquierda.
pub fn viewport_label(ui: &Ui, viewport: Rect, text: &str) {
    let painter = ui.painter_at(viewport);
    let galley = painter.layout_no_wrap(text.to_uppercase(), FontId::proportional(10.5), theme::TEXT);
    let rect = Rect::from_min_size(viewport.min + vec2(8.0, 8.0), galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(0x1c, 0x1d, 0x1f, 200));
    painter.galley(rect.min + vec2(6.0, 3.0), galley, theme::TEXT);
}

/// Título de un panel lateral.
pub fn panel_title(ui: &mut Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, theme::RIBBON);
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::BORDER));
    painter.text(
        rect.left_center() + vec2(10.0, 0.0),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        FontId::proportional(11.5),
        Color32::WHITE,
    );
}

/// Fila del navegador con icono en vez de muestra de color (la raíz del árbol).
pub fn tree_icon_row(ui: &mut Ui, icon: Icon, label: &str, detail: Option<&str>) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let painter = ui.painter();
    icons::paint(
        painter,
        Rect::from_center_size(rect.left_center() + vec2(10.0, 0.0), vec2(16.0, 16.0)),
        icon,
        theme::TEXT,
    );
    painter.text(
        rect.left_center() + vec2(24.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        Color32::WHITE,
    );
    if let Some(detail) = detail {
        painter.text(
            rect.right_center() - vec2(6.0, 0.0),
            Align2::RIGHT_CENTER,
            detail,
            FontId::proportional(11.0),
            theme::TEXT_WEAK,
        );
    }
    response
}

/// Pestaña de una etapa de trabajo en la cinta: su número, su nombre y,
/// si tiene algo pendiente, cuántas cosas.
pub fn stage_tab(ui: &mut Ui, number: usize, label: &str, pending: usize, active: bool) -> Response {
    let font = FontId::proportional(11.5);
    let hovered_color = |hovered: bool| if active || hovered { Color32::WHITE } else { theme::TEXT_WEAK };
    let galley = ui.painter().layout_no_wrap(label.to_uppercase(), font.clone(), Color32::PLACEHOLDER);
    let badge = (pending > 0)
        .then(|| ui.painter().layout_no_wrap(pending.to_string(), FontId::proportional(10.0), Color32::PLACEHOLDER));
    let badge_width = badge.as_ref().map_or(0.0, |b| b.size().x.max(8.0) + 14.0);
    let size = vec2(galley.size().x + 50.0 + badge_width, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, theme::RIBBON);
        painter.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, theme::ACCENT));
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, Color32::from_white_alpha(10));
    }
    let color = hovered_color(response.hovered());
    let circle = rect.left_center() + vec2(20.0, 0.0);
    if active {
        painter.circle_filled(circle, 8.0, theme::ACCENT);
    } else {
        painter.circle_stroke(circle, 7.5, Stroke::new(1.0, color));
    }
    painter.text(circle, Align2::CENTER_CENTER, number.to_string(), FontId::proportional(10.5), color);
    let text_pos = rect.left_center() + vec2(34.0, -galley.size().y / 2.0);
    painter.galley(text_pos, galley, color);
    if let Some(badge) = badge {
        let center = rect.right_center() - vec2(10.0 + (badge.size().x.max(8.0) + 6.0) / 2.0, 0.0);
        let pill = Rect::from_center_size(center, vec2(badge.size().x.max(8.0) + 6.0, 15.0));
        painter.rect_filled(pill, 7.5, theme::WARN);
        painter.galley(center - badge.size() / 2.0, badge, Color32::BLACK);
    }
    response
}

/// Flecha entre dos pestañas de etapa: se hacen en ese orden.
pub fn stage_arrow(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 26.0), Sense::hover());
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "›", FontId::proportional(14.0), theme::TEXT_WEAK);
}
