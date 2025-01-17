//! Types for creating and configuring drawable text objects.
//!
//! The main type here is [Text], which can be created using [TextRenderer::create_text]. This is a
//! piece of text which can be drawn to the screen with a variety of effects.

use ab_glyph::{Font, PxScale};
use wgpu::util::DeviceExt;

use crate::{FontId, TextRenderer};

/// Options for a text outline.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub(crate) struct Outline {
    pub(crate) color: [f32; 4],
    pub(crate) width: f32,
}

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub(crate) struct SdfTextData {
    pub(crate) radius: f32,
    pub(crate) outline: Option<Outline>,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) struct TextData {
    pub(crate) text: String,
    pub(crate) font: FontId,
    pub(crate) position: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) scale: f32,
    pub(crate) halign: HorizontalAlignment,
    pub(crate) valign: VerticalAlignment,

    pub(crate) sdf: Option<SdfTextData>,
}

impl TextData {
    fn settings_uniform(&self) -> SettingsUniform {
        SettingsUniform {
            color: self.color,
            text_position: self.position,
            _padding: [0.; 2],
        }
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
            text_position: self.position,
            outline_width,
            sdf_radius,
            image_scale: self.scale,
            _padding: [0.; 3],
        }
    }
}

/// Settings for font size.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub enum FontSize {
    /// A font's size in pt.
    Pt(f32),
    /// A font's size in px.
    Px(f32),
}

impl FontSize {
    pub(crate) fn scale(&self, font: &impl Font) -> PxScale {
        match self {
            FontSize::Px(px) => font.pt_to_px_scale(*px * (72. / 96.)).unwrap(),
            FontSize::Pt(pt) => font.pt_to_px_scale(*pt).unwrap(),
        }
    }

    pub(crate) fn px_size(&self, font: &impl Font) -> f32 {
        self.scale(font).y
    }
}

/// Settings for horizontal text alignment
///
/// These control where the text drawn is with respect to its position
#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd)]
pub enum HorizontalAlignment {
    /// Anchors the position at the left side of the text.
    ///
    /// Text is drawn starting at the render position.
    #[default]
    Left,
    /// Anchors the position to the middle of the text.
    Center,
    /// Anchors the position at the right side of the text.
    ///
    /// Text is drawn ending at the render position.
    Right,
    /// Anchors the text position at some point between the start and end of the text.
    ///
    /// A value of 0 is Left alignment, a value of 1 is Right alignment, and values in between
    /// shift between the two continuously (e.g., a value of 0.5 is Center alignment).
    ///
    /// Values outside the range of 0-1 will be clamped within it.
    Ratio(f32),
}

impl HorizontalAlignment {
    /// The proportion of the alignment.
    ///
    /// This ranges from 0-1, where 0 is Left alignment and 1 is Right alignment.
    pub fn proportion(&self) -> f32 {
        match self {
            Self::Left => 0.,
            Self::Right => 1.,
            Self::Center => 0.5,
            Self::Ratio(r) => r.clamp(0., 1.),
        }
    }
}

/// Settings for vertical text alignment.
///
/// See <https://freetype.org/freetype2/docs/glyphs/glyphs-3.html> for more info on font metrics.
#[derive(Default, Copy, Clone, Debug, PartialEq, PartialOrd)]
pub enum VerticalAlignment {
    /// Anchors the position to the baseline of the text.
    ///
    /// In the roman alphabet, the baseline is usually at the bottom of characters such as a, b, c,
    /// etc. Characters like g or j usually go below the baseline.
    #[default]
    Baseline,
    /// Anchors the position to the highest point of the font.
    ///
    /// This means characters will never rise above the render position.
    Top,
    /// Anchors the position to be exactly halfway between the highest and lowest points of the
    /// font.
    Middle,
    /// Anchors the position to the lowest point of the font.
    ///
    /// This means characters will never go below the render position
    Bottom,
    /// Anchors the position at some point between the highest and lowest points of the font.
    ///
    /// A value of 0 is Bottom alignment, a value of 1 is Top alignment, and values in between
    /// shift between the two continuously (e.g., a value of 0.5 is Middle alignment).
    ///
    /// Values outside the range of 0-1 will be clamped within it.
    Ratio(f32),
}

