#![warn(missing_docs)]
//! A font rendering crate for rendering text using signed distance fields.
//!
//! This crate aims to provide a general and easy to use API for rendering text using [wgpu], mainly
//! targeting video games. It can render text normally (using raster images), or with signed
//! distance fields, which allow for performant scaling and effects such as outlines, glows and
//! shadows.
//!
//! I used [the learn wgpu tutorial](https://sotrh.github.io/learn-wgpu/) as reference for a lot of
//! the wgpu code in this crate (particularly examples).

mod sdf;
pub mod text;

use rayon::iter::{IntoParallelIterator, ParallelExtend, ParallelIterator};
use text::{Text, TextData};

use std::num::NonZeroU64;
use std::sync::RwLock;

pub use ab_glyph;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use ahash::AHashMap;
use itertools::Itertools;
use log::info;
use text::SettingsUniform;
use wgpu::{include_wgsl, util::DeviceExt, TextureViewDescriptor};

type HashMap<K, V> = AHashMap<K, V>;

#[derive(Debug)]
struct CharTexture {
    bind_group: &'static wgpu::BindGroup,
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
#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
pub struct FontId(usize);

/// Settings for how the signed distance field calculation should work for this font.
#[derive(Debug, Clone, Copy)]
pub struct SdfSettings {
    /// The sdf spread radius.
    ///
    /// This field defines the length of the distance field in pixels. This imposes a limit on the
    /// size of effects such as outlines, glow, shadows etc. A higher radius means you can create
    /// larger outlines, but will use more memory on the GPU.
    pub radius: f32,
}

struct FontData {
    font: FontArc,
    scale: PxScale,
    char_cache: RwLock<CharacterCache>,
    sdf_settings: Option<SdfSettings>,
}

impl FontData {
    fn new(font: FontArc, scale: PxScale) -> Self {
        Self {
            font,
            scale,
            sdf_settings: None,
            char_cache: Default::default(),
        }
    }

    fn new_with_sdf(font: FontArc, scale: PxScale, sdf_settings: SdfSettings) -> Self {
        Self {
            font,
            scale,
            sdf_settings: Some(sdf_settings),
            char_cache: Default::default(),
        }
    }
}

#[derive(Default)]
struct FontMap {
    fonts: Vec<FontData>,
}

impl FontMap {
    /// Load a font into the map
    fn load(&mut self, font: FontArc, size: f32) -> FontId {
        let id = self.fonts.len();
        let scale = font.pt_to_px_scale(size).unwrap();
        self.fonts.push(FontData::new(font, scale));
        FontId(id)
    }

    /// Load a font into the map with sdf rendering enabled
    fn load_with_sdf(&mut self, font: FontArc, size: f32, sdf_settings: SdfSettings) -> FontId {
        let id = self.fonts.len();
        let scale = font.pt_to_px_scale(size).unwrap();
        self.fonts
            .push(FontData::new_with_sdf(font, scale, sdf_settings));
        FontId(id)
    }

