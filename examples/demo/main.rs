mod wgpu_renderer;
use std::{sync::Arc, time::Instant};

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

use kaku::{
    text::{Outline, Text, TextData, TextOptions},
    FontId, SdfSettings, TextRenderer,
};

const FPS_POLL_TIME_LIMIT: f32 = 0.5;

struct BasicTextAppInner {
    renderer: Renderer,
    text_renderer: TextRenderer,
    hello_world: Text,
    hello_world_sdf: Text,
    hello_world_outline: Text,
    hello_world_scaled: Text,
    fira_sans_sdf: FontId,
    fps_text: Text,
    frame_count: f32,
    fps_poll_start: Instant,
}

#[derive(Default)]
struct BasicTextApp {
    inner: Option<BasicTextAppInner>,
}

impl BasicTextAppInner {
    fn new(window: Arc<Window>) -> Self {
        let renderer = Renderer::new(window);
        let mut text_renderer = TextRenderer::new(&renderer.device, &renderer.config);
        let fira_sans = FontArc::new(
            FontRef::try_from_slice(include_bytes!("../fonts/FiraSans-Regular.ttf")).unwrap(),
        );
        let fira_sans_sdf =
            text_renderer.load_font_with_sdf(fira_sans.clone(), 60., SdfSettings { radius: 20.0 });
        let fira_sans = text_renderer.load_font(fira_sans, 60.);

        let hello_world = text_renderer.create_text(
            TextData::new(
                "hello, world! glyph :3",
                [50., 120.],
                fira_sans,
                Default::default(),
            ),
            &renderer.device,
            &renderer.queue,
        );

        let hello_world_sdf = text_renderer.create_text(
            TextData::new(
                "hello, world! glyph :3",
                [50., 220.],
                fira_sans_sdf,
                Default::default(),
            ),
            &renderer.device,
            &renderer.queue,
        );

        let hello_world_outline = text_renderer.create_text(
            TextData::new(
                "hello, world! glyph :3",
                [50., 320.],
                fira_sans_sdf,
                TextOptions {
                    colour: [1.; 4],
                    outline: Some(kaku::text::Outline {
                        colour: [1., 0., 0., 1.],
                        width: 15.,
                    }),
                    ..Default::default()
                },
            ),
            &renderer.device,
            &renderer.queue,
        );

        let hello_world_scaled = text_renderer.create_text(
            TextData::new(
                "hello, world! glyph :3",
                [50., 520.],
                fira_sans_sdf,
                TextOptions {
                    scale: 2.0,
                    ..Default::default()
                },
            ),
            &renderer.device,
            &renderer.queue,
        );

        let fps_text = text_renderer.create_text(
            TextData::new(
                "fps: ",
                [40., 40.],
                fira_sans_sdf,
                TextOptions {
                    colour: [1., 0., 1., 1.],
                    scale: 0.3,
                    outline: Some(Outline {
                        colour: [0., 0., 0., 1.],
                        width: 2.,
                    }),
                },
            ),
            &renderer.device,
            &renderer.queue,
        );

        Self {
            text_renderer,
            renderer,
            hello_world,
            hello_world_sdf,
            hello_world_outline,
            hello_world_scaled,
            fira_sans_sdf,
            fps_text,
            fps_poll_start: Instant::now(),
            frame_count: 0.,
        }
    }

    fn update(&mut self) {
        self.frame_count += 1.;
        let elapsed = self.fps_poll_start.elapsed().as_secs_f32();

        if elapsed > FPS_POLL_TIME_LIMIT {
            let fps = self.frame_count / elapsed;

            self.fps_text = self.text_renderer.create_text(
                TextData::new(
                    format!("fps: {fps:.2}"),
                    [40., 40.],
                    self.fira_sans_sdf,
                    TextOptions {
                        colour: [1., 0., 1., 1.],
                        scale: 0.3,
                        outline: Some(Outline {
                            colour: [0., 0., 0., 1.],
                            width: 3.,
                        }),
                    },
                ),
                &self.renderer.device,
                &self.renderer.queue,
            );

            self.frame_count = 0.;
            self.fps_poll_start = Instant::now();
        }
    }

    fn render(&mut self) -> Result<(), SurfaceError> {
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

        // Important code is here!
        self.text_renderer
            .draw_text(&mut render_pass, &self.fps_text);
        self.text_renderer
            .draw_text(&mut render_pass, &self.hello_world);
        self.text_renderer
            .draw_text(&mut render_pass, &self.hello_world_sdf);
        self.text_renderer
            .draw_text(&mut render_pass, &self.hello_world_outline);
        self.text_renderer
            .draw_text(&mut render_pass, &self.hello_world_scaled);

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
                .with_inner_size(PhysicalSize::new(1400, 600));

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

        inner.update();

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
