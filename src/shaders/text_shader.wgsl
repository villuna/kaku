struct VertexInput {
    @location(0) tex_coord: vec2<f32>,
};

struct CharacterInstance {
    @location(1) position: vec2<f32>,
    @location(2) size: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

// Projection matrix that allows us to draw in pixel coords
@group(0) @binding(0)
var<uniform> screen: mat4x4<f32>;

@vertex
fn vs_main(vertex: VertexInput, instance: CharacterInstance) -> VertexOutput {
    var out: VertexOutput;

    var position = instance.position + vertex.tex_coord * instance.size;
    out.position = screen * vec4<f32>(position, 0.0, 1.0);
    out.tex_coord = vertex.tex_coord;
    return out;
}

struct TextSettings {
    @location(0) colour: vec4<f32>,
};

@group(2) @binding(0)
var<uniform> settings: TextSettings;

@group(1) @binding(0)
var texture: texture_2d<f32>;
@group(1) @binding(1)
var texture_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = textureSample(texture, texture_sampler, input.tex_coord).r;
    return vec4<f32>(settings.colour.rgb, settings.colour.a * alpha);
}