/// A builder for a [Text] struct.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct TextBuilder {
    text: String,
    font: FontId,
    position: [f32; 2],
    outline: Option<Outline>,
    color: [f32; 4],
    scale: f32,
    custom_font_size: Option<FontSize>,
    halign: HorizontalAlignment,
    valign: VerticalAlignment,
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
            custom_font_size: None,
            halign: Default::default(),
            valign: Default::default(),
        }
    }

    /// Creates a new Text object from the current configuration and uploads any necessary data
    /// to the GPU.
    pub fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text_renderer: &mut TextRenderer,
    ) -> Text {
        let scale = match self.custom_font_size {
            None => self.scale,
            Some(size) => {
                let self_size = size.px_size(&text_renderer.fonts.get(self.font).font);
                let font_size = text_renderer.fonts.get(self.font).px_size;

                self.scale * (self_size / font_size)
            }
        };

        let data = TextData {
            text: self.text.clone(),
            font: self.font,
            position: self.position,
            color: self.color,
            scale,
            halign: self.halign,
            valign: self.valign,

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

    /// Sets the horizontal alignment of the text.
    ///
    /// See [HorizontalAlignment] for details.
    pub fn horizontal_align(&mut self, halign: HorizontalAlignment) -> &mut Self {
        self.halign = halign;
        self
    }

    /// Sets the vertical alignment of the text.
    ///
    /// See [VerticalAlignment] for details.
    pub fn vertical_align(&mut self, valign: VerticalAlignment) -> &mut Self {
        self.valign = valign;
        self
    }

    /// Adds an outline to the text, with given colour and width. If the width is less than or
    /// equal to zero, this turns off the outline.
    ///
    /// Text can only be outlined if it is drawn using sdf, so if the font is not sdf-enabled then
    /// this won't do anything. The outline can only be as wide as the sdf radius of the font. If
    /// you want a wider outline, use a wider radius (see [crate::SdfSettings]).
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

    /// Sets the colour of the text, in RGBA (values are in the range 0-1). The default is solid
    /// black.
    pub fn color(&mut self, color: [f32; 4]) -> &mut Self {
        self.color = color;
        self
    }

    /// Sets the scale of the text. The default is 1.0.
    ///
    /// If the font is not sdf-enabled, it will be scaled up bilinearly, and you may get
    /// pixellation/bluriness. If it is sdf-enabled, it will be cleaner but you may still get
    /// artefacts at high scale.
    pub fn scale(&mut self, scale: f32) -> &mut Self {
        self.scale = scale;
        self
    }

    /// Adjusts the text scale so that it is drawn at a certain font size. If the argument is None,
    /// it resets the text to the default size of the font (the size it was loaded into the text
    /// renderer with).
    ///
    /// If the font is not SDF-enabled, then upscaling will be done with bilinear filtering,
    /// and will not look very good.
    ///
    /// Note that this is multiplicative with the scale option; e.g. if the font size is set to be
    /// 40pt and the scale is set to 2.0, then the font will be drawn at 80pt size.
    pub fn font_size(&mut self, size: Option<FontSize>) -> &mut Self {
        self.custom_font_size = size;
        self
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SettingsUniform {
    color: [f32; 4],
    text_position: [f32; 2],
    _padding: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SdfSettingsUniform {
    color: [f32; 4],
    outline_color: [f32; 4],
    text_position: [f32; 2],
    outline_width: f32,
    sdf_radius: f32,
    image_scale: f32,
    _padding: [f32; 3],
}

/// A piece of text that can be rendered to the screen.
///
/// Create one of these using a [TextBuilder], then render it to a wgpu render pass using
/// [TextRenderer::draw_text].
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
        text_renderer: &mut TextRenderer,
    ) -> Self {
        text_renderer.generate_char_textures(data.text.chars(), data.font, device, queue);
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
        text_renderer: &mut TextRenderer,
    ) {
        text_renderer.generate_char_textures(text.chars(), self.data.font, device, queue);
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

    /// Changes the position of the text on the screen.
    pub fn set_position(&mut self, position: [f32; 2], queue: &wgpu::Queue) {
        self.data.position = position;
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
