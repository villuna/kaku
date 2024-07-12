struct VertexInput {
    @location(0) tex_coord: vec2<f32>,
};

struct CharacterInstance {
    @location(1) char_position: vec2<f32>,
    @location(2) size: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) vertex_position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct SdfTextSettings {
    @location(0) colour: vec4<f32>,
    @location(1) outline_colour: vec4<f32>,
    @location(2) text_position: vec2<f32>,
    @location(3) outline_width: f32,
    @location(4) sdf_radius: f32,
    @location(5) image_scale: f32,
};

// Projection matrix that allows us to draw in pixel coords
@group(0) @binding(0)
var<uniform> screen: mat4x4<f32>;

@group(2) @binding(0)
var<uniform> settings: SdfTextSettings;

@vertex
fn vs_main(vertex: VertexInput, instance: CharacterInstance) -> VertexOutput {
    var out: VertexOutput;

    var position = instance.char_position + settings.text_position + vertex.tex_coord * instance.size;
    out.vertex_position = screen * vec4<f32>(position, 0.0, 1.0);
    out.tex_coord = vertex.tex_coord;
    return out;
}

@group(1) @binding(0)
var texture: texture_2d<f32>;
@group(1) @binding(1)
var texture_sampler: sampler;

// function to scale distance according to sdf spread
fn scale_distance(value: f32, radius: f32) -> f32 {
    return (value - 0.5) * 2.0 * radius;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let value = textureSample(texture, texture_sampler, input.tex_coord).r;
    let distance = scale_distance(value, settings.sdf_radius);
    let aa_thresh = 1.0 / settings.image_scale;
    let radius = settings.outline_width / settings.image_scale;
    let outline_alpha = smoothstep(radius + aa_thresh, radius - aa_thresh, distance) * settings.outline_colour.a;

    return vec4<f32>(settings.outline_colour.rgb, outline_alpha);
}
