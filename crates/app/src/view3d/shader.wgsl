// Sombreado de la vista 3D: tela (por las dos caras), obstáculos y suelo.

struct Uniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // rgb: color de fondo (para fundir el suelo a lo lejos).
    background: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

@vertex
fn vs(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(v.position, 1.0);
    out.world = v.position;
    out.normal = v.normal;
    out.color = v.color;
    return out;
}

fn shade(base: vec3<f32>, n: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let key = normalize(vec3<f32>(0.4, 1.0, 0.6));
    let fill = normalize(u.eye.xyz - world);
    let diffuse = max(dot(n, key), 0.0) * 0.65 + max(dot(n, fill), 0.0) * 0.25;
    // Luz de cielo: más clara arriba que abajo.
    let sky = mix(0.18, 0.32, n.y * 0.5 + 0.5);
    return base * (sky + diffuse);
}

@fragment
fn fs_mesh(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    var base = in.color;
    if !front {
        // Revés de la tela: misma tela, algo más apagada.
        n = -n;
        base = base * 0.7;
    }
    return vec4<f32>(shade(base, n, in.world), 1.0);
}

@fragment
fn fs_floor(in: VertexOut) -> @location(0) vec4<f32> {
    // Rejilla de 10 cm con líneas cada metro más marcadas.
    let p = in.world.xz;
    let w = fwidth(p);
    let fine = abs(fract(p * 10.0 - 0.5) - 0.5) / (w * 10.0);
    let coarse = abs(fract(p - 0.5) - 0.5) / w;
    let line_fine = 1.0 - min(min(fine.x, fine.y), 1.0);
    let line_coarse = 1.0 - min(min(coarse.x, coarse.y), 1.0);
    var color = shade(in.color, vec3<f32>(0.0, 1.0, 0.0), in.world);
    color = mix(color, color * 0.8, line_fine * 0.5);
    color = mix(color, color * 0.6, line_coarse);
    // Se funde con el fondo a lo lejos.
    let dist = length(in.world - u.eye.xyz);
    let fog = smoothstep(4.0, 12.0, dist);
    return vec4<f32>(mix(color, u.background.rgb, fog), 1.0);
}
