use eframe::wgpu;
use openitgo_app::app::{ReaderApp, View};
use openitgo_app::opener::AsyncOpener;
use openitgo_core::models::Comic;
use std::time::{Duration, Instant};

fn main() -> eframe::Result<()> {
    std::env::set_var("OPENITGO_LOG", "1");
    let tmp = tempfile::tempdir().expect("temporary BC3 fixture directory");
    let path = tmp.path().join("page.png");
    image::RgbaImage::from_pixel(5, 7, image::Rgba([255, 0, 0, 255]))
        .save(&path)
        .expect("write BC3 fixture");

    let mut app = ReaderApp::default();
    app.settings.compress_images = true;
    app.page_loader = openitgo_app::loader::PageLoader::new_with_compress(true, 1);
    app.opener = Some(AsyncOpener::<Comic>::open(tmp.path().to_path_buf(), |p| {
        openitgo_parser::parse(p).map_err(|e| e.to_string())
    }));
    app.current_view = View::Loading(path);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: openitgo_app::cache::wgpu_options_with_bc3(),
        ..Default::default()
    };
    eframe::run_native(
        "OpenItGo BC3 smoke",
        options,
        Box::new(|cc| {
            openitgo_app::cache::install_wgpu_render_state(
                &cc.egui_ctx,
                cc.wgpu_render_state.clone(),
            );
            openitgo_app::fonts::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(SmokeApp {
                app,
                render_state: cc.wgpu_render_state.clone(),
                start: Instant::now(),
                finished: false,
            }))
        }),
    )
}

struct SmokeApp {
    app: ReaderApp,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    start: Instant,
    finished: bool,
}

impl eframe::App for SmokeApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        openitgo_app::cache::install_wgpu_render_state(
            ui.ctx(),
            frame.wgpu_render_state().cloned(),
        );
        self.app.ui(ui, frame);
        let ctx = ui.ctx();
        ctx.request_repaint_after(Duration::from_millis(100));

        if self.finished {
            return;
        }
        let Some(reader) = self.app.reader_view.open.as_mut() else {
            if self.start.elapsed() > Duration::from_secs(30) {
                panic!("timed out waiting for BC3 page");
            }
            return;
        };
        let current = reader.state.current_page;
        if !reader.cache.contains_full(current) {
            if self.start.elapsed() > Duration::from_secs(30) {
                panic!("timed out waiting for BC3 texture");
            }
            return;
        }
        let Some(handle) = reader.cache.get_texture(ctx, current) else {
            panic!("BC3 texture handle missing");
        };
        let Some((texture, gpu_size)) = reader.cache.native_texture(current) else {
            panic!("native BC3 texture missing");
        };
        assert_eq!(gpu_size, [8, 8]);
        self.finished = true;
        let render_state = self.render_state.as_ref().expect("wgpu render state");
        let bytes_per_row = (gpu_size[0] * 4).next_multiple_of(256);
        let buffer = render_state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bc3-smoke-readback"),
            size: (bytes_per_row * gpu_size[1]) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = render_state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(gpu_size[1]),
                },
            },
            wgpu::Extent3d {
                width: gpu_size[0],
                height: gpu_size[1],
                depth_or_array_layers: 1,
            },
        );
        render_state.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        render_state
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        receiver.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let mut rgba = [0u8; 64];
        bcdec_rs::bc3(&data[..16], &mut rgba, 16);
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        drop(data);
        buffer.unmap();
        eprintln!(
            "[smoke] native BC3 readback passed: texture {:?}",
            handle.id()
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
