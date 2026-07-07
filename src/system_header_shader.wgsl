struct SystemHeaderUniforms {
    params0: vec4<f32>,
    params1: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: SystemHeaderUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

const COS_30: f32 = 0.86602540378;
const SIN_30: f32 = 0.5;
const STRIPE_WIDTH: f32 = 16.0;
const STRIPE_PERIOD: f32 = STRIPE_WIDTH * 2.0;

fn header_size() -> vec2<f32> {
    return uniforms.params0.xy;
}

fn header_phase() -> f32 {
    return uniforms.params0.z;
}

fn pixels_per_point() -> f32 {
    return max(uniforms.params0.w, 1.0);
}

fn accent_color() -> vec3<f32> {
    return uniforms.params1.rgb;
}

fn smoothstep01(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
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
    let size = header_size();
    let local = vec2(in.uv.x * size.x, (1.0 - in.uv.y) * size.y);
    let stripe_coord = local.x * COS_30 - local.y * SIN_30 - header_phase();
    let band = stripe_coord - floor(stripe_coord / STRIPE_PERIOD) * STRIPE_PERIOD;
    let aa = max(0.75 / pixels_per_point(), 0.35);
    let edge_dist = abs(band - STRIPE_WIDTH * 0.5) - STRIPE_WIDTH * 0.5;
    let accent_mix = 1.0 - smoothstep01(-aa, aa, edge_dist);
    let color = mix(vec3(0.0, 0.0, 0.0), accent_color(), accent_mix);
    return vec4(color, 1.0);
}
