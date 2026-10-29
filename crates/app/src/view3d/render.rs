//! Dibujo de la escena 3D con `wgpu`.
//!
//! La escena se pinta en una textura propia (con antialiasing y búfer de
//! profundidad) que luego egui muestra como una imagen más. Así la vista 3D
//! no depende de cómo egui configure la ventana.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use glam::{Mat4, Vec3};

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    background: [f32; 4],
}

/// Malla en la GPU que puede crecer.
struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

impl GpuMesh {
    fn new(device: &wgpu::Device, label: &str) -> Self {
        Self {
            vertices: Self::buffer(device, label, wgpu::BufferUsages::VERTEX, 4),
            indices: Self::buffer(device, label, wgpu::BufferUsages::INDEX, 4),
            index_count: 0,
        }
    }

    fn buffer(device: &wgpu::Device, label: &str, usage: wgpu::BufferUsages, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(4).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vertices: &[Vertex], indices: &[u32]) {
        let vbytes: &[u8] = bytemuck::cast_slice(vertices);
        let ibytes: &[u8] = bytemuck::cast_slice(indices);
        if vbytes.len() as u64 > self.vertices.size() {
            self.vertices = Self::buffer(device, "vertices", wgpu::BufferUsages::VERTEX, vbytes.len() as u64 * 2);
        }
        if ibytes.len() as u64 > self.indices.size() {
            self.indices = Self::buffer(device, "indices", wgpu::BufferUsages::INDEX, ibytes.len() as u64 * 2);
        }
        queue.write_buffer(&self.vertices, 0, vbytes);
        queue.write_buffer(&self.indices, 0, ibytes);
        self.index_count = indices.len() as u32;
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.index_count > 0 {
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }
}

struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::TextureView,
    texture_id: egui::TextureId,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    egui_renderer: std::sync::Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    mesh_pipeline: wgpu::RenderPipeline,
    floor_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    floor: GpuMesh,
    obstacles: GpuMesh,
    cloth: GpuMesh,
    target: Option<Target>,
}

impl Renderer {
    pub fn new(state: &egui_wgpu::RenderState) -> Self {
        let device = state.device.clone();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("view3d"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("view3d_uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("view3d"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view3d"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("view3d"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |fragment: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3],
                    })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // La tela se ve por las dos caras.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mesh_pipeline = pipeline("fs_mesh");
        let floor_pipeline = pipeline("fs_floor");

        let mut floor = GpuMesh::new(&device, "floor");
        let (fv, fi) = floor_mesh(20.0);
        floor.upload(&device, &state.queue, &fv, &fi);
        Self {
            obstacles: GpuMesh::new(&device, "obstacles"),
            cloth: GpuMesh::new(&device, "cloth"),
            floor,
            device,
            queue: state.queue.clone(),
            egui_renderer: state.renderer.clone(),
            mesh_pipeline,
            floor_pipeline,
            uniforms,
            bind_group,
            target: None,
        }
    }

    pub fn set_obstacles(&mut self, vertices: &[Vertex], indices: &[u32]) {
        self.obstacles.upload(&self.device, &self.queue, vertices, indices);
    }

    pub fn set_cloth(&mut self, vertices: &[Vertex], indices: &[u32]) {
        self.cloth.upload(&self.device, &self.queue, vertices, indices);
    }

    /// Pinta la escena y devuelve la textura para mostrarla con egui.
    pub fn render(&mut self, size: [u32; 2], view_proj: Mat4, eye: Vec3, background: egui::Rgba) -> egui::TextureId {
        let size = [size[0].max(1), size[1].max(1)];
        self.ensure_target(size);
        let target = self.target.as_ref().expect("acabamos de crearlo");
        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            eye: eye.extend(1.0).to_array(),
            background: background.to_array(),
        };
        self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let [r, g, b, _] = background.to_array().map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view3d"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    resolve_target: Some(&target.resolve),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                        store: wgpu::StoreOp::Discard,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.floor_pipeline);
            self.floor.draw(&mut pass);
            pass.set_pipeline(&self.mesh_pipeline);
            self.obstacles.draw(&mut pass);
            self.cloth.draw(&mut pass);
        }
        self.queue.submit([encoder.finish()]);
        target.texture_id
    }

    fn ensure_target(&mut self, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let texture = |label, format, samples, usage| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = texture("view3d_msaa", COLOR_FORMAT, SAMPLES, attachment);
        let depth = texture("view3d_depth", DEPTH_FORMAT, SAMPLES, attachment);
        let resolve = texture("view3d_color", COLOR_FORMAT, 1, attachment | wgpu::TextureUsages::TEXTURE_BINDING);
        let mut egui_renderer = self.egui_renderer.write();
        let texture_id = match &self.target {
            Some(old) => {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    &self.device,
                    &resolve,
                    wgpu::FilterMode::Linear,
                    old.texture_id,
                );
                old.texture_id
            }
            None => egui_renderer.register_native_texture(&self.device, &resolve, wgpu::FilterMode::Linear),
        };
        drop(egui_renderer);
        self.target = Some(Target { size, msaa, depth, resolve, texture_id });
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if let Some(target) = &self.target {
            self.egui_renderer.write().free_texture(&target.texture_id);
        }
    }
}

fn floor_mesh(half: f32) -> (Vec<Vertex>, Vec<u32>) {
    let color = [0.55, 0.55, 0.53];
    let corner = |x: f32, z: f32| Vertex { position: [x, 0.0, z], normal: [0.0, 1.0, 0.0], color };
    let vertices = vec![corner(-half, -half), corner(half, -half), corner(half, half), corner(-half, half)];
    (vertices, vec![0, 2, 1, 0, 3, 2])
}

/// Añade una esfera (malla UV) a `vertices` / `indices`.
pub fn push_sphere(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, center: Vec3, radius: f32, color: [f32; 3]) {
    const RINGS: u32 = 32;
    const SEGMENTS: u32 = 64;
    let base = vertices.len() as u32;
    for r in 0..=RINGS {
        let phi = std::f32::consts::PI * r as f32 / RINGS as f32;
        for s in 0..=SEGMENTS {
            let theta = std::f32::consts::TAU * s as f32 / SEGMENTS as f32;
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            vertices.push(Vertex { position: (center + n * radius).to_array(), normal: n.to_array(), color });
        }
    }
    let row = SEGMENTS + 1;
    for r in 0..RINGS {
        for s in 0..SEGMENTS {
            let a = base + r * row + s;
            let b = a + row;
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
}

/// Añade una caja alineada con los ejes.
pub fn push_cuboid(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, center: Vec3, half: Vec3, color: [f32; 3]) {
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut n = Vec3::ZERO;
            n[axis] = sign;
            let u = Vec3::from_slice(&[[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]][axis]);
            let v = n.cross(u);
            let base = vertices.len() as u32;
            for (du, dv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = center + (n + u * du + v * dv) * half;
                vertices.push(Vertex { position: p.to_array(), normal: n.to_array(), color });
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}
