//! Vista 3D: las piezas del patrón como tela simulada, cosidas entre sí y
//! cayendo sobre el suelo y, si se quiere, sobre un obstáculo (esfera, mesa
//! o el maniquí, para las prendas).

mod render;
mod scene;
mod seams;
mod viewcube;

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, PointerButton, Sense, Ui};
use eframe::egui_wgpu;
use glam::{Mat4, Vec3};
use pattern::{Pattern, PieceId};

use render::{Renderer, Vertex};
pub use scene::piece_fabric;
use scene::{Obstacle, Placement, Runner, ScenePiece, SceneSettings, SeamDraw};

use crate::icons::Icon;
use crate::stages::SceneReport;
use crate::{gpu, theme, widgets};

/// Tiempo mínimo entre dos actualizaciones en vivo: mallar y montar la
/// escena cuesta unos milisegundos, y al arrastrar el patrón cambia en cada
/// fotograma.
const LIVE_INTERVAL: Duration = Duration::from_millis(100);

const OBSTACLE_COLOR: [f32; 3] = [0.42, 0.44, 0.48];
const MANNEQUIN_COLOR: [f32; 3] = [0.62, 0.58, 0.54];

/// Cámara que orbita alrededor de un punto.
#[derive(Clone, Copy, Debug)]
struct Camera {
    target: Vec3,
    /// Giro alrededor del eje vertical (rad).
    yaw: f32,
    /// Inclinación sobre el horizonte (rad).
    pitch: f32,
    distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { target: Vec3::new(0.0, 0.45, 0.0), yaw: 0.5, pitch: 0.4, distance: 2.8 }
    }
}

impl Camera {
    /// Mira a la tela y al suelo de debajo, a una distancia que lo abarque.
    fn framing(points: &[Vec3]) -> Self {
        let (min, max) = points
            .iter()
            .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(a, b), p| (a.min(*p), b.max(*p)));
        if !min.is_finite() {
            return Self::default();
        }
        let (min, max) = (min.with_y(0.0), max.with_y(max.y.max(0.5)));
        let size = (max - min).length();
        Self { target: (min + max) / 2.0, distance: (size * 1.6).max(1.5), ..Self::default() }
    }

    fn eye(&self) -> Vec3 {
        let dir = Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), self.yaw.cos() * self.pitch.cos());
        self.target + dir * self.distance
    }

    fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y);
        // wgpu usa profundidad de 0 a 1 con Y hacia arriba, como DirectX. La
        // profundidad va invertida (1 cerca, 0 en el infinito) para tener
        // precisión de sobra: la tela en el suelo está a solo 3 mm de él.
        let proj =
            glam::camera::rh::proj::directx::perspective_infinite_reverse(40f32.to_radians(), aspect.max(1e-3), 0.01);
        proj * view
    }

    fn orbit(&mut self, delta: egui::Vec2) {
        self.yaw -= delta.x * 0.008;
        self.pitch = (self.pitch + delta.y * 0.008).clamp(-0.1, 1.5);
    }

    fn pan(&mut self, delta: egui::Vec2, viewport_height: f32) {
        let eye = self.eye();
        let forward = (self.target - eye).normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        // Que el punto bajo el ratón siga al ratón.
        let scale = 2.0 * self.distance * (20f32.to_radians()).tan() / viewport_height.max(1.0);
        self.target += (-right * delta.x + up * delta.y) * scale;
        self.target.y = self.target.y.max(0.0);
    }

    fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (-scroll * 0.002).exp()).clamp(0.2, 20.0);
    }
}

/// Lo que hay cargado ahora mismo en la simulación.
struct Loaded {
    pattern: Pattern,
    settings: SceneSettings,
    triangles: Vec<[u32; 3]>,
    indices: Vec<u32>,
    colors: Vec<[f32; 3]>,
    back_colors: Vec<[f32; 3]>,
    seam_draws: Vec<SeamDraw>,
    failed: Vec<PieceId>,
    crossed: Vec<pattern::SeamId>,
    seams: usize,
    pieces: Vec<ScenePiece>,
    turn_stages: Vec<usize>,
    fasteners: Vec<scene::FastenerDraw>,
}

