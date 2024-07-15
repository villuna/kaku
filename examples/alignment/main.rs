//! Alignment - Shows the difference between horizontal and vertical alignments
//!
//! Since this example has to be integrated into wgpu and winit, the code is pretty verbose. I've
//! commented the code that's important to this crate, so you don't have to sift through all the
//! boilerplate.
mod wgpu_renderer;
use std::sync::Arc;

use ab_glyph::{FontArc, FontRef};
use wgpu::SurfaceError;
use wgpu_renderer::Renderer;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    error::EventLoopError,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;

use kaku::{HorizontalAlignment, Text, TextBuilder, TextRenderer, TextRendererBuilder, VerticalAlignment};

struct BasicTextAppInner {
    renderer: Renderer,
    text_renderer: TextRenderer,

    valign_baseline: Text,
    valign_top: Text,
    valign_middle: Text,
    valign_bottom: Text,

    halign_left: Text,
    halign_center: Text,
    halign_right: Text,
}

#[derive(Default)]
struct BasicTextApp {
    inner: Option<BasicTextAppInner>,
}

impl BasicTextAppInner {
    // -- IMPORTANT CODE IS IN THIS IMPL BLOCK --

    fn new(window: Arc<Window>) -> Self {
        let renderer = Renderer::new(window);

        let format = renderer.config.format;
        let size = (renderer.config.width, renderer.config.height);
        let mut text_renderer = TextRendererBuilder::new(format, size).build(&renderer.device);

        let fira_sans = FontArc::new(
            FontRef::try_from_slice(include_bytes!("../fonts/FiraSans-Regular.ttf")).unwrap(),
        );
        let fira_sans = text_renderer.load_font(fira_sans, 40.);

        let mut builder = TextBuilder::new("hello!", fira_sans, [50., 100.]);
        builder.vertical_align(VerticalAlignment::Baseline);
        let valign_baseline = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        builder.vertical_align(VerticalAlignment::Top);
        builder.position([230., 100.]);
        let valign_top = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        builder.vertical_align(VerticalAlignment::Middle);
        builder.position([430., 100.]);
        let valign_middle = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        builder.vertical_align(VerticalAlignment::Bottom);
        builder.position([630., 100.]);
        let valign_bottom = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        let mut builder = TextBuilder::new("hello, align!", fira_sans, [WINDOW_WIDTH as f32 / 2., 300.]);
        builder.horizontal_align(HorizontalAlignment::Left);
        let halign_left = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        builder.horizontal_align(HorizontalAlignment::Center);
        builder.position([WINDOW_WIDTH as f32 / 2., 400.]);
        let halign_center = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        builder.horizontal_align(HorizontalAlignment::Right);
        builder.position([WINDOW_WIDTH as f32 / 2., 500.]);
        let halign_right = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        Self {
            text_renderer,
            renderer,
            valign_baseline,
            valign_top,
            valign_middle,
            valign_bottom,
            halign_left,
            halign_center,
            halign_right,
        }
    }

    fn render(&mut self) -> Result<(), SurfaceError> {
        // Here is where we actually render our text!
        //
        // First, set up the render pass...
        let output = self.renderer.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 182. / 255.,
                        g: 162. / 255.,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        self.text_renderer
            .draw_text(&mut render_pass, &self.valign_baseline);
        self.text_renderer
            .draw_text(&mut render_pass, &self.valign_top);
        self.text_renderer
            .draw_text(&mut render_pass, &self.valign_middle);
        self.text_renderer
            .draw_text(&mut render_pass, &self.valign_bottom);

        self.text_renderer
            .draw_text(&mut render_pass, &self.halign_left);
        self.text_renderer
            .draw_text(&mut render_pass, &self.halign_center);
        self.text_renderer
            .draw_text(&mut render_pass, &self.halign_right);

        // And that's it!

        drop(render_pass);

        self.renderer
            .queue
            .submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

impl ApplicationHandler for BasicTextApp {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.inner.is_none() {
            let attributes = Window::default_attributes()
                .with_title("basic text example")
                .with_inner_size(PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));

            let window = event_loop.create_window(attributes).unwrap();
            self.inner = Some(BasicTextAppInner::new(Arc::new(window)));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        if window_id == inner.renderer.window.id() {
            match event {
                WindowEvent::CloseRequested
                | WindowEvent::KeyboardInput {
                    event:
                        KeyEvent {
                            physical_key: PhysicalKey::Code(KeyCode::Escape),
                            state: ElementState::Pressed,
                            ..
                        },
                    ..
                } => {
                    event_loop.exit();
                }

                WindowEvent::Resized(physical_size) => {
                    inner.renderer.resize(physical_size);
                    inner
                        .text_renderer
                        .resize(physical_size.into(), &inner.renderer.queue);
                }

                _ => {}
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };

        match inner.render() {
            Ok(_) => {}
            // Reconfigure the surface if lost
            Err(wgpu::SurfaceError::Lost) => {
                let size = inner.renderer.size;
                inner.renderer.resize(size);
            }
            // The system is out of memory, we should probably quit
            Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
            // All other errors (Outdated, Timeout) should be resolved by the next frame
            Err(e) => eprintln!("{:?}", e),
        }
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = BasicTextApp::default();
    event_loop.run_app(&mut app)
}
