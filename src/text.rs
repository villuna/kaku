//! Types for creating and configuring drawable text objects.
//!
//! The main type here is [Text], which can be created using [TextRenderer::create_text]. This is a
//! piece of text which can be drawn to the screen with a variety of effects.

use wgpu::util::DeviceExt;

use crate::{FontId, TextRenderer};

/// Options for a text outline.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Outline {
    pub(crate) color: [f32; 4],
    pub(crate) width: f32,
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct SdfTextData {
    pub(crate) radius: f32,
    pub(crate) outline: Option<Outline>,
}

#[derive(Debug, Clone)]
pub(crate) struct TextData {
    pub(crate) text: String,
    pub(crate) font: FontId,
    pub(crate) position: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) scale: f32,

    pub(crate) sdf: Option<SdfTextData>,
}

impl TextData {
    fn settings_uniform(&self) -> SettingsUniform {
        SettingsUniform { color: self.color }
    }

    fn sdf_settings_uniform(&self) -> SdfSettingsUniform {
        let sdf = &self
            .sdf
            .expect("sdf_settings_uniform called but no sdf data found");
        let outline_color = sdf.outline.map(|o| o.color).unwrap_or([0.; 4]);
        let outline_width = sdf.outline.map(|o| o.width).unwrap_or(0.);
        let sdf_radius = sdf.radius;

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
    pub(crate) text: String,
    pub(crate) font: FontId,
    pub(crate) position: [f32; 2],
    pub(crate) outline: Option<Outline>,
    pub(crate) color: [f32; 4],
    pub(crate) scale: f32,
}

impl TextBuilder {
    /// Creates a new TextBuilder.
    pub fn new(text: impl Into<String>, font: FontId, position: [f32; 2]) -> Self {
        Self {
            text: text.into(),
            font,
            position,

            outline: None,
            color: [0., 0., 0., 1.],
            scale: 1.,
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
        let data = TextData {
            text: self.text.clone(),
            font: self.font,
            position: self.position,
            color: self.color,
            scale: self.scale,

            sdf: text_renderer.font_uses_sdf(self.font).then(|| SdfTextData {
                radius: text_renderer
                    .fonts
                    .get(self.font)
                    .sdf_settings
                    .unwrap()
                    .radius,
                outline: self.outline,
            }),
        };
        Text::new(data, device, queue, text_renderer)
    }

    /// Sets the content of the text.
    pub fn text(&mut self, text: String) -> &mut Self {
        self.text = text;
        self
    }

    /// Sets the font the text will be drawn with.
    pub fn font(&mut self, font: FontId) -> &mut Self {
        self.font = font;
        self
    }

    /// Sets the position of the text on the screen, in pixel coordinates.
    pub fn position(&mut self, position: [f32; 2]) -> &mut Self {
        self.position = position;
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
            self.outline = Some(Outline { color, width });
        } else {
            self.outline = None;
        }

        self
    }

    /// Sets this text to have no outline.
    ///
    /// Text will not be outlined by default, so only use this if you've already set the outline
    /// and want to get rid of it e.g. when building another text object.
    pub fn no_outline(&mut self) -> &mut Self {
        self.outline = None;
        self
    }

    /// Sets the colour of the text, in RGBA (values are in the range 0-1).
    pub fn color(&mut self, color: [f32; 4]) -> &mut Self {
        self.color = color;
        self
    }

    /// Sets the scale of the text.
    ///
    /// If the font is not sdf-enabled, it will be scaled up bilinearly, and you may get
    /// pixellation/bluriness. If it is sdf-enabled, it will be cleaner but you may still get
    /// artefacts at high scale.
    pub fn scale(&mut self, scale: f32) -> &mut Self {
        self.scale = scale;
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
    pub(crate) data: TextData,
    pub(crate) instance_buffer: wgpu::Buffer,
    pub(crate) settings_bind_group: wgpu::BindGroup,

    settings_buffer: wgpu::Buffer,
    instance_capacity: usize,
}

impl Text {
    /// Creates a new [Text] object and uploads all necessary data to the GPU.
    fn new(
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

        let (settings_buffer, settings_bind_group) = if text_renderer.font_uses_sdf(data.font) {
            let text_settings = data.sdf_settings_uniform();
            let settings_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kaku sdf text settings uniform buffer"),
                contents: bytemuck::cast_slice(&[text_settings]),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            });

            let settings_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kaku sdf text settings uniform bind group"),
                layout: &text_renderer.sdf_settings_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings_buffer.as_entire_binding(),
                }],
            });

            (settings_buffer, settings_bind_group)
        } else {
            let text_settings = data.settings_uniform();

            let settings_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kaku text settings uniform buffer"),
                contents: bytemuck::cast_slice(&[text_settings]),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            });

            let settings_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kaku text settings uniform bind group"),
                layout: &text_renderer.settings_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings_buffer.as_entire_binding(),
                }],
            });

            (settings_buffer, settings_bind_group)
        };

        Self {
            data,
            instance_buffer,
            settings_bind_group,
            settings_buffer,
            instance_capacity: instances.len(),
        }
    }

    /// Changes the text displayed by this text object.
    ///
    /// This is faster than recreating the object because it may reuse its existing gpu buffer
    /// instead of recreating it.
    pub fn set_text(
        &mut self,
        text: String,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_renderer: &TextRenderer,
    ) {
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
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&new_instances),
            );
        }
    }

    // Uploads the current settings (as described in self.data) to the settings buffer on the GPU.
    fn update_settings_buffer(&self, queue: &wgpu::Queue) {
        if self.data.sdf.is_some() {
            queue.write_buffer(
                &self.settings_buffer,
                0,
                bytemuck::cast_slice(&[self.data.sdf_settings_uniform()]),
            );
        } else {
            queue.write_buffer(
                &self.settings_buffer,
                0,
                bytemuck::cast_slice(&[self.data.settings_uniform()]),
            );
        }
    }

    /// Changes the color of the text.
    pub fn set_color(&mut self, color: [f32; 4], queue: &wgpu::Queue) {
        self.data.color = color;
        self.update_settings_buffer(queue);
    }

    /// Changes the scale of the text.
    pub fn set_scale(&mut self, scale: f32, queue: &wgpu::Queue) {
        self.data.scale = scale;
        self.update_settings_buffer(queue);
    }

    /// Sets the outline to be on with the given options. If the width is less than or equal to zero, it turns
    /// the outline off.
    ///
    /// This does nothing if the font is not rendered with sdf.
    pub fn set_outline(&mut self, color: [f32; 4], width: f32, queue: &wgpu::Queue) {
        if let Some(sdf) = &mut self.data.sdf {
            if width > 0. {
                sdf.outline = Some(Outline { color, width });
            } else {
                sdf.outline = None;
            }
        }

        self.update_settings_buffer(queue);
    }

    /// Removes the outline from the text, if there was one.
    ///
    /// This does nothing if the font is not rendered with sdf.
    pub fn set_no_outline(&mut self, queue: &wgpu::Queue) {
        if let Some(sdf) = &mut self.data.sdf {
            sdf.outline = None;
        }

        self.update_settings_buffer(queue)
    }
}
