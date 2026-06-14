struct CsvButtonUniforms {
    params0: vec4<f32>,
    params1: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: CsvButtonUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

const TAU: f32 = 6.28318530718;
const COS_30: f32 = 0.86602540378;
const SIN_30: f32 = 0.5;

fn button_size() -> vec2<f32> {
    return uniforms.params0.xy;
}

fn button_radius() -> f32 {
    return uniforms.params0.z;
}

fn button_phase() -> f32 {
    return uniforms.params0.w;
}

fn button_pixels_per_point() -> f32 {
    return uniforms.params1.x;
}

fn smoothstep01(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn triangle_window_coords(uv: vec2<f32>) -> vec2<f32> {
    let c = uv - vec2(0.5, 0.5);
    let rx = c.x * COS_30 + c.y * SIN_30;
    let ry = -c.x * SIN_30 + c.y * COS_30;
    return vec2(0.50 + rx * 0.72, 0.58 + ry * 0.78);
}

fn triangle_color_at(sample_uv: vec2<f32>) -> vec3<f32> {
    let red = vec2(0.50, 0.06);
    let green = vec2(0.10, 0.92);
    let blue = vec2(0.90, 0.92);

    let red_w = 1.0 / (dot(sample_uv - red, sample_uv - red) + 0.010);
    let green_w = 1.0 / (dot(sample_uv - green, sample_uv - green) + 0.010);
    let blue_w = 1.0 / (dot(sample_uv - blue, sample_uv - blue) + 0.010);
    let sum = red_w + green_w + blue_w;

    var color = vec3(red_w, green_w, blue_w) / sum;
    color = pow(color, vec3(0.85, 0.85, 0.85));
    return clamp(color, vec3(0.0), vec3(1.0));
}

fn flow_color(uv: vec2<f32>) -> vec3<f32> {
    let base = triangle_window_coords(uv);
    let bottom_band = smoothstep01(0.10, 0.96, uv.y);
    let cover_band = smoothstep01(0.16, 1.0, uv.y);
    let phase = button_phase();
    let wave_x = sin(phase * 2.4 + uv.y * 5.4 + uv.x * TAU * 1.55);
    let wave_y = cos(phase * 1.65 - uv.x * TAU * 1.10);
    let drift = sin(phase * 0.95 + uv.x * TAU * 0.75);

    let sample_x = clamp(
        base.x + bottom_band * (0.135 * wave_x + 0.045 * wave_y + 0.020 * drift),
        -0.20,
        1.20,
    );
    let flow_floor = 0.90 + 0.05 * drift + 0.03 * wave_y;
    let sample_y = mix(
        clamp(base.y + bottom_band * 0.035 * wave_y, -0.10, 1.10),
        clamp(flow_floor, 0.82, 0.98),
        cover_band * 0.42,
    );

    return triangle_color_at(vec2(sample_x, sample_y));
}

fn rounded_rect_sdf(point: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(point) - (half_size - vec2(radius, radius));
    return length(max(q, vec2(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
    );
    var uvs = array<vec2<f32>, 6>(
        vec2(0.0, 0.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(0.0, 1.0),
        vec2(1.0, 0.0),
        vec2(1.0, 1.0),
    );

    var out: VertexOutput;
    out.position = vec4(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let size = button_size();
    let local = (in.uv - vec2(0.5, 0.5)) * size;
    let dist = rounded_rect_sdf(local, size * 0.5, button_radius());
    let aa = max(1.0 / max(button_pixels_per_point(), 1.0), 0.5);
    let alpha = 1.0 - smoothstep(-aa, aa, dist);
    if (alpha <= 0.0) {
        discard;
    }

    let color_uv = vec2(in.uv.x, 1.0 - in.uv.y);
    let color = flow_color(color_uv);
    let inner_edge = 1.0 - smoothstep(0.0, aa * 1.4, abs(dist + aa * 0.6));
    let stroked = mix(color, vec3(1.0, 1.0, 1.0), inner_edge * 0.09);
    return vec4(stroked, alpha);
}
