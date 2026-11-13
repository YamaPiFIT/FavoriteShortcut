//! 1 つの OS ウィンドウに egui を載せて描く仕組み（winit + egui-winit + egui-wgpu）。
//!
//! GPU（wgpu のデバイス）は全ウィンドウで共有し、ウィンドウごとに egui のコンテキスト・入力・
//! 描画器を持つ。ウィンドウを自由に隠したり出したりでき、隠している間は何も描かない。
//! （トレイに常駐しながらホットキーでランチャーを一瞬で出すため、eframe ではなく自前で動かしている）

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_wgpu::wgpu;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// 全ウィンドウで共有する GPU。
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
}

impl Gpu {
    /// GPU を選んで初期化する。Windows 標準の DirectX 12 を優先し、使えない PC では
    /// これまでどおり使えるもの（Vulkan / OpenGL）を自動で選ぶ。
    /// 専用の GPU が無い環境でも、Windows のソフトウェア描画（Microsoft Basic Render Driver）などが
    /// 選ばれれば動く。どれも使えなければエラーを返す（呼び出し側で知らせて終了する）。
    ///
    /// DirectX 12 を優先するのは、Vulkan（特に NVIDIA のドライバー）だとアプリのメモリが
    /// 40〜70MB ほど多くなるため。環境変数 WGPU_BACKEND で指定されていれば、それだけを使う（確認用）。
    pub fn create(window: Arc<Window>) -> Result<(Self, wgpu::Surface<'static>), String> {
        if let Some(backends) = wgpu::Backends::from_env() {
            return Self::create_with(window, backends);
        }
        match Self::create_with(window.clone(), wgpu::Backends::DX12) {
            Ok(v) => Ok(v),
            Err(e) => {
                crate::log::warn(&format!("DirectX 12 で描画を始められませんでした（{e}）。ほかの方式を試します。"));
                Self::create_with(window, wgpu::Backends::PRIMARY | wgpu::Backends::GL)
            }
        }
    }

    fn create_with(window: Arc<Window>, backends: wgpu::Backends) -> Result<(Self, wgpu::Surface<'static>), String> {
        let mut create = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        create.instance_descriptor.backends = backends;
        // 常駐する小さなアプリなので、省電力側の GPU（ノート PC の内蔵 GPU など）を選ぶ
        create.power_preference = wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower);
        // 描画の速さより、使うメモリの少なさを優先する（画面が小さく、描く量も少ないため）。
        // 既定（速さ優先）だと、GPU 用のメモリを大きな単位でまとめて確保するので 60MB ほど多くなる
        let base = Arc::clone(&create.device_descriptor);
        create.device_descriptor =
            Arc::new(move |adapter| wgpu::DeviceDescriptor { memory_hints: wgpu::MemoryHints::MemoryUsage, ..base(adapter) });
        let setup = egui_wgpu::WgpuSetup::CreateNew(create);
        let instance = pollster::block_on(setup.new_instance());
        let surface = instance.create_surface(window).map_err(|e| e.to_string())?;
        let config = egui_wgpu::WgpuConfiguration { wgpu_setup: setup, ..Default::default() };
        let state =
            pollster::block_on(egui_wgpu::RenderState::create(&config, &instance, Some(&surface), egui_wgpu::RendererOptions::default()))
                .map_err(|e| e.to_string())?;
        crate::log::info(&format!("描画に使う GPU: {}", egui_wgpu::adapter_info_summary(&state.adapter.get_info())));
        Ok((Self { instance, adapter: state.adapter, device: state.device, queue: state.queue, format: state.target_format }, surface))
    }
}

/// egui を載せたウィンドウ 1 つ分。
pub struct WindowHost {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: egui_wgpu::Renderer,
    pub ctx: egui::Context,
    state: egui_winit::State,
    /// 次に描き直す時刻（None なら要求されるまで描かない）
    pub next_repaint: Option<Instant>,
    /// 中身に合わせて高さを変えるウィンドウか
    pub auto_height: bool,
    /// Ctrl+V が押された（クリップボードに文字が無いと egui には何も届かないため、自前で伝える）
    ctrl_v_pressed: bool,
    modifiers: winit::keyboard::ModifiersState,
}

pub struct FrameOutput {
    /// 中身が使った大きさ（論理座標）。高さを中身に合わせるときに使う
    pub content_height: f32,
}

impl WindowHost {
    pub fn new(
        gpu: &Gpu,
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
        surface: Option<wgpu::Surface<'static>>,
        setup_ctx: impl FnOnce(&egui::Context),
    ) -> Result<Self, String> {
        let surface = match surface {
            Some(s) => s,
            None => gpu.instance.create_surface(window.clone()).map_err(|e| e.to_string())?,
        };
        let size = window.inner_size();
        let caps = surface.get_capabilities(&gpu.adapter);
        let alpha_mode = caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: gpu.format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![gpu.format],
        };
        surface.configure(&gpu.device, &config);

