//! Types for creating and configuring drawable text objects.
//!
//! The main type here is [Text], which can be created using [TextRenderer::create_text]. This is a
//! piece of text which can be drawn to the screen with a variety of effects.

use wgpu::util::DeviceExt;

use crate::{FontId, FontMap, TextRenderer};

/// Options for a text outline.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Outline {
    pub(crate) color: [f32; 4],
    pub(crate) width: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct TextOptions {
    pub(crate) text: String,
    pub(crate) font: FontId,
    pub(crate) position: [f32; 2],
    pub(crate) outline: Option<Outline>,
    pub(crate) color: [f32; 4],
    pub(crate) scale: f32,
}

impl TextOptions {
    fn settings_uniform(&self) -> SettingsUniform {
        SettingsUniform {
            color: self.color,
        }
    }

    fn sdf_settings_uniform(&self, font_map: &FontMap) -> SdfSettingsUniform {
        let outline_color = self.outline.map(|o| o.color).unwrap_or([0.; 4]);
        let outline_width = self.outline.map(|o| o.width).unwrap_or(0.);
        let sdf_radius = font_map.get(self.font).sdf_settings.expect("Sdf settings don't exist").radius;

        SdfSettingsUniform {
            color: self.color,
            outline_color,
            outline_width,
            sdf_radius,
            image_scale: self.scale,
            _padding: 0.,
        }
    }
}

/// A builder for a [Text] struct.
#[derive(Debug, Clone)]
pub struct TextBuilder {
    options: TextOptions,
}

impl TextBuilder {
    /// Creates a new TextBuilder.
    pub fn new(text: impl Into<String>, font: FontId, position: [f32; 2]) -> Self {
        Self {
            options: TextOptions {
                text: text.into(),
                font,
                position,

                outline: None,
                color: [0., 0., 0., 1.],
                scale: 1.,
            },
        }
    }

    /// Creates a new [Text] object from the current configuration and uploads any necessary data
    /// to the GPU.
    pub fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_renderer: &TextRenderer,
    ) -> Text {
        Text::new(self.options.clone(), device, queue, text_renderer)
    }

    /// Sets the content of the text.
    pub fn text(&mut self, text: String) -> &mut Self {
        self.options.text = text;
        self
    }

    /// Sets the font the text will be drawn with.
    pub fn font(&mut self, font: FontId) -> &mut Self {
        self.options.font = font;
        self
    }

    /// Sets the position of the text on the screen, in pixel coordinates.
    pub fn position(&mut self, position: [f32; 2]) -> &mut Self {
        self.options.position = position;
        self
    }

    /// Adds an outline to the text, with given colour and width.
    ///
    /// If the width is less than or equal to zero, this just turns off the outline (same as
    /// [TextBuilder::no_outline]).
    ///
    /// Text can only be outlined if it is drawn using sdf, so if the font is not sdf-enabled then
    /// this won't do anything.
    ///
    /// The outline can only be as wide as the sdf radius of the font. If you want a wider outline,
    /// use a wider radius (see [crate::SdfSettings]).
    pub fn outlined(&mut self, color: [f32; 4], width: f32) -> &mut Self {
        if width > 0. {
            self.options.outline = Some(Outline { color, width });
        } else {
            self.options.outline = None;
        }

        self
    }

    /// Sets this text to have no outline.
    ///
    /// Text will not be outlined by default, so only use this if you've already set the outline
    /// and want to get rid of it e.g. when building another text object.
    pub fn no_outline(&mut self) -> &mut Self {
        self.options.outline = None;
        self
    }

    /// Sets the colour of the text, in RGBA (values are in the range 0-1).
    pub fn color(&mut self, color: [f32; 4]) -> &mut Self {
        self.options.color = color;
        self
    }

    /// Sets the scale of the text.
    ///
    /// If the font is not sdf-enabled, it will be scaled up bilinearly, and you may get
    /// pixellation/bluriness. If it is sdf-enabled, it will be cleaner but you may still get
    /// artefacts at high scale.
    pub fn scale(&mut self, scale: f32) -> &mut Self {
        self.options.scale = scale;
        self
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SettingsUniform {
    color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SdfSettingsUniform {
    pub(crate) color: [f32; 4],
    pub(crate) outline_color: [f32; 4],
    pub(crate) outline_width: f32,
    pub(crate) sdf_radius: f32,
    pub(crate) image_scale: f32,
    _padding: f32,
}

/// A piece of text that can be rendered to the screen. Create one of these using a [TextBuilder],
/// then render it to a wgpu render pass using [TextRenderer::draw_text].
#[derive(Debug)]
pub struct Text {
    pub(crate) options: TextOptions,
    pub(crate) instance_buffer: wgpu::Buffer,
    pub(crate) settings_bind_group: wgpu::BindGroup,

    instance_capacity: usize,
}

impl Text {
    /// Creates a new [Text] object and uploads all necessary data to the GPU.
    fn new(
        options: TextOptions,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_renderer: &TextRenderer,
    ) -> Self {
        text_renderer.update_char_textures(&options.text, options.font, device, queue);
        let instances = text_renderer.create_text_instances(&options);

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaku text instance buffer"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let settings_bind_group = if text_renderer.font_uses_sdf(options.font) {
            let text_settings = options.sdf_settings_uniform(&text_renderer.fonts);
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
            let text_settings = options.settings_uniform();

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
            options,
            instance_buffer,
            settings_bind_group,
            instance_capacity: instances.len(),
        }
    }

    /// Changes the text displayed by this text object.
    ///
    /// This is faster than recreating the object because it may reuse its existing gpu buffer
    /// instead of recreating it.
    pub fn change_text(
        &mut self,
        text: String,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_renderer: &TextRenderer,
    ) {
        text_renderer.update_char_textures(&text, self.options.font, device, queue);
        self.options.text = text;
        let new_instances = text_renderer.create_text_instances(&self.options);

        if new_instances.len() > self.instance_capacity {
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kaku text instance buffer"),
                contents: bytemuck::cast_slice(&new_instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });

            self.instance_capacity = new_instances.len();
        } else {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&new_instances),
            );
        }
    }
}