pub struct View3D {
    renderer: Option<Renderer>,
    /// La tarjeta con la que se dibuja.
    adapter: Option<egui_wgpu::wgpu::AdapterInfo>,
    runner: Runner,
    pub settings: SceneSettings,
    camera: Camera,
    /// Encuadre inicial de la escena cargada.
    home: Camera,
    loaded: Option<Loaded>,
    positions: Vec<Vec3>,
    version: u64,
    obstacle: Option<Obstacle>,
    /// Si el patrón era una prenda para el maniquí la última vez, para
    /// poner o quitar el maniquí cuando cambia.
    on_avatar: Option<bool>,
    /// Último obstáculo elegido sin maniquí, para volver a él.
    object_obstacle: Obstacle,
    /// Reajustar la prenda en cuanto cambia el patrón, sin reiniciar.
    live: bool,
    /// Cuándo se hizo la última actualización en vivo.
    last_live: Instant,
    /// Vuelo de la cámara hacia una vista, en curso.
    fly: Option<viewcube::Fly>,
    /// Último patrón que no se ha podido pasar en vivo (alguna pieza no se
    /// puede mallar), para no reintentarlo en cada fotograma.
    rejected: Option<Pattern>,
    /// Resaltar las costuras con un cordón de su color.
    pub highlight_seams: bool,
    /// Si las costuras dibujadas ahora llevan el cordón.
    drawn_highlight: bool,
}

