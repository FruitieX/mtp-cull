@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    let p = positions[index];
    var out: Vertex;
    out.position = vec4(p, 0.0, 1.0);
    out.uv = p * vec2(0.5, -0.5) + vec2(0.5);
    return out;
}

@fragment
fn fs_main(in: Vertex) -> @location(0) vec4<f32> {
    // Each destination pixel averages its source footprint. Successive levels
    // form a low-pass pyramid; the viewer interpolates between levels on zoom.
    return textureSampleLevel(source, source_sampler, in.uv, 0.0);
}