        let renderer = egui_wgpu::Renderer::new(&gpu.device, gpu.format, egui_wgpu::RendererOptions::default());
        let ctx = egui::Context::default();
        setup_ctx(&ctx);
        let max_texture_side = gpu.device.limits().max_texture_dimension_2d as usize;
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side),
        );

        Ok(Self {
            window,
            surface,
            config,
            renderer,
            ctx,
            state,
            next_repaint: Some(Instant::now()),
            auto_height: false,
            ctrl_v_pressed: false,
            modifiers: winit::keyboard::ModifiersState::empty(),
        })
    }

    pub fn id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    /// winit からのイベントを egui に渡す。描き直しが必要なら true。
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            // 大きさが変わったときは、次の描画の前に描画先を作り直す（run_frame）
            WindowEvent::Resized(_) => {}
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::Focused(false) => self.modifiers = winit::keyboard::ModifiersState::empty(),
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } if event.state.is_pressed() && !event.repeat => {
                use winit::keyboard::{KeyCode, PhysicalKey};
                let m = self.modifiers;
                if event.physical_key == PhysicalKey::Code(KeyCode::KeyV) && m.control_key() && !m.alt_key() && !m.shift_key() {
                    self.ctrl_v_pressed = true;
                }
            }
            _ => {}
        }
        let response = self.state.on_window_event(&self.window, event);
        response.repaint
    }

    pub fn request_repaint(&mut self) {
        self.next_repaint = Some(Instant::now());
        // 隠しているウィンドウは描かない（表示するときに描き直す）
        if self.is_shown() {
            self.window.request_redraw();
        }
    }

    pub fn request_repaint_after(&mut self, delay: Duration) {
        let at = Instant::now() + delay;
        if self.next_repaint.is_none_or(|t| at < t) {
            self.next_repaint = Some(at);
        }
    }

    /// 1 フレーム描く。ui_fn に画面の中身を描かせる。
    pub fn run_frame(&mut self, gpu: &Gpu, mut ui_fn: impl FnMut(&mut egui::Ui)) -> FrameOutput {
        self.next_repaint = None;
        let mut raw_input = self.state.take_egui_input(&self.window);
        if std::mem::take(&mut self.ctrl_v_pressed)
            && !raw_input.events.iter().any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_)))
        {
            // ファイルだけをコピーしているときなど、egui に何も届かない Ctrl+V をキーとして伝える
            raw_input.events.push(egui::Event::Key {
                key: egui::Key::V,
                physical_key: Some(egui::Key::V),
                pressed: true,
                repeat: false,
                // egui-winit と同じく、Windows では Ctrl を command としても扱う
                modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() },
            });
        }
        let mut content_height = 0.0;
        let mut full_output = self.ctx.run_ui(raw_input, |ui| {
            ui_fn(ui);
            content_height = ui.min_rect().height();
        });
        self.state.handle_platform_output(&self.window, full_output.platform_output);

        // 次に描き直す時刻（アニメーションやカーソルの点滅など）
        if let Some(viewport) = full_output.viewport_output.get(&egui::ViewportId::ROOT) {
            let delay = viewport.repaint_delay;
            if delay < Duration::from_secs(3600) {
                self.request_repaint_after(delay);
            }
        }

        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 || self.window.is_minimized() == Some(true) {
            // 最小化中は描かない（テクスチャの更新だけは捨てずに反映する）
            self.update_textures(gpu, &full_output.textures_delta);
            self.free_textures(&full_output.textures_delta);
            full_output.textures_delta.clear();
            return FrameOutput { content_height };
        }
        if self.config.width != size.width || self.config.height != size.height {
            // ウィンドウの大きさに合わせて描画先を作り直してから描く。古い大きさのまま描くと、
            // DirectX 12 では引き伸ばされた絵が見えてしまう（中身に合わせて高さを変えたランチャーなど）
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&gpu.device, &self.config);
        }

        let ppp = full_output.pixels_per_point;
        let primitives = self.ctx.tessellate(full_output.shapes, ppp);
        self.update_textures(gpu, &full_output.textures_delta);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                self.request_repaint();
                None
            }
            _ => None,
        };

        if let Some(frame) = frame {
            // サイズが変わっていたら設定し直してから描く
            if frame.texture.width() != self.config.width || frame.texture.height() != self.config.height {
                drop(frame);
                self.surface.configure(&gpu.device, &self.config);
                self.request_repaint();
            } else {
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
                let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point: ppp };
                let user_buffers = self.renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &primitives, &screen);
                {
                    let mut pass = encoder
                        .begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("egui_render"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                                depth_slice: None,
                            })],
                            ..Default::default()
                        })
                        .forget_lifetime();
                    self.renderer.render(&mut pass, &primitives, &screen);
                }
                gpu.queue.submit(user_buffers.into_iter().chain(std::iter::once(encoder.finish())));
                self.window.pre_present_notify();
                gpu.queue.present(frame);
            }
        }

        self.free_textures(&full_output.textures_delta);
        // 反映済み（空にしておかないと egui の確認で止まる）
        full_output.textures_delta.clear();
        FrameOutput { content_height }
    }

    fn update_textures(&mut self, gpu: &Gpu, delta: &egui::TexturesDelta) {
        for (id, image_deltas) in &delta.set {
            for image_delta in image_deltas {
                self.renderer.update_texture(&gpu.device, &gpu.queue, *id, image_delta);
            }
        }
    }

    fn free_textures(&mut self, delta: &egui::TexturesDelta) {
        for id in &delta.free {
            self.renderer.free_texture(id);
        }
    }

    /// ウィンドウが表示されているか（Windows に問い合わせる。前面に出さずに表示した場合も正しく分かるように）。
    pub fn is_shown(&self) -> bool {
        match self.hwnd() {
            Some(hwnd) => unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool() },
            None => self.window.is_visible() == Some(true),
        }
    }

    /// Win32 のウィンドウハンドル（メッセージボックスの持ち主などに使う）。
    pub fn hwnd(&self) -> Option<windows::Win32::Foundation::HWND> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match self.window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(windows::Win32::Foundation::HWND(h.hwnd.get() as *mut _)),
            _ => None,
        }
    }
}
