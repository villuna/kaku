#![warn(missing_docs)]
//! A font rendering crate for rendering text using signed distance fields.
//!
//! This crate aims to provide a general and easy to use API for rendering text using [wgpu].
//! It can render text normally (using raster images), or with signed distance fields, which
//! allows for performant scaling and outlining.
//!
//! # Usage
//!
//! Here is an example of how to use the crate. You first need to create a [TextRenderer] struct,
//! then load a font using [ab_glyph], then you can create a [Text] object, which is the thing that
//! can be drawn.
//!
//! ```rust
//! let mut text_renderer =
//!     TextRendererBuilder::new(target_format, target_size).build(&device);
//!     
//! let font = ab_glyph::FontRef::try_from_slice(include_bytes!("FiraSans-Regular.ttf"))?;
//! let font = text_renderer.load_font_with_sdf(font, 45., SdfSettings { radius: 15. });
//!
//! let text = TextBuilder::new("Hello, world!", font, [100., 100.])
//!     .outlined([1.; 4], 10.)
//!     .build(&device, &queue, &mut text_renderer);
//! ```
//!
//! You can then draw this text object during a render pass like so:
//!
//! ```rust
//! text_renderer.draw(&mut render_pass, &text);
//! ```
//!
//! # Performance
//!
//! Calculating the signed distance field for a character takes a small but not-insignificant
//! amount of time. This will only happen once for each character in a font, and can be done ahead
//! of time using [TextRenderer::generate_char_textures], but is still a cost. If you don't need
//! the features provided by sdf rendering, you should use non-sdf rendering instead.

mod sdf;
mod text;

pub use text::{FontSize, HorizontalAlignment, Text, TextBuilder, VerticalAlignment};

use image::GrayImage;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use text::TextData;

use std::num::NonZeroU64;

pub use ab_glyph;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use ahash::AHashMap;
use itertools::Itertools;
use log::info;
use sdf::create_sdf_texture;
use text::{SdfSettingsUniform, SettingsUniform};
use wgpu::{
    include_wgsl, util::DeviceExt, DepthStencilState, TextureFormat, TextureViewDescriptor,
};

type HashMap<K, V> = AHashMap<K, V>;

pub use sdf::SdfSettings;

#[derive(Debug)]
struct CharTexture {
    bind_group: wgpu::BindGroup,
    position: [f32; 2],
    size: [f32; 2],
}

#[derive(Debug)]
struct Character {
    /// The texture for the glyph. Optional since characters that are e.g. unrecognised or
    /// whitespace only might not have a texture.
    texture: Option<CharTexture>,
    /// The amount of space to leave after this character
    advance: f32,
}

type CharacterCache = HashMap<char, Character>;

/// A handle to a font stored in the [TextRenderer].
///
/// When you load a font into the text renderer using [TextRenderer::load_font], it will give you
/// back one of these IDs referencing that font.
#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy, Ord, PartialOrd)]
pub struct FontId(usize);

#[derive(Debug)]
struct FontData {
    font: FontArc,
    px_size: f32,
    scale: PxScale,
    char_cache: CharacterCache,
    sdf_settings: Option<SdfSettings>,
}

impl FontData {
    fn new(font: FontArc, size: FontSize) -> Self {
        let scale = size.scale(&font);
        let px_size = size.px_size(&font);

        Self {
            font,
            scale,
            px_size,
            sdf_settings: None,
            char_cache: Default::default(),
        }
    }

    fn new_with_sdf(font: FontArc, size: FontSize, sdf_settings: SdfSettings) -> Self {
        let scale = size.scale(&font);
        let px_size = size.px_size(&font);

        Self {
            font,
            scale,
            px_size,
            sdf_settings: Some(sdf_settings),
            char_cache: Default::default(),
        }
    }
}

#[derive(Default, Debug)]
struct FontMap {
    fonts: Vec<FontData>,
}

impl FontMap {
    /// Load a font into the map
    fn load(&mut self, font: FontArc, size: FontSize) -> FontId {
        let id = self.fonts.len();
        self.fonts.push(FontData::new(font, size));
        FontId(id)
    }

    /// Load a font into the map with sdf rendering enabled
    fn load_with_sdf(
        &mut self,
        font: FontArc,
        size: FontSize,
        sdf_settings: SdfSettings,
    ) -> FontId {
        let id = self.fonts.len();
        self.fonts
            .push(FontData::new_with_sdf(font, size, sdf_settings));
        FontId(id)
    }