    fn get(&self, font: FontId) -> &FontData {
        // This works because we never delete fonts and the only safe way to get a fontid is through
        // this struct
        &self.fonts[font.0]
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

        // Note that wgsl constructs matrices by *row*, not by column
        // which means this is the transpose of what it should be
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

/// The main struct that handles text rendering to the screen. Use this struct to load fonts and
/// draw text during a render pass.
pub struct TextRenderer {
    fonts: FontMap,
    char_bind_group_layout: wgpu::BindGroupLayout,

    screen_bind_group: wgpu::BindGroup,
    screen_buffer: wgpu::Buffer,

    pub(crate) settings_bind_group_layout: wgpu::BindGroupLayout,

    vertex_buffer: wgpu::Buffer,

    pipeline: wgpu::RenderPipeline,
}

impl TextRenderer {
    /// Creates a new TextRenderer with no fonts loaded
    pub fn new(device: &wgpu::Device, target_config: &wgpu::SurfaceConfiguration) -> Self {
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

        // The screen bind group transforms pixel coords into screen coords
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

        let screen_uniform = ScreenUniform::new((target_config.width, target_config.height));

        // hey british guy, what's the wgpu function to create a buffer with no data?
        // "why it's device.create_buffer init"
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
        let settings_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("kaku text settings uniform bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<SettingsUniform>() as _
                        ),
                    },
                    count: None,
                }],
            });

        // The render pipeline to use to render the text
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kaku text rendering pipeline layout"),
            bind_group_layouts: &[
                &screen_bind_group_layout,
                &char_bind_group_layout,
                &settings_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(include_wgsl!("text_shader.wgsl"));

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kaku text rendering pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[texture_vertex_layout(), character_instance_layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaku character vertex buffer"),
            contents: bytemuck::cast_slice(&TEXTURE_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            fonts: Default::default(),
            char_bind_group_layout,
            settings_bind_group_layout,
            pipeline,
            screen_bind_group,
            screen_buffer,
            vertex_buffer,
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

    /// Loads a font for use in the text renderer
    pub fn load_font<F>(&mut self, font: F, size: f32) -> FontId
    where
        F: Font + Send + Sync + 'static,
    {
        self.fonts.load(FontArc::new(font), size)
    }

    /// Loads a font for use in the text renderer with sdf rendering.
    ///
    /// There are no requirements on the font, any font can be used for sdf rendering. A font with
    /// SDF enabled can be scaled up without pixellation, and can have effects applied to it.
    /// However, creating the textures for each character will take longer and the textures will
    /// take up more space on the GPU. So if you don't need any of these effects, use
    /// [TextRenderer::load_font] instead.
    pub fn load_font_with_sdf<F>(&mut self, font: F, size: f32, sdf_settings: SdfSettings) -> FontId
    where
        F: Font + Send + Sync + 'static,
    {
        self.fonts
            .load_with_sdf(FontArc::new(font), size, sdf_settings)
    }

    /// Creates a new [Text] object, and creates all gpu buffers needed for it
    pub fn create_text(&self, text: TextData, device: &wgpu::Device, queue: &wgpu::Queue) -> Text {
        Text::new(text, device, queue, self)
    }

    /// Draws a [Text] object to the given render pass.
    pub fn draw_text<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        text: &'pass Text,
    ) {
        let font_data = self.fonts.get(text.data.font);
        match font_data.sdf_settings.as_ref() {
            Some(settings) => self.draw_text_sdf(render_pass, text, font_data, settings),
            None => self.draw_text_no_sdf(render_pass, text, font_data),
        }
    }

    fn draw_text_sdf<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        text: &'pass Text,
        font_data: &FontData,
        sdf_settings: &SdfSettings,
    ) {
        todo!()
    }

    fn draw_text_no_sdf<'pass>(
        &'pass self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        text: &'pass Text,
        font_data: &FontData,
    ) {
        let char_cache = font_data.char_cache.read().unwrap();

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.screen_bind_group, &[]);
        render_pass.set_bind_group(2, &text.settings_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, text.instance_buffer.slice(..));

        // This could be an iterator but it would be like 3 lines longer and impossible to read
        let mut i = 0;
        for c in text.data.text.chars() {
            let char_data = char_cache.get(&c).unwrap();

            if let Some(texture) = &char_data.texture {
                render_pass.set_bind_group(1, texture.bind_group, &[]);
                render_pass.draw(0..4, i as u32..i as u32 + 1);
                i += 1;
            }
        }
    }

    /// Creates an instance buffer for a given piece of text
    fn create_buffer_for_text(&self, text: &TextData, device: &wgpu::Device) -> wgpu::Buffer {
        let mut position = text.position;
        let char_cache = self.fonts.get(text.font).char_cache.read().unwrap();
        let scale = text.options.scale;

        let instances: Vec<CharacterInstance> = text
            .text
            .chars()
            .filter_map(|c| {
                let char_data = char_cache.get(&c).unwrap();

                let result = char_data.texture.as_ref().map(|texture| {
                    let x = position[0] + texture.position[0] * scale;
                    let y = position[1] + texture.position[1] * scale;

                    let w = texture.size[0] * scale;
                    let h = texture.size[1] * scale;

                    CharacterInstance {
                        position: [x, y],
                        size: [w, h],
                    }
                });

                position[0] += char_data.advance * scale;
                result
            })
            .collect_vec();

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaku text instance buffer"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        instance_buffer
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
    pub fn update_char_textures(
        &self,
        text: &str,
        font: FontId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let now = std::time::Instant::now();
        let font_data = self.fonts.get(font);
        let mut char_cache = font_data.char_cache.write().unwrap();
        let new_characters = text
            .chars()
            .filter(|c| !char_cache.contains_key(c))
            .unique()
            .collect_vec();

        let font = &font_data.font;
        let scale = font_data.scale;

        let char_data = new_characters
            .into_par_iter()
            .map(|c| (c, self.create_char_texture(c, font, scale, device, queue)));

        char_cache.par_extend(char_data);

        println!("cached in {}us", now.elapsed().as_micros());
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

            // Create the image and write the glyph data to it
            let mut image = image::GrayImage::new(width, height);

            outlined.draw(|x, y, val| image.put_pixel(x, y, image::Luma([(val * 255.) as u8])));

            let texture_size = wgpu::Extent3d {
                width,
                height,
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
                &image,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(width),
                    rows_per_image: Some(height),
                },
                texture_size,
            );

            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Nearest,
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

            CharTexture {
                // TODO: Get rid of this
                // this is a hack to get past the render pass lifetime restriction until the wgpu
                // update that will get rid of it
                //
                // For now, this is all I need since I will never be deleting textures from the
                // cache and will never be dropping the texture cache (in the game this crate was
                // originally made for)
                //
                // but, it will have to be removed eventually
                bind_group: Box::leak(Box::new(bind_group)),
                size: [width as f32, height as f32],
                position: [x, y],
            }
        });

        Character { texture, advance }
    }
}

#[cfg(test)]
mod tests {}
