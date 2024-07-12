//! Demo - an example that shows all the things that kaku can do.
//!
//! Since this demo has to be integrated into wgpu and winit, the code is pretty verbose. I've
//! commented the code that's important to this crate, so you don't have to sift through all the
//! boilerplate.
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

const WINDOW_WIDTH: u32 = 1600;
const WINDOW_HEIGHT: u32 = 700;

use kaku::{SdfSettings, Text, TextBuilder, TextRenderer};

fn hsva_to_rgba(mut h: f32, mut s: f32, mut v: f32, a: f32) -> [f32; 4] {
    s = s.clamp(0., 1.);
    v = v.clamp(0., 1.);
    h %= 360.0;

    let c = v * s;
    let hp = h / 60.;
    let x = c * (1. - (hp % 2. - 1.).abs());

    let [r1, g1, b1] = if 0. <= hp && hp < 1. {
        [c, x, 0.]
    } else if 1. <= hp && hp < 2. {
        [x, c, 0.]
    } else if 2. <= hp && hp < 3. {
        [0., c, x]
    } else if 3. <= hp && hp < 4. {
        [0., x, c]
    } else if 4. <= hp && hp < 5. {
        [x, 0., c]
    } else if 5. <= hp && hp < 6. {
        [c, 0., x]
    } else {
        unreachable!()
    };

    let m = v - c;
    [r1 + m, g1 + m, b1 + m, a]
}

const FPS_POLL_TIME_LIMIT: f32 = 0.5;

struct BasicTextAppInner {
    renderer: Renderer,
    text_renderer: TextRenderer,
    hello_world: Text,
    hello_world_sdf: Text,
    hello_world_outline: Text,
    hello_world_scaled: Text,
    fps_text: Text,
    frame_count: f32,
    fps_poll_start: Instant,
    start: Instant,
}

#[derive(Default)]
struct BasicTextApp {
    inner: Option<BasicTextAppInner>,
}

impl BasicTextAppInner {
    // -- IMPORTANT CODE IS IN THIS IMPL BLOCK --

    fn new(window: Arc<Window>) -> Self {
        let renderer = Renderer::new(window);

        // To use kaku, you first need to create a TextRenderer. This holds onto important data on
        // the GPU that we need to use for rendering.
        let mut text_renderer = TextRenderer::new(&renderer.device, &renderer.config, 1);
        let fira_sans = FontArc::new(
            FontRef::try_from_slice(include_bytes!("../fonts/FiraSans-Regular.ttf")).unwrap(),
        );

        let fira_sans_sdf =
            text_renderer.load_font_with_sdf(fira_sans.clone(), 60., SdfSettings { radius: 20.0 });
        let fira_sans = text_renderer.load_font(fira_sans, 60.);

        // If you want to create a lot of similar text with slightly different options, you can use
        // the TextBuilder in a stateful way:
        let mut builder = TextBuilder::new("hello, world! glyph :3", fira_sans, [50., 120.]);

        let hello_world = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        builder.font(fira_sans_sdf);
        builder.position([50., 220.]);
        let hello_world_sdf = builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        let outline_color = hsva_to_rgba(0.0, 1.0, 1.0, 1.0);
        builder.position([50., 340.]);
        builder.color([1.; 4]);
        builder.outlined(outline_color, 15.);
        let hello_world_outline =
            builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        builder.position([50., 520.]);
        builder.font_size(Some(120.));
        builder.color([0., 0., 0., 1.]);
        builder.no_outline();
        let hello_world_scaled =
            builder.build(&renderer.device, &renderer.queue, &mut text_renderer);

        // Or you can use the builder with chained methods like this for a one-off
        let fps_text = TextBuilder::new("fps: ", fira_sans_sdf, [40., 40.])
            .color([1., 0., 1., 1.])
            .scale(0.3)
            .outlined([1., 1., 1., 1.], 2.)
            .build(&renderer.device, &renderer.queue, &mut text_renderer);

        Self {
            text_renderer,
            renderer,
            hello_world,
            hello_world_sdf,
            hello_world_outline,
            hello_world_scaled,
            fps_text,
            fps_poll_start: Instant::now(),
            frame_count: 0.,
            start: Instant::now(),
        }
    }

    fn update(&mut self) {
        self.frame_count += 1.;
        let elapsed = self.fps_poll_start.elapsed().as_secs_f32();

        if elapsed > FPS_POLL_TIME_LIMIT {
            let fps = self.frame_count / elapsed;

            self.fps_text.set_text(
                format!("fps: {fps:.2}"),
                &self.renderer.device,
                &self.renderer.queue,
                &mut self.text_renderer,
            );

            self.frame_count = 0.;
            self.fps_poll_start = Instant::now();
        }

        let total_elapsed = self.start.elapsed().as_secs_f32();
        let outline_color = hsva_to_rgba(total_elapsed * 50., 1., 1., 1.);
        let outline_width = 10. * ((total_elapsed * std::f32::consts::PI).cos() + 1.) / 2. + 5.;
        self.hello_world_outline
            .set_outline(outline_color, outline_width, &self.renderer.queue);
        self.hello_world_outline.set_position(
            [
                50. + 5. * (total_elapsed * 3.).cos(),
                340. + 5. * (total_elapsed * 3.).sin(),
            ],
            &self.renderer.queue,
        );
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

        // Now, we can simply draw our Text objects onto the render pass using the TextRenderer
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