    fn get(&self, font: FontId) -> &FontData {
        self.fonts.get(font.0).expect("Font not found in renderer!")
    }

    fn get_mut(&mut self, font: FontId) -> &mut FontData {
        self.fonts
            .get_mut(font.0)
            .expect("Font not found in renderer!")
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Zeroable, bytemuck::Pod)]
struct ScreenUniform {
    projection: [[f32; 4]; 4],
}

impl ScreenUniform {
    fn new(target_size: (u32, u32)) -> Self {
        let width = target_size.0 as f32;
        let height = target_size.1 as f32;
        let sx = 2.0 / width;
        let sy = -2.0 / height;

        // Note that wgsl matrices are *column-major*
        // which means each sub-array is one column, not one row
        // i found that out the hard way
        ScreenUniform {
            projection: [
                [sx, 0.0, 0.0, 0.0],
                [0.0, sy, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0, 1.0],
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Zeroable, bytemuck::Pod)]
struct TextureVertex {
    tex_coord: [f32; 2],
}

/// Creates vertex data to draw a quad with given position and size
const TEXTURE_VERTICES: [TextureVertex; 4] = [
    TextureVertex {
        tex_coord: [0.0, 0.0],
    },
    TextureVertex {
        tex_coord: [0.0, 1.0],
    },
    TextureVertex {
        tex_coord: [1.0, 0.0],
    },
    TextureVertex {
        tex_coord: [1.0, 1.0],
    },
];

fn texture_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<TextureVertex>() as _,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &const {
            wgpu::vertex_attr_array![
                0 => Float32x2,
            ]
        },
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Zeroable, bytemuck::Pod)]
struct CharacterInstance {
    /// The position of the top-left corner
    position: [f32; 2],
    /// The width and height of the box
    size: [f32; 2],
}

fn character_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<CharacterInstance>() as _,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &const {
            wgpu::vertex_attr_array![
                1 => Float32x2,
                2 => Float32x2,
            ]
        },
    }
}

/// A builder for a [TextRenderer] struct.
#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct TextRendererBuilder {
    target_format: wgpu::TextureFormat,
    target_size: (u32, u32),
    msaa_samples: u32,
    depth_format: Option<TextureFormat>,
}

impl TextRendererBuilder {
    /// Creates a new TextRendererBuilder.
    ///
    /// This function takes in the format of the target surface that the TextRenderer will draw
    /// to, and the size of the target surface.
    pub fn new(target_format: wgpu::TextureFormat, target_size: (u32, u32)) -> Self {
        Self {
            target_format,
            target_size,
            msaa_samples: 1,
            depth_format: None,
        }
    }

    /// Sets the number of samples to use for multisampling. The default is 1 (no multisampling).
    ///
    /// Text rendered this way doesn't really benefit from multisampling, so this won't make the
    /// text look any better. Instead, this option is used if you want to draw on a render pass
    /// that already uses multisampling.
    pub fn with_msaa_sample_count(mut self, samples: u32) -> Self {
        self.msaa_samples = samples;
        self
    }

    /// Sets the format of the depth buffer.
    ///
    /// By default the renderer will only be compatible with render passes that don't use a depth
    /// buffer. If yours does use a depth buffer, you will want to set this option.
    pub fn with_depth(mut self, depth_format: TextureFormat) -> Self {
        self.depth_format = Some(depth_format);
        self
    }

    /// Creates a new TextRenderer from the current configuration.
    pub fn build(self, device: &wgpu::Device) -> TextRenderer {
        TextRenderer::new(
            device,
            self.target_format,
            self.target_size,
            self.msaa_samples,
            self.depth_format,
        )
    }
}

