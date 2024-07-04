//! Types for creating and configuring drawable text objects.
//!
//! The main type here is [Text], which can be created using [TextRenderer::create_text]. This is a
//! piece of text which can be drawn to the screen with a variety of effects.

use wgpu::util::DeviceExt;

use crate::{FontId, TextRenderer};

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
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SettingsUniform {
    colour: [f32; 4],
}

/// A piece of text that can be rendered to the screen.
#[derive(Debug)]
pub struct Text {
    pub(crate) data: TextData,
    pub(crate) instance_buffer: wgpu::Buffer,
    pub(crate) settings_bind_group: wgpu::BindGroup,
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
        let instance_buffer = text_renderer.create_buffer_for_text(&data, device);
        let text_settings = data.settings_uniform();
        let settings_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaku text settings uniform buffer"),
            contents: bytemuck::cast_slice(&[text_settings]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let settings_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kaku text settings uniform bind group"),
            layout: &text_renderer.settings_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: settings_buffer.as_entire_binding(),
            }],
        });

        Self {
            data,
            instance_buffer,
            settings_bind_group,
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
