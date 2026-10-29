//! Vista 3D: las piezas del patrón como tela simulada, cosidas entre sí y
//! cayendo sobre el suelo y, si se quiere, sobre un obstáculo (esfera, mesa
//! o el maniquí, para las prendas).

mod render;
mod scene;

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, PointerButton, Sense, Ui};
use glam::{Mat4, Vec3};
use pattern::{Pattern, PieceId};

use render::{Renderer, Vertex};
use scene::{FabricKind, Obstacle, Placement, Runner, ScenePiece, SceneSettings};

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
    failed: Vec<String>,
    seams: usize,
    pieces: Vec<ScenePiece>,
}

pub struct View3D {
    renderer: Option<Renderer>,
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
    /// Último patrón que no se ha podido pasar en vivo (alguna pieza no se
    /// puede mallar), para no reintentarlo en cada fotograma.
    rejected: Option<Pattern>,
}

impl View3D {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // El campo de distancia del maniquí tarda un poco: se prepara ya,
        // por si hace falta.
        std::thread::spawn(avatar::mannequin);
        Self {
            renderer: cc.wgpu_render_state.as_ref().map(Renderer::new),
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
            rejected: None,
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
            if scene.failed.iter().any(|name| !loaded.failed.contains(name)) {
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
            failed: scene.failed,
            seams: scene.seams,
            pieces: scene.pieces,
        });
        let params = simulator::SimParams { pressure: pattern.pressure as f32, ..self.settings.params() };
        self.runner.load(scene.cloth, scene.colliders, params, live);
        self.version = u64::MAX;
    }

    pub fn ui(&mut self, ui: &mut Ui, pattern: &Pattern, color_of: impl Fn(PieceId) -> Color32) {
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
        let settings_before = self.settings;
        let mut outdated = self.loaded.as_ref().is_some_and(|l| l.pattern != *pattern);
        let rejected = self.rejected.as_ref() == Some(pattern);
        if self.live && outdated && !rejected {
            let wait = LIVE_INTERVAL.saturating_sub(self.last_live.elapsed());
            if wait.is_zero() {
                self.update_live(pattern, &color_of);
                outdated = self.loaded.as_ref().is_some_and(|l| l.pattern != *pattern);
            } else {
                ui.ctx().request_repaint_after(wait);
            }
        }

        ui.horizontal_wrapped(|ui| {
            let running = self.runner.is_running();
            if ui.button(if running { "⏸ Pausa" } else { "▶ Simular" }).clicked() {
                if !running && outdated {
                    self.reload(pattern, &color_of);
                }
                self.runner.set_running(!running);
            }
            if ui.button("⟲ Reiniciar").on_hover_text("Vuelve a colocar las piezas y empieza de nuevo").clicked() {
                self.reload(pattern, &color_of);
            }
            ui.checkbox(&mut self.live, "En vivo")
                .on_hover_text("Al editar el patrón, la prenda se reajusta sin volver a empezar");
            ui.separator();
            egui::ComboBox::from_id_salt("placement").selected_text(self.settings.placement.label()).show_ui(
                ui,
                |ui| {
                    for placement in Placement::ALL {
                        ui.selectable_value(&mut self.settings.placement, placement, placement.label())
                            .on_hover_text(placement.hint());
                    }
                },
            );
            ui.checkbox(&mut self.settings.sew, "Coser").on_hover_text("Cierra las costuras del patrón");
            egui::ComboBox::from_id_salt("obstacle").selected_text(self.settings.obstacle.label()).show_ui(ui, |ui| {
                for obstacle in Obstacle::ALL {
                    ui.selectable_value(&mut self.settings.obstacle, obstacle, obstacle.label());
                }
            });
            if self.settings.obstacle != Obstacle::Mannequin {
                self.object_obstacle = self.settings.obstacle;
            }
            egui::ComboBox::from_id_salt("fabric").selected_text(self.settings.fabric.label()).show_ui(ui, |ui| {
                for kind in FabricKind::ALL {
                    ui.selectable_value(&mut self.settings.fabric, kind, kind.label());
                }
            });
            ui.menu_button("⚙", |ui| {
                ui.label("Tamaño de los triángulos");
                ui.add(egui::Slider::new(&mut self.settings.mesh_size, 8.0..=60.0).suffix(" mm"));
                ui.label("Subpasos por fotograma");
                ui.add(egui::Slider::new(&mut self.settings.substeps, 5..=60))
                    .on_hover_text("Más subpasos: la tela se estira menos, pero cuesta más");
                ui.checkbox(&mut self.settings.self_collision, "Auto-colisión")
                    .on_hover_text("Que la tela no se atraviese a sí misma (un panel con otro)");
                if ui.button("Encuadrar cámara").clicked() {
                    self.camera = self.home;
                }
            });
        });
        if self.settings != settings_before {
            // Cambiar el tejido o la malla no desmonta la prenda.
            self.update_live(pattern, &color_of);
        }

        let stats = self.runner.stats();
        ui.horizontal_wrapped(|ui| {
            let loaded = self.loaded.as_ref().expect("cargado arriba");
            ui.weak(format!(
                "{:.1} s · {} partículas · {} costuras · {:.1} ms por paso",
                stats.sim_time,
                loaded.colors.len(),
                loaded.seams,
                stats.step_ms
            ));
            if pattern.pressure > 0.0 || stats.pressure > 0.0 {
                ui.weak(format!("· relleno {:.0} Pa, {:.1} l", stats.pressure, stats.volume * 1e3));
            }
            if stats.step_ms > scene::FRAME * 1000.0 {
                ui.weak("(más lento que el tiempo real)");
            }
            if stats.sewing {
                ui.colored_label(Color32::from_rgb(90, 170, 230), "Cosiendo…");
            }
            if self.rejected.as_ref() == Some(pattern) {
                ui.colored_label(
                    Color32::from_rgb(230, 150, 40),
                    "Hay una pieza que no se puede mallar: la prenda se actualizará cuando se arregle.",
                );
            } else if outdated && !self.live {
                ui.colored_label(Color32::from_rgb(230, 150, 40), "El patrón ha cambiado: pulsa Reiniciar para verlo.");
            }
            if !loaded.failed.is_empty() {
                ui.colored_label(
                    Color32::from_rgb(230, 80, 80),
                    format!("No se han podido mallar: {}", loaded.failed.join(", ")),
                );
            }
        });

        self.viewport(ui);
        if self.runner.is_running() {
            ui.ctx().request_repaint();
        }
    }

    fn viewport(&mut self, ui: &mut Ui) {
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
        let shift = ui.input(|i| i.modifiers.shift);
        if response.dragged_by(PointerButton::Primary) && !shift {
            self.camera.orbit(response.drag_delta());
        } else if response.dragged() {
            self.camera.pan(response.drag_delta(), rect.height());
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom(scroll);
            }
        }
        if response.double_clicked() {
            self.camera = self.home;
        }

        if let Some(loaded) = &self.loaded {
            if self.obstacle != Some(loaded.settings.obstacle) {
                let (mut vertices, mut indices) = (Vec::new(), Vec::new());
                if let Some(avatar) = loaded.settings.obstacle.avatar() {
                    let normals = simulator::vertex_normals(&avatar.positions, &avatar.triangles);
                    vertices.extend(avatar.positions.iter().zip(&normals).map(|(p, n)| Vertex {
                        position: p.to_array(),
                        normal: n.to_array(),
                        color: MANNEQUIN_COLOR,
                    }));
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
            if version != self.version && self.positions.len() == loaded.colors.len() {
                let normals = simulator::vertex_normals(&self.positions, &loaded.triangles);
                let vertices: Vec<Vertex> = self
                    .positions
                    .iter()
                    .zip(&normals)
                    .zip(&loaded.colors)
                    .map(|((p, n), c)| Vertex { position: p.to_array(), normal: n.to_array(), color: *c })
                    .collect();
                renderer.set_cloth(&vertices, &loaded.indices);
                self.version = version;
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
                "En pausa: pulsa ▶ Simular para coser",
                egui::FontId::proportional(14.0),
                ui.visuals().strong_text_color(),
            );
        }
    }
}
