//! Tema oscuro sobrio, al estilo de las aplicaciones de CAD: grises
//! neutros, un único color de acento y controles compactos.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme, Visuals};

/// Color de acento: selección, herramienta activa, foco.
pub const ACCENT: Color32 = Color32::from_rgb(0x1f, 0x96, 0xd8);
/// Barra de la aplicación (arriba del todo).
pub const APP_BAR: Color32 = Color32::from_rgb(0x1c, 0x1d, 0x1f);
/// Cinta de herramientas.
pub const RIBBON: Color32 = Color32::from_rgb(0x34, 0x36, 0x39);
/// Paneles laterales y barra de estado.
pub const PANEL: Color32 = Color32::from_rgb(0x2a, 0x2c, 0x2f);
/// Cabeceras de sección de los paneles.
pub const HEADER: Color32 = Color32::from_rgb(0x23, 0x25, 0x27);
/// Fondo de las vistas 2D y 3D.
pub const VIEWPORT: Color32 = Color32::from_rgb(0x3a, 0x3d, 0x41);
/// Separadores entre zonas.
pub const BORDER: Color32 = Color32::from_rgb(0x18, 0x19, 0x1b);
pub const TEXT: Color32 = Color32::from_rgb(0xd6, 0xd8, 0xdb);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x8d, 0x92, 0x99);
pub const WARN: Color32 = Color32::from_rgb(0xe8, 0xa3, 0x3d);
pub const ERROR: Color32 = Color32::from_rgb(0xe5, 0x5c, 0x5c);
pub const OK: Color32 = Color32::from_rgb(0x5c, 0xb8, 0x7a);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.visuals = visuals();
        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(6.0, 4.0);
        spacing.button_padding = egui::vec2(8.0, 3.0);
        spacing.interact_size.y = 22.0;
        spacing.menu_margin = Margin::same(4);
        spacing.window_margin = Margin::same(10);
        spacing.indent = 14.0;
        spacing.combo_width = 120.0;
        style.text_styles = [
            (TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(10.5, FontFamily::Proportional)),
        ]
        .into();
    });
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let radius = CornerRadius::same(3);
    v.panel_fill = PANEL;
    v.window_fill = Color32::from_rgb(0x30, 0x32, 0x35);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(4);
    v.extreme_bg_color = VIEWPORT;
    v.text_edit_bg_color = Some(Color32::from_rgb(0x1f, 0x21, 0x23));
    v.faint_bg_color = Color32::from_rgb(0x2f, 0x31, 0x34);
    v.code_bg_color = HEADER;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERROR;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x6c, 0xc4, 0xf2));
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.striped = true;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3c, 0x3f, 0x43));
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = Color32::from_rgb(0x3b, 0x3e, 0x42);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x3b, 0x3e, 0x42);
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x46, 0x49, 0x4e));
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = Color32::from_rgb(0x48, 0x4c, 0x51);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x48, 0x4c, 0x51);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x5a, 0x5f, 0x66));
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = ACCENT.gamma_multiply(0.8);
    w.active.weak_bg_fill = ACCENT.gamma_multiply(0.8);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = Color32::from_rgb(0x44, 0x48, 0x4d);
    w.open.weak_bg_fill = Color32::from_rgb(0x44, 0x48, 0x4d);
    w.open.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x5a, 0x5f, 0x66));
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = radius;
    v
}
