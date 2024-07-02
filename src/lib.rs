#![warn(missing_docs)]
//! A font rendering crate for rendering text using signed distance fields.
//!
//! This crate was designed for a video game where I needed a lot of quick text rendering with
//! outlines, so this is the main aim of this crate.
//!
//! A lot of the functionality of this crate was taken from
//! https://learnopengl.com/In-Practice/Text-Rendering.
//!
//! Also, I used the learn wpgu tutorial for reference for the wpgu code.

use std::{cell::RefCell, num::NonZeroU64};

pub use ab_glyph;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use ahash::AHashMap;
use itertools::Itertools;
use log::info;
use text::SettingsUniform;
use wgpu::{include_wgsl, util::DeviceExt, TextureViewDescriptor};

pub use text::{Text, TextData, TextOptions};

mod sdf;
mod text;

type HashMap<K, V> = AHashMap<K, V>;

#[derive(Debug)]
struct CharTexture {
    view: wgpu::TextureView,
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

#[derive(Debug, Clone, Copy)]
pub struct SdfSettings {
    radius: f32,
}

struct FontData {
    font: FontArc,
    scale: PxScale,
    char_cache: RefCell<CharacterCache>,
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
        self.fonts.push(FontData::new_with_sdf(font, scale, sdf_settings));
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

/// A matrix that turns pixel coordinates into wgpu screen coordinates.
fn create_screen_matrix(size: (u32, u32)) -> ScreenUniform {
    let width = size.0 as f32;
    let height = size.1 as f32;
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
    screen_uniform: ScreenUniform,

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

        let screen_uniform = create_screen_matrix((target_config.width, target_config.height));

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
            screen_uniform,
            vertex_buffer,
        }
    }

    /// Loads a font for use in the text renderer
    pub fn load_font<F: Font>(&mut self, font: F, size: f32) -> FontId
    where
        F: Font + Send + Sync + 'static,
    {
        self.fonts.load(FontArc::new(font), size)
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
        let char_cache = self.fonts.get(text.data.font).char_cache.borrow();

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
                render_pass.set_bind_group(1, &texture.bind_group, &[]);
                render_pass.draw(0..4, i as u32..i as u32 + 1);
                i += 1;
            }
        }
    }

    /// Creates and caches the character textures necessary to draw a certain string with a given
    /// font.
    fn update_char_textures(
        &self,
        text: &str,
        font: FontId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let cache = self.fonts.get(font);
        let new_characters = text
            .chars()
            .filter(|c| !cache.char_cache.borrow().contains_key(c))
            .unique()
            .collect_vec();

        for c in new_characters {
            self.create_char_texture(c, font, device, queue);
        }
    }

    /// Creates an instance buffer for a given piece of text
    fn create_buffer_for_text(&self, text: &TextData, device: &wgpu::Device) -> wgpu::Buffer {
        let mut position = text.position;
        let char_cache = self.fonts.get(text.font).char_cache.borrow();
        let scale = text.options.scale;

        let instances: Vec<CharacterInstance> = text
            .text
            .chars()
            .filter_map(|c| {
                let char_data = char_cache.get(&c).unwrap();

                let result = char_data.texture.as_ref().map(|texture| {
                    let x = position[0] + texture.position[0] * scale;
                    let y = position[1] + texture.position[1] * scale;

                    let w = texture.size[0] as f32 * scale;
                    let h = texture.size[1] as f32 * scale;

                    let res = CharacterInstance {
                        position: [x, y],
                        size: [w, h],
                    };

                    res
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

    fn create_char_texture(
        &self,
        c: char,
        font_id: FontId,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        info!("Creating character texture for {c}");
        let font = self.fonts.get(font_id);
        // Calculate metrics
        let scaled = font.font.as_scaled(font.scale);
        let glyph = font.font.glyph_id(c).with_scale(font.scale);

        let bearing = [
            scaled.h_side_bearing(glyph.id),
            scaled.v_side_bearing(glyph.id),
        ];
        let advance = scaled.h_advance(glyph.id);

        let texture = scaled.outline_glyph(glyph).map(|outlined| {
            // TODO: This is a rect, not just a width and height. use this to draw the pixels at the right positions.
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
                view,
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

        let char_data = Character { texture, advance };

        self.fonts
            .get(font_id)
            .char_cache
            .borrow_mut()
            .insert(c, char_data);
    }
}

#[cfg(test)]
mod tests {}