impl View3D {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // El campo de distancia del maniquí tarda un poco: se prepara ya,
        // por si hace falta.
        std::thread::spawn(avatar::mannequin);
        Self {
            renderer: cc.wgpu_render_state.as_ref().map(Renderer::new),
            adapter: cc.wgpu_render_state.as_ref().map(|s| s.adapter.get_info()),
            runner: Runner::new(),
            settings: SceneSettings::default(),
            camera: Camera::default(),
            home: Camera::default(),
            loaded: None,
            positions: Vec::new(),
            version: u64::MAX,
            obstacle: None,
            on_avatar: None,
            object_obstacle: SceneSettings::default().obstacle,
            live: true,
            last_live: Instant::now(),
            fly: None,
            rejected: None,
            highlight_seams: false,
            drawn_highlight: false,
        }
    }

    /// Vuelve a montar la escena con el patrón actual, manteniendo si estaba
    /// en marcha o en pausa.
    fn reload(&mut self, pattern: &Pattern, color_of: &impl Fn(PieceId) -> Color32) {
        self.load(pattern, color_of, false);
    }

    /// Pasa la prenda simulada al patrón o los ajustes actuales sin
    /// desmontarla: cada pieza conserva su forma 3D y la simulación sigue
    /// desde ahí. Si los ajustes nuevos obligan a recolocar las piezas, o
    /// no hay nada cargado, vuelve a empezar.
    fn update_live(&mut self, pattern: &Pattern, color_of: &impl Fn(PieceId) -> Color32) {
        let keeps = self.loaded.as_ref().is_some_and(|l| l.settings.keeps_shape(&self.settings));
        self.load(pattern, color_of, keeps);
        self.last_live = Instant::now();
    }

    fn load(&mut self, pattern: &Pattern, color_of: &impl Fn(PieceId) -> Color32, live: bool) {
        self.rejected = None;
        let mut scene = scene::build(pattern, &self.settings, color_of);
        if live && let Some(loaded) = &self.loaded {
            // Mientras se edita, una pieza puede quedar sin poder mallarse
            // (un contorno que se cruza): se espera a que vuelva a estar bien
            // en vez de quitarla de la prenda.
            if scene.failed.iter().any(|id| !loaded.failed.contains(id)) {
                self.rejected = Some(pattern.clone());
                return;
            }
            self.version = self.runner.read(self.version, &mut self.positions);
            scene::carry_over(&loaded.pieces, &self.positions, &mut scene);
        }
        let indices = scene.cloth.triangles.iter().flatten().copied().collect();
        // Al cambiar de colocación u obstáculo, la cámara vuelve a encuadrar.
        let reframe = !live
            && self.loaded.as_ref().is_none_or(|l| {
                (l.settings.placement, l.settings.obstacle) != (self.settings.placement, self.settings.obstacle)
            });
        let mut framed = scene.cloth.positions.clone();
        if let Some(avatar) = self.settings.obstacle.avatar() {
            // Que se vea el maniquí entero, no solo la prenda.
            framed.extend([Vec3::ZERO, Vec3::Y * avatar.height()]);
        }
        self.home = Camera::framing(&framed);
        if reframe {
            self.camera = self.home;
        }
        self.loaded = Some(Loaded {
            pattern: pattern.clone(),
            settings: self.settings,
            triangles: scene.cloth.triangles.clone(),
            indices,
            colors: scene.colors,
            back_colors: scene.back_colors,
            seam_draws: scene.seam_draws,
            failed: scene.failed,
            crossed: scene.crossed,
            seams: scene.seams,
            pieces: scene.pieces,
            turn_stages: scene.turn_stages,
            fasteners: scene.fasteners,
        });
        let params = simulator::SimParams { pressure: pattern.pressure as f32, ..self.settings.params() };
        self.runner.load(scene.cloth, scene.colliders, params, live);
        self.version = u64::MAX;
    }

    /// Mantiene la simulación al día con el patrón: la primera carga, el
    /// maniquí según el tipo de prenda y los cambios en vivo. Se llama en
    /// cada fotograma mientras la vista 3D está a la vista.
    pub fn sync(&mut self, ctx: &egui::Context, pattern: &Pattern, color_of: impl Fn(PieceId) -> Color32) {
        // Las prendas se ven sobre el maniquí y los objetos sin él.
        if self.on_avatar != Some(pattern.on_avatar) {
            self.on_avatar = Some(pattern.on_avatar);
            let wanted = if pattern.on_avatar { Obstacle::Mannequin } else { self.object_obstacle };
            if self.settings.obstacle != wanted {
                self.settings.obstacle = wanted;
                if self.loaded.is_some() {
                    self.reload(pattern, &color_of);
                }
            }
        }
        if self.loaded.is_none() {
            // La primera vez arranca sola, para que se vea coser sin buscar el botón.
            self.reload(pattern, &color_of);
            self.runner.set_running(true);
        }
        let rejected = self.rejected.as_ref() == Some(pattern);
        if self.live && self.outdated(pattern) && !rejected {
            let wait = LIVE_INTERVAL.saturating_sub(self.last_live.elapsed());
            if wait.is_zero() {
                self.update_live(pattern, &color_of);
                // Las comprobaciones miran la escena nueva en el siguiente
                // fotograma, aunque la simulación esté en pausa.
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(wait);
            }
        }
        if self.runner.is_running() {
            ctx.request_repaint();
        }
    }

    /// Si la escena cargada es de una versión anterior del patrón.
    fn outdated(&self, pattern: &Pattern) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.pattern != *pattern)
    }

    /// Arranca o para la simulación.
    pub fn toggle_running(&mut self, pattern: &Pattern, color_of: impl Fn(PieceId) -> Color32) {
        let running = self.runner.is_running();
        if !running && self.outdated(pattern) {
            self.reload(pattern, &color_of);
        }
        self.runner.set_running(!running);
    }

    /// Coser las costuras de una en una (en el orden del patrón) o todas a
    /// la vez. Vuelve a empezar, para ver el montaje desde el principio.
    pub fn set_sew_in_order(&mut self, on: bool, pattern: &Pattern, color_of: impl Fn(PieceId) -> Color32) {
        self.settings.sew_in_order = on;
        self.reload(pattern, &color_of);
        self.runner.set_running(true);
    }

    /// La etapa de la línea de montaje que se está cosiendo ahora (0 = las
    /// costuras sin etapa; `k` = la etapa `k`, contando desde 1), o `None`
    /// si no se está cosiendo.
    pub fn sewing_stage(&self) -> Option<usize> {
        let stats = self.runner.stats();
        let turn = stats.turn.filter(|_| stats.sewing)?;
        self.loaded.as_ref()?.turn_stages.get(turn as usize).copied()
    }

    /// Lo que se ha visto al montar la escena: piezas que no se han podido
    /// mallar y costuras que se retuercen al colocar las piezas.
    pub fn report(&self) -> SceneReport {
        self.loaded
            .as_ref()
            .map_or_else(SceneReport::default, |l| SceneReport { failed: l.failed.clone(), crossed: l.crossed.clone() })
    }

    /// Vuelve a la cámara inicial.
    pub fn home(&mut self) {
        self.fly_to(self.home);
    }

    fn fly_to(&mut self, to: Camera) {
        self.fly = Some(viewcube::Fly::new(self.camera, to));
    }

    /// Grupos de la etapa «Colocación» de la cinta: cómo se colocan las
    /// piezas antes de coser y ver esa colocación en 3D.
    pub fn placement_ribbon(&mut self, ui: &mut Ui, pattern: &Pattern, color_of: impl Fn(PieceId) -> Color32) {
        let settings_before = self.settings;
        widgets::ribbon_group(ui, "COLOCAR", false, |ui, _| {
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    widgets::prop_label(ui, "Modo");
                    egui::ComboBox::from_id_salt("placement")
                        .width(130.0)
                        .selected_text(self.settings.placement.label())
                        .show_ui(ui, |ui| {
                            for placement in Placement::ALL {
                                ui.selectable_value(&mut self.settings.placement, placement, placement.label())
                                    .on_hover_text(placement.hint());
                            }
                        });
                });
                ui.checkbox(&mut self.settings.sew, "Coser las costuras")
                    .on_hover_text("Cierra las costuras del patrón");
            });
        });
        widgets::ribbon_group(ui, "VER", false, |ui, _| {
            if widgets::ribbon_button(ui, Icon::Placement, "Colocación", false, true)
                .on_hover_text("Vuelve a colocar las piezas y para, para ver dónde empieza cada una antes de coser")
                .clicked()
            {
                self.reload(pattern, &color_of);
                self.runner.set_running(false);
            }
            let running = self.runner.is_running();
            let (icon, label) = if running { (Icon::Pause, "Pausa") } else { (Icon::Play, "Coser") };
            if widgets::ribbon_button(ui, icon, label, running, true).clicked() {
                self.toggle_running(pattern, &color_of);
            }
        });
        if self.settings != settings_before {
            self.update_live(pattern, &color_of);
        }
    }

    /// Grupos de la etapa «Simulación» de la cinta.
    pub fn ribbon(&mut self, ui: &mut Ui, pattern: &Pattern, color_of: impl Fn(PieceId) -> Color32) {
        let settings_before = self.settings;
        let running = self.runner.is_running();
        widgets::ribbon_group(ui, "SIMULAR", false, |ui, _| {
            let (icon, label) = if running { (Icon::Pause, "Pausa") } else { (Icon::Play, "Simular") };
            if widgets::ribbon_button(ui, icon, label, running, true).clicked() {
                self.toggle_running(pattern, &color_of);
            }
            if widgets::ribbon_button(ui, Icon::Restart, "Reiniciar", false, true)
                .on_hover_text("Vuelve a colocar las piezas y empieza de nuevo")
                .clicked()
            {
                self.reload(pattern, &color_of);
            }
            if widgets::ribbon_button(ui, Icon::Live, "En vivo", self.live, true)
                .on_hover_text("Al editar el patrón, la prenda se reajusta sin volver a empezar")
                .clicked()
            {
                self.live = !self.live;
            }
        });
        widgets::ribbon_group(ui, "ENTORNO", false, |ui, _| {
            egui::Grid::new("environment").num_columns(2).spacing(egui::vec2(6.0, 2.0)).show(ui, |ui| {
                widgets::prop_label(ui, "Obstáculo");
                egui::ComboBox::from_id_salt("obstacle")
                    .width(110.0)
                    .selected_text(self.settings.obstacle.label())
                    .show_ui(ui, |ui| {
                        for obstacle in Obstacle::ALL {
                            ui.selectable_value(&mut self.settings.obstacle, obstacle, obstacle.label());
                        }
                    });
                ui.end_row();
            });
            if self.settings.obstacle != Obstacle::Mannequin {
                self.object_obstacle = self.settings.obstacle;
            }
        });
        widgets::ribbon_group(ui, "AJUSTES", false, |ui, _| {
            let response = widgets::ribbon_button(ui, Icon::Settings, "Precisión", false, true)
                .on_hover_text("Tamaño de la malla, subpasos y auto-colisión");
            egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                ui.set_min_width(220.0);
                self.settings_menu(ui);
            });
        });
        widgets::ribbon_group(ui, "VISTA", false, |ui, _| {
            if widgets::ribbon_button(ui, Icon::Home, "Inicio", false, true)
                .on_hover_text("Encuadrar la cámara")
                .clicked()
            {
                self.home();
            }
        });
        if self.settings != settings_before {
            // Cambiar la malla no desmonta la prenda.
            self.update_live(pattern, &color_of);
        }
    }

    /// Ajustes finos de la simulación.
    fn settings_menu(&mut self, ui: &mut Ui) {
        ui.label("Tamaño de los triángulos");
        ui.add(egui::Slider::new(&mut self.settings.mesh_size, 8.0..=60.0).suffix(" mm"));
        ui.label("Subpasos por fotograma");
        ui.add(egui::Slider::new(&mut self.settings.substeps, 5..=60))
            .on_hover_text("Más subpasos: la tela se estira menos, pero cuesta más");
        ui.checkbox(&mut self.settings.self_collision, "Auto-colisión")
            .on_hover_text("Que la tela no se atraviese a sí misma (un panel con otro)");
    }

    /// Estado de la simulación para la barra de estado.
    pub fn status(&self, ui: &mut Ui, pattern: &Pattern) {
        if let Some(info) = &self.adapter {
            let (text, color) = if gpu::is_software(info) {
                ("Sin GPU: dibujando con la CPU", theme::WARN)
            } else {
                (info.name.as_str(), theme::TEXT_WEAK)
            };
            ui.label(egui::RichText::new(text).color(color)).on_hover_text(gpu::describe(info));
            widgets::separator_v(ui, 18.0);
        }
        let Some(loaded) = self.loaded.as_ref() else { return };
        let stats = self.runner.stats();
        if !loaded.failed.is_empty() {
            let names: Vec<&str> =
                loaded.failed.iter().filter_map(|&id| pattern.piece(id)).map(|p| p.name.as_str()).collect();
            ui.colored_label(theme::ERROR, format!("No se han podido mallar: {}", names.join(", ")));
        } else if self.rejected.as_ref() == Some(pattern) {
            ui.colored_label(
                theme::WARN,
                "Hay una pieza que no se puede mallar: la prenda se actualizará cuando se arregle",
            );
        } else if self.outdated(pattern) && !self.live {
            ui.colored_label(theme::WARN, "El patrón ha cambiado: pulsa Reiniciar para verlo");
        }
        if stats.sewing {
            let step = self.sewing_stage().and_then(|k| Some((k, loaded.pattern.steps.get(k.checked_sub(1)?)?)));
            match step {
                Some((k, step)) => ui.colored_label(
                    theme::ACCENT,
                    format!("Cosiendo «{}» (etapa {k} de {})…", step.name, loaded.pattern.steps.len()),
                ),
                None => ui.colored_label(theme::ACCENT, "Cosiendo…"),
            };
        }
        if pattern.pressure > 0.0 || stats.pressure > 0.0 {
            ui.label(
                egui::RichText::new(format!("relleno {:.0} Pa, {:.1} l", stats.pressure, stats.volume * 1e3))
                    .color(theme::TEXT_WEAK),
            );
        }
        let slow = if stats.step_ms > scene::FRAME * 1000.0 { " (más lento que el tiempo real)" } else { "" };
        ui.label(
            egui::RichText::new(format!(
                "{:.1} s · {} partículas · {} costuras · {:.1} ms/paso{slow}",
                stats.sim_time,
                loaded.colors.len(),
                loaded.seams,
                stats.step_ms
            ))
            .color(theme::TEXT_WEAK),
        );
    }
    /// La vista 3D, con el cubo de vista y la barra de navegación encima.
    pub fn viewport(&mut self, ui: &mut Ui, pattern: &Pattern, color_of: impl Fn(PieceId) -> Color32) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let Some(renderer) = self.renderer.as_mut() else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "La vista 3D necesita una tarjeta gráfica compatible con wgpu.",
                egui::FontId::proportional(14.0),
                ui.visuals().weak_text_color(),
            );
            return;
        };

        // Cámara: arrastrar para girar, botón derecho / central o Mayús para
        // desplazar, rueda para acercar, doble clic para volver a encuadrar.
        if let Some(fly) = self.fly {
            match fly.camera() {
                Some(camera) => {
                    self.camera = camera;
                    ui.ctx().request_repaint();
                }
                None => {
                    self.camera = fly.end();
                    self.fly = None;
                }
            }
        }
        let shift = ui.input(|i| i.modifiers.shift);
        if response.dragged() {
            self.fly = None;
        }
        if response.dragged_by(PointerButton::Primary) && !shift {
            self.camera.orbit(response.drag_delta());
        } else if response.dragged() {
            self.camera.pan(response.drag_delta(), rect.height());
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.fly = None;
                self.camera.zoom(scroll);
            }
        }
        if response.double_clicked() {
            self.fly = Some(viewcube::Fly::new(self.camera, self.home));
        }

        if let Some(loaded) = &self.loaded {
            if self.obstacle != Some(loaded.settings.obstacle) {
                let (mut vertices, mut indices) = (Vec::new(), Vec::new());
                if let Some(avatar) = loaded.settings.obstacle.avatar() {
                    let normals = simulator::vertex_normals(&avatar.positions, &avatar.triangles);
                    vertices.extend(
                        avatar.positions.iter().zip(&normals).map(|(p, n)| Vertex::plain(*p, *n, MANNEQUIN_COLOR)),
                    );
                    indices.extend(avatar.triangles.iter().flatten());
                }
                match loaded.settings.obstacle.collider() {
                    Some(simulator::Collider::Sphere { center, radius }) => {
                        render::push_sphere(&mut vertices, &mut indices, center, radius, OBSTACLE_COLOR)
                    }
                    Some(simulator::Collider::Cuboid { center, half_extents }) => {
                        render::push_cuboid(&mut vertices, &mut indices, center, half_extents, OBSTACLE_COLOR)
                    }
                    _ => {}
                }
                renderer.set_obstacles(&vertices, &indices);
                self.obstacle = Some(loaded.settings.obstacle);
            }
            let version = self.runner.read(self.version, &mut self.positions);
            let changed = version != self.version || self.drawn_highlight != self.highlight_seams;
            if changed && self.positions.len() == loaded.colors.len() {
                let normals = simulator::vertex_normals(&self.positions, &loaded.triangles);
                let vertices: Vec<Vertex> = (0..self.positions.len())
                    .map(|i| Vertex {
                        position: self.positions[i].to_array(),
                        normal: normals[i].to_array(),
                        color: loaded.colors[i],
                        back: loaded.back_colors[i],
                    })
                    .collect();
                renderer.set_cloth(&vertices, &loaded.indices);
                let mut mesh = seams::SeamMesh::default();
                let seam_color = |i: usize| {
                    let c = egui::Rgba::from(crate::canvas::seam_color(i));
                    [c.r(), c.g(), c.b()]
                };
                seams::build(
                    &loaded.seam_draws,
                    &self.positions,
                    &normals,
                    self.highlight_seams,
                    seam_color,
                    &mut mesh,
                );
                seams::fasteners(&loaded.fasteners, &self.positions, &normals, &mut mesh);
                renderer.set_seams(&mesh.vertices, &mesh.indices);
                self.version = version;
                self.drawn_highlight = self.highlight_seams;
            }
        }

        let ppp = ui.ctx().pixels_per_point();
        let size = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let background = egui::Rgba::from(ui.visuals().extreme_bg_color);
        let texture = renderer.render(
            size,
            self.camera.view_proj(rect.width() / rect.height().max(1.0)),
            self.camera.eye(),
            background,
        );
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter_at(rect).image(texture, rect, uv, Color32::WHITE);
        if !self.runner.is_running() && self.runner.stats().sim_time == 0.0 {
            ui.painter_at(rect).text(
                rect.center_top() + egui::vec2(0.0, 16.0),
                egui::Align2::CENTER_TOP,
                "En pausa: pulsa ▶ para coser",
                egui::FontId::proportional(14.0),
                ui.visuals().strong_text_color(),
            );
        }

        widgets::viewport_label(ui, rect, "Vista 3D");
        if let Some((yaw, pitch)) = viewcube::show(ui, rect, &self.camera) {
            self.fly_to(Camera { yaw, pitch, ..self.camera });
        }
        widgets::nav_bar(ui, rect, "3d", |ui| {
            if widgets::icon_button(ui, Icon::Home, "Inicio: encuadrar la cámara (doble clic)", false).clicked() {
                self.home();
            }
            widgets::separator_v(ui, 24.0);
            let running = self.runner.is_running();
            let (icon, tip) = if running { (Icon::Pause, "Pausar la simulación") } else { (Icon::Play, "Simular") };
            if widgets::icon_button(ui, icon, tip, running).clicked() {
                self.toggle_running(pattern, &color_of);
            }
            if widgets::icon_button(ui, Icon::Restart, "Reiniciar la simulación", false).clicked() {
                self.reload(pattern, &color_of);
            }
            if widgets::icon_button(ui, Icon::Live, "En vivo: reajustar la prenda al editar", self.live).clicked() {
                self.live = !self.live;
            }
        });
    }
}
