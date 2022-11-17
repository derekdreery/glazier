// Copyright 2018 The Druid Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::any::Any;
use std::sync::{Arc, Mutex};

use glazier::kurbo::Size;

use glazier::{
    Application, Cursor, FileDialogOptions, FileDialogToken, FileInfo, FileSpec, HotKey, KeyEvent,
    Menu, MouseEvent, Region, SysMods, TimerToken, WinHandler, WindowBuilder, WindowHandle,
};
use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};
use tracing::info;

#[derive(Default)]
struct HelloState {
    size: Size,
    handle: WindowHandle,
    gpu_state: Arc<Mutex<Option<GpuState>>>,
}

impl WinHandler for HelloState {
    fn connect(&mut self, handle: &WindowHandle) {
        self.handle = handle.clone();
    }

    fn size(&mut self, size: Size) {
        info!("size: {:?}", size);
        let mut guard = self.gpu_state.lock().unwrap();
        unsafe { guard.as_mut().unwrap().resize() }.unwrap();
    }

    fn prepare_paint(&mut self) {
        self.handle.invalidate();
    }

    fn paint(&mut self, _: &Region) {
        self.render();
    }

    fn command(&mut self, id: u32) {
        match id {
            0x100 => {
                self.handle.close();
                Application::global().quit()
            }
            0x101 => {
                let options = FileDialogOptions::new().show_hidden().allowed_types(vec![
                    FileSpec::new("Rust Files", &["rs", "toml"]),
                    FileSpec::TEXT,
                    FileSpec::JPG,
                ]);
                self.handle.open_file(options);
            }
            0x102 => {
                let options = FileDialogOptions::new().show_hidden().allowed_types(vec![
                    FileSpec::new("Rust Files", &["rs", "toml"]),
                    FileSpec::TEXT,
                    FileSpec::JPG,
                ]);
                self.handle.save_as(options);
            }
            _ => info!("unexpected id {}", id),
        }
    }

    fn save_as(&mut self, _token: FileDialogToken, file: Option<FileInfo>) {
        info!("save file result: {:?}", file);
    }

    fn open_file(&mut self, _token: FileDialogToken, file_info: Option<FileInfo>) {
        info!("open file result: {:?}", file_info);
    }

    fn key_down(&mut self, event: KeyEvent) -> bool {
        info!("keydown: {:?}", event);
        false
    }

    fn key_up(&mut self, event: KeyEvent) {
        info!("keyup: {:?}", event);
    }

    fn wheel(&mut self, event: &MouseEvent) {
        info!("mouse_wheel {:?}", event);
    }

    fn mouse_move(&mut self, event: &MouseEvent) {
        self.handle.set_cursor(&Cursor::Arrow);
        info!("mouse_move {:?}", event);
    }

    fn mouse_down(&mut self, event: &MouseEvent) {
        info!("mouse_down {:?}", event);
        self.render();
    }

    fn mouse_up(&mut self, event: &MouseEvent) {
        info!("mouse_up {:?}", event);
    }

    fn timer(&mut self, id: TimerToken) {
        info!("timer fired: {:?}", id);
    }

    fn got_focus(&mut self) {
        info!("Got focus");
    }

    fn lost_focus(&mut self) {
        info!("Lost focus");
    }

    fn request_close(&mut self) {
        self.handle.close();
    }

    fn destroy(&mut self) {
        Application::global().quit()
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

impl HelloState {
    fn render(&self) {
        unsafe {
            let (width, height) = size_px(&self.handle);
            let mut state_guard = self.gpu_state.lock().unwrap();
            let state = state_guard.as_mut().unwrap();
            let output = state.surface.get_current_texture().unwrap();
            let view = output
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder =
                state
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Render Encoder"),
                    });
            {
                let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.1,
                                g: 0.2,
                                b: 0.3,
                                a: 1.0,
                            }),
                            store: true,
                        },
                    })],
                    depth_stencil_attachment: None,
                });
            }
            state.queue.submit(std::iter::once(encoder.finish()));
            output.present();
        }
    }
}

fn main() {
    tracing_subscriber::fmt().init();
    let mut file_menu = Menu::new();
    file_menu.add_item(
        0x100,
        "E&xit",
        Some(&HotKey::new(SysMods::Cmd, "q")),
        Some(true),
        false,
    );
    file_menu.add_item(
        0x101,
        "O&pen",
        Some(&HotKey::new(SysMods::Cmd, "o")),
        Some(true),
        false,
    );
    file_menu.add_item(
        0x102,
        "S&ave",
        Some(&HotKey::new(SysMods::Cmd, "s")),
        Some(true),
        false,
    );
    let mut menubar = Menu::new();
    menubar.add_dropdown(Menu::new(), "Application", true);
    menubar.add_dropdown(file_menu, "&File", true);

    let app = Application::new().unwrap();
    let mut builder = WindowBuilder::new(app.clone());
    let win_state = HelloState::default();
    let gpu_state = win_state.gpu_state.clone();
    builder.set_handler(Box::new(win_state));
    builder.set_title("Hello example");
    builder.set_menu(menubar);

    let window = builder.build().unwrap();
    unsafe {
        let state = pollster::block_on(GpuState::new(window.clone()));
        *gpu_state.lock().unwrap() = Some(state);
    }
    window.show();

    app.run(None);
}

struct GpuState {
    handle: WindowHandle,
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: (u32, u32),
}

impl GpuState {
    async unsafe fn new(handle: WindowHandle) -> Self {
        let (width, height) = size_px(&handle);

        let instance = wgpu::Instance::new(wgpu::Backends::all());
        let surface = instance.create_surface(&handle);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    features: wgpu::Features::empty(),
                    limits: if cfg!(target_arch = "wasm32") {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    },
                    label: None,
                },
                None,
            )
            .await
            .unwrap();

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface.get_supported_formats(&adapter)[0],
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
        };
        surface.configure(&device, &config);

        Self {
            handle,
            surface,
            device,
            queue,
            config,
            size: (width, height),
        }
    }

    unsafe fn resize(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (width, height) = size_px(&self.handle);
        if self.size == (width, height) {
            // nothing to do
            return Ok(());
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);

        Ok(())
    }
}

fn size_px(handle: &WindowHandle) -> (u32, u32) {
    let Size { width, height } = handle.get_size();
    info!("size: ({}, {})", width, height);
    info!("scale: {:?}", handle.get_scale().unwrap());
    let (width, height) = handle.get_scale().unwrap().dp_to_px_xy(width, height);
    let (width, height) = (width as u32, height as u32);
    assert!(width > 0 && height > 0, "width and height must be > 0");
    (width, height)
}
