//! Types for creating and configuring drawable text objects.
//!
//! The main type here is [Text], which can be created using [TextRenderer::create_text]. This is a
//! piece of text which can be drawn to the screen with a variety of effects.

use wgpu::util::DeviceExt;

use crate::{FontId, FontMap, TextRenderer};

/// Data describing a text object.
#[derive(Debug, Clone)]
pub struct TextData {
    /// The actual string to render
    pub text: String,
    /// The position of the text on the screen.
    /// This will be the left-most point of the text, at the baseline.
    pub position: [f32; 2],
    /// The font to use when rendering
    pub font: FontId,

    /// Extra options that have default values (so can be created using the [Default] trait).
    /// See [TextOptions]
    pub options: TextOptions,
}

impl TextData {
    /// Creates a new [TextData] struct.
    pub fn new<T: Into<String>>(
        text: T,
        position: [f32; 2],
        font: FontId,
        options: TextOptions,
    ) -> Self {
        Self {
            text: text.into(),
            position,
            font,
            options,
        }
    }

    fn settings_uniform(&self) -> SettingsUniform {
        SettingsUniform {
            colour: self.options.colour,
        }
    }

    fn sdf_settings_uniform(&self, fonts: &FontMap) -> SdfSettingsUniform {
        let sdf_radius = fonts
            .get(self.font)
            .sdf_settings
            .map(|s| s.radius)
            .unwrap_or_default();

        SdfSettingsUniform {
            colour: self.options.colour,
            outline_colour: self.options.outline.map(|o| o.colour).unwrap_or([0.0; 4]),
            outline_radius: self.options.outline.map(|o| o.width).unwrap_or(0.),
            sdf_radius,
            image_scale: self.options.scale,
            _padding: 0.,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SettingsUniform {
    colour: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SdfSettingsUniform {
    pub(crate) colour: [f32; 4],
    pub(crate) outline_colour: [f32; 4],
    pub(crate) outline_radius: f32,
    pub(crate) sdf_radius: f32,
    pub(crate) image_scale: f32,
    _padding: f32,
}

/// A piece of text that can be rendered to the screen.
#[derive(Debug)]
pub struct Text {
    pub(crate) data: TextData,
    pub(crate) instance_buffer: wgpu::Buffer,
    pub(crate) settings_bind_group: wgpu::BindGroup,

    instance_capacity: usize,
}

impl Text {
    /// Creates a new [Text] object and uploads all necessary data to the GPU.
    /// See also [TextRenderer::create_text] for a convenient wrapper to this function
    pub fn new(
        data: TextData,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_renderer: &TextRenderer,
    ) -> Self {
        text_renderer.update_char_textures(&data.text, data.font, device, queue);
        let instances = text_renderer.create_text_instances(&data);

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaku text instance buffer"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let settings_bind_group = if text_renderer.font_uses_sdf(data.font) {
            let text_settings = data.sdf_settings_uniform(&text_renderer.fonts);
            let settings_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kaku sdf text settings uniform buffer"),
                contents: bytemuck::cast_slice(&[text_settings]),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            });

            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kaku sdf text settings uniform bind group"),
                layout: &text_renderer.sdf_settings_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings_buffer.as_entire_binding(),
                }],
            })
        } else {
            let text_settings = data.settings_uniform();

            let settings_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kaku text settings uniform buffer"),
                contents: bytemuck::cast_slice(&[text_settings]),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            });

            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kaku text settings uniform bind group"),
                layout: &text_renderer.settings_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings_buffer.as_entire_binding(),
                }],
            })
        };

        Self {
            data,
            instance_buffer,
            settings_bind_group,
            instance_capacity: instances.len(),
        }
    }

    /// Changes the text displayed by this text object.
    /// 
    /// This is faster than recreating the object because it may reuse its existing gpu buffer
    /// instead of recreating it.
    pub fn change_text(&mut self, text: String, device: &wgpu::Device, queue: &wgpu::Queue, text_renderer: &TextRenderer) {
        text_renderer.update_char_textures(&text, self.data.font, device, queue);
        self.data.text = text;
        let new_instances = text_renderer.create_text_instances(&self.data);

        if new_instances.len() > self.instance_capacity {
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kaku text instance buffer"),
                contents: bytemuck::cast_slice(&new_instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });

            self.instance_capacity = new_instances.len();
        } else {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&new_instances));
        }
    }
}

/// Settings for a text outline.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Outline {
    /// The colour of the outline.
    pub colour: [f32; 4],
    /// The width of the outline.
    ///
    /// This does not change how far letters are spaced apart from each other. It simply controls
    /// how thick the outline should be around the letter.
    pub width: f32,
}

/// Options for how to draw the text that have default values (so this struct can implement
/// [Default]).
#[derive(Copy, Clone, Debug)]
pub struct TextOptions {
    /// The colour of the text in RGBA (values going from 0.0 to 1.0)
    pub colour: [f32; 4],
    /// Details about the outline of the text.
    ///
    /// If this is None, the text will not be outlined.
    /// If this is Some, the text will be outlined with colour and width specified by the [Outline]
    /// struct.
    pub outline: Option<Outline>,
    /// The scale at which to draw the text.
    /// If it is 1.0, text will be drawn at the default size for the font.
    pub scale: f32,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self {
            colour: [0., 0., 0., 1.],
            outline: None,
            scale: 1.0,
        }
    }
}