fn create_text_pipeline(
    label: &str,
    layout: &wgpu::PipelineLayout,
    render_format: wgpu::TextureFormat,
    samples: u32,
    shader: &wgpu::ShaderModule,
    depth_format: Option<TextureFormat>,
    device: &wgpu::Device,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            buffers: &[texture_vertex_layout(), character_instance_layout()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: render_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: depth_format.map(|format| DepthStencilState {
            format,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
    })
}

#[derive(Debug)]
/// The main struct that handles text rendering to the screen. Use this struct to load fonts and
/// draw text during a render pass.
///
/// Create one with a [TextRendererBuilder].
pub struct TextRenderer {
    fonts: FontMap,
    char_bind_group_layout: wgpu::BindGroupLayout,

    screen_bind_group: wgpu::BindGroup,
    screen_buffer: wgpu::Buffer,

    pub(crate) settings_layout: wgpu::BindGroupLayout,
    pub(crate) sdf_settings_layout: wgpu::BindGroupLayout,

    vertex_buffer: wgpu::Buffer,

    basic_pipeline: wgpu::RenderPipeline,
    sdf_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
}

impl TextRenderer {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        target_size: (u32, u32),
        msaa_samples: u32,
        depth_stencil_state: Option<TextureFormat>,
    ) -> Self {
        // Texture bind group layout to use when creating cached char textures
        let char_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("kaku character texture bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        // The screen uniform is a matrix that transforms pixel coords into screen coords
        let screen_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("kaku screen uniform bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(std::mem::size_of::<ScreenUniform>() as _),
                        },
                        count: None,
                    }
                ]
            });

        let screen_uniform = ScreenUniform::new(target_size);

        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaku screen uniform buffer"),
            contents: bytemuck::cast_slice(&[screen_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kaku screen uniform bind group"),
            layout: &screen_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_buffer.as_entire_binding(),
            }],
        });

        // The settings bind group for a piece of text details how it should be drawn in the
        // fragment stage
        let settings_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kaku text settings uniform bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<SettingsUniform>() as _),
                },
                count: None,
            }],
        });

        let sdf_settings_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("kaku sdf text settings uniform bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<SdfSettingsUniform>() as _,
                        ),
                    },
                    count: None,
                }],
            });

        // The render pipeline to use to render the text with no sdf
        let basic_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kaku text rendering pipeline layout"),
                bind_group_layouts: &[
                    &screen_bind_group_layout,
                    &char_bind_group_layout,
                    &settings_layout,
                ],
                push_constant_ranges: &[],
            });

        let basic_shader = device.create_shader_module(include_wgsl!("shaders/text_shader.wgsl"));

        let basic_pipeline = create_text_pipeline(
            "kaku basic text render pipeline",
            &basic_pipeline_layout,
            target_format,
            msaa_samples,
            &basic_shader,
            depth_stencil_state,
            device,
        );

        // The render pipeline to use to render the text with no sdf
        let sdf_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kaku sdf text rendering pipeline layout"),
            bind_group_layouts: &[
                &screen_bind_group_layout,
                &char_bind_group_layout,
                &sdf_settings_layout,
            ],
            push_constant_ranges: &[],
        });

        let sdf_shader = device.create_shader_module(include_wgsl!("shaders/sdf_text_shader.wgsl"));

        let sdf_pipeline = create_text_pipeline(
            "kaku sdf text render pipeline",
            &sdf_pipeline_layout,
            target_format,
            msaa_samples,
            &sdf_shader,
            depth_stencil_state,
            device,
        );

        let outline_shader =
            device.create_shader_module(include_wgsl!("shaders/sdf_outline_shader.wgsl"));

        let outline_pipeline = create_text_pipeline(
            "kaku sdf text outline render pipeline",
            &sdf_pipeline_layout,
            target_format,
            msaa_samples,
            &outline_shader,
            depth_stencil_state,
            device,
        );

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaku character vertex buffer"),
            contents: bytemuck::cast_slice(&TEXTURE_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            fonts: Default::default(),
            char_bind_group_layout,
            settings_layout,
            basic_pipeline,
            screen_bind_group,
            screen_buffer,
            vertex_buffer,
            sdf_settings_layout,
            sdf_pipeline,
            outline_pipeline,
        }
    }

    /// Configure the text renderer to draw to a surface with the given dimensions.
    ///
    /// You want to use this when the window resizes. You might also want to use it before drawing
    /// to a texture which is smaller than the screen, if you so choose.
    pub fn resize(&self, new_size: (u32, u32), queue: &wgpu::Queue) {
        let screen_uniform = ScreenUniform::new(new_size);
        queue.write_buffer(
            &self.screen_buffer,
            0,
            bytemuck::cast_slice(&[screen_uniform]),
        );
    }

    /// Loads a font for use in the text renderer.
    pub fn load_font<F>(&mut self, font: F, size: FontSize) -> FontId
    where
        F: Font + Send + Sync + 'static,
    {
        self.fonts.load(FontArc::new(font), size)
    }

    /// Loads a font for use in the text renderer with sdf rendering.
    ///
    /// Sny font can be used for sdf rendering. A font with SDF enabled can be scaled up without
    /// pixellation, and can have effects applied to it. However, creating the textures for each
    /// character will take longer and the textures will take up more space on the GPU. So if you
    /// don't need any of these effects, use [TextRenderer::load_font] instead.
    pub fn load_font_with_sdf<F>(
        &mut self,
        font: F,
        size: FontSize,
        sdf_settings: SdfSettings,
    ) -> FontId
    where
        F: Font + Send + Sync + 'static,
    {
        self.fonts
            .load_with_sdf(FontArc::new(font), size, sdf_settings)
    }

    /// Draws a [Text] object to the given render pass.
    pub fn draw_text<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        text: &'pass Text,
    ) {
        // Set the pipeline depending on if the font uses sdf
        let use_sdf = self.font_uses_sdf(text.data.font);
        let use_outline = text.data.sdf.is_some_and(|sdf| sdf.outline.is_some());

        if use_sdf {
            render_pass.set_pipeline(&self.sdf_pipeline);
        } else {
            render_pass.set_pipeline(&self.basic_pipeline);
        }

        let font_data = self.fonts.get(text.data.font);

        render_pass.set_bind_group(0, &self.screen_bind_group, &[]);
        render_pass.set_bind_group(2, &text.settings_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, text.instance_buffer.slice(..));

        if use_outline {
            render_pass.set_pipeline(&self.outline_pipeline);

            let mut i = 0;
            for c in text.data.text.chars() {
                let char_data = font_data.char_cache.get(&c).unwrap();

                if let Some(texture) = &char_data.texture {
                    render_pass.set_bind_group(1, &texture.bind_group, &[]);
                    render_pass.draw(0..4, i as u32..i as u32 + 1);
                    i += 1;
                }
            }

            render_pass.set_pipeline(&self.sdf_pipeline);
        }

        let mut i = 0;
        for c in text.data.text.chars() {
            let char_data = font_data.char_cache.get(&c).unwrap();

            if let Some(texture) = &char_data.texture {
                render_pass.set_bind_group(1, &texture.bind_group, &[]);
                render_pass.draw(0..4, i as u32..i as u32 + 1);
                i += 1;
            }
        }
    }

    /// Returns whether a given font was loaded with sdf enabled.
    pub fn font_uses_sdf(&self, font: FontId) -> bool {
        self.fonts.get(font).sdf_settings.is_some()
    }

    fn create_text_instances(&self, text: &TextData) -> Vec<CharacterInstance> {
        let mut position = [0., 0.];
        let scale = text.scale;
        let font = self.fonts.get(text.font);
        let char_cache = &font.char_cache;
        let scaled_font = font.font.as_scaled(font.scale);
        let ascent = scaled_font.ascent() * scale;
        let descent = scaled_font.descent() * scale;
        let line_gap = scaled_font.line_gap();

        let mut instances: Vec<CharacterInstance> = text
            .text
            .lines()
            .flat_map(|line| {
                let mut instances = Vec::new();
                for c in line.chars() {
                    let char_data = char_cache.get(&c).unwrap();

                    if let Some(texture) = char_data.texture.as_ref() {
                        let x = position[0] + texture.position[0] * scale;
                        let y = position[1] + texture.position[1] * scale;

                        let w = texture.size[0] * scale;
                        let h = texture.size[1] * scale;

                        instances.push(CharacterInstance {
                            position: [x, y],
                            size: [w, h],
                        });
                    }

                    position[0] += char_data.advance * scale;
                }

                // Apply horizontal alignment line by line
                let text_width = position[0];
                let h_offset = -text_width * text.halign.proportion();

                for instance in &mut instances {
                    instance.position[0] += h_offset;
                }

                // Reset position for the next line
                position[0] = 0.;
                position[1] += ascent - descent + line_gap;

                instances
            })
            .collect_vec();

        // Apply vertical alignment to the whole text

        let v_offset = match text.valign {
            VerticalAlignment::Baseline => 0.,
            VerticalAlignment::Top => ascent,
            VerticalAlignment::Middle => ascent - (ascent - descent) * 0.5,
            VerticalAlignment::Bottom => descent,
            VerticalAlignment::Ratio(r) => ascent - (ascent - descent) * r.clamp(0., 1.),
        };

        for instance in &mut instances {
            instance.position[1] += v_offset;
        }

        instances
    }

    /// Creates and caches the character textures necessary to draw a certain string with a given
    /// font.
    ///
    /// This is called every time a new [Text] is created, but you might also want to call
    /// it yourself if you know you're going to be displaying some text in the future and want to
    /// generate the character textures in advance.
    ///
    /// For example, if you are making a game with a score display that might change every frame,
    /// you might want to cache all the characters from '0' to '9' beforehand to save this from
    /// happening between frames.
    pub fn generate_char_textures(
        &mut self,
        chars: impl Iterator<Item = char>,
        font: FontId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let char_data = {
            let font_data = self.fonts.get(font);
            let new_characters = chars
                .filter(|c| !font_data.char_cache.contains_key(c))
                .unique()
                .collect_vec();

            let font = &font_data.font;
            let scale = font_data.scale;
            let sdf = font_data.sdf_settings.as_ref();

            new_characters
                .into_par_iter()
                .map(|c| {
                    let data = match sdf {
                        None => self.create_char_texture(c, font, scale, device, queue),
                        Some(sdf) => {
                            self.create_char_texture_sdf(c, font, scale, sdf, device, queue)
                        }
                    };
                    (c, data)
                })
                .collect::<Vec<_>>()
        };

        self.fonts.get_mut(font).char_cache.extend(char_data);
    }

    fn create_char_texture_sdf(
        &self,
        c: char,
        font: &FontArc,
        scale: PxScale,
        sdf: &SdfSettings,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Character {
        info!("Creating sdf character texture for {c}");
        // Calculate metrics
        let scaled = font.as_scaled(scale);
        let glyph = font.glyph_id(c).with_scale(scale);

        let advance = scaled.h_advance(glyph.id);

        let texture = scaled.outline_glyph(glyph).map(|outlined| {
            let px_bounds = outlined.px_bounds();
            let width = px_bounds.width().ceil() as u32;
            let height = px_bounds.height().ceil() as u32;
            let mut x = px_bounds.min.x;
            let mut y = px_bounds.min.y;

            let mut image = image::GrayImage::new(width, height);
            outlined.draw(|x, y, val| image.put_pixel(x, y, image::Luma([(val * 255.) as u8])));

            let (sdf_image, padding) = create_sdf_texture(&image, (width, height), sdf);

            image = sdf_image;
            x -= padding as f32;
            y -= padding as f32;

            let bind_group = self.create_char_bind_group(c, &image, device, queue);

            CharTexture {
                bind_group,
                size: [image.width() as f32, image.height() as f32],
                position: [x, y],
            }
        });

        Character { texture, advance }
    }

    fn create_char_texture(
        &self,
        c: char,
        font: &FontArc,
        scale: PxScale,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Character {
        info!("Creating character texture for {c}");
        // Calculate metrics
        let scaled = font.as_scaled(scale);
        let glyph = font.glyph_id(c).with_scale(scale);

        let advance = scaled.h_advance(glyph.id);

        let texture = scaled.outline_glyph(glyph).map(|outlined| {
            let px_bounds = outlined.px_bounds();
            let width = px_bounds.width().ceil() as u32;
            let height = px_bounds.height().ceil() as u32;
            let x = px_bounds.min.x;
            let y = px_bounds.min.y;

            let mut image = image::GrayImage::new(width, height);
            outlined.draw(|x, y, val| image.put_pixel(x, y, image::Luma([(val * 255.) as u8])));

            let bind_group = self.create_char_bind_group(c, &image, device, queue);

            CharTexture {
                bind_group,
                size: [image.width() as f32, image.height() as f32],
                position: [x, y],
            }
        });

        Character { texture, advance }
    }

    fn create_char_bind_group(
        &self,
        c: char,
        image: &GrayImage,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> wgpu::BindGroup {
        let texture_size = wgpu::Extent3d {
            width: image.width(),
            height: image.height(),
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("kaku texture for character: '{c}'")),
            size: texture_size,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
            mip_level_count: 1,
            // TODO: multisampling
            sample_count: 1,
        });

        let view = texture.create_view(&TextureViewDescriptor {
            label: Some(&format!("kaku texture view for character: '{c}'")),
            ..Default::default()
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(image.width()),
                rows_per_image: Some(image.height()),
            },
            texture_size,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("kaku bind group for character '{c}'")),
            layout: &self.char_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        bind_group
    }
}
