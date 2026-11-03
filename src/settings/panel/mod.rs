// SPDX-License-Identifier: GPL-3.0-only
//! 设置面板（M3）：自绘设置窗口（tiny-skia 绘制 + DirectWrite 文字）。
//! 布局为左侧分类导航 + 右侧内容区；主线程模态运行，交互只改工作副本 `config`，
//! 关闭时经 `App::apply_panel_config` 统一写回、保存并重注册热键；
//! 主题与开机自启即时生效（面板自身重绘 + schtasks 调用）。
//! 窗口尺寸固定为横向矩形（宽 > 高），切换分类页不改变窗口大小。

mod controls;
mod draw;
mod interact;
mod keycap;
mod link;
mod reward;
mod update;
mod window;

use tiny_skia::Pixmap;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, DeleteDC, DeleteObject, EndPaint, GetDC, HBITMAP, HDC, InvalidateRect, PAINTSTRUCT, ReleaseDC,
    SRCCOPY, UpdateWindow,
};
use windows::Win32::UI::Input::Ime::HIMC;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::app::{App, message_box};
use crate::overlay::dib::convert_rgba_to_bgra;
use crate::theme::{self, Palette};

use controls::{RowId, Section, W};

const WINDOW_CLASS: PCWSTR = w!("airsnip.settings");

/// 面板运行时状态。
pub struct Panel {
    pub palette: &'static Palette,
    /// 工作副本（关闭时写回）。
    pub config: crate::settings::Config,
    /// 界面缩放系数（与截图覆盖层同一套，ADR 0016）：改设置时立即重算并重排窗口，
    /// 面板自身当场就能反映新倍率。
    pub scale: f32,
    /// 内容区悬停行下标。
    pub hover: Option<usize>,
    /// 导航栏悬停分类。
    pub nav_hover: Option<Section>,
    /// 当前分类页。
    pub section: Section,
    /// 正在捕获的热键。
    pub capturing: Option<RowId>,
    /// 面板窗口句柄（IME 关联操作用）。
    pub(super) hwnd: HWND,
    /// 主程序窗口句柄（更新完成后需要通知它退出，SYS-7）。
    pub(super) main_hwnd: HWND,
    /// 捕获期间的 IME 上下文（禁输入法，防止按键被 IME 改写为 0xE5 之类的键码）。
    pub(super) ime_old: HIMC,
    /// 开机自启真实状态（任务计划是否存在，SYS-4）。
    pub autostart_on: bool,
    /// 配置有改动、待立即应用（热键/主题即时生效，不等面板关闭）。
    pub(super) config_dirty: bool,
    /// 上屏缓冲（DIB section）。
    pub(super) dib_bmp: HBITMAP,
    pub(super) mem_dc: HDC,
    pub(super) dib_bits: *mut u8,
}

impl Drop for Panel {
    fn drop(&mut self) {
        unsafe {
            if !self.dib_bmp.0.is_null() {
                let _ = DeleteObject(self.dib_bmp.into());
            }
            if !self.mem_dc.0.is_null() {
                let _ = DeleteDC(self.mem_dc);
            }
        }
    }
}

/// 打开设置面板（主线程模态，阻塞到窗口关闭）。
pub fn run(app: &mut App) {
    let hinstance = app.hinstance;
    let mut panel = Panel {
        palette: theme::palette(theme::resolve(app.config.theme)),
        config: app.config.clone(),
        scale: 1.0,
        hover: None,
        nav_hover: None,
        section: Section::General,
        capturing: None,
        hwnd: HWND::default(),
        main_hwnd: app.hwnd,
        ime_old: HIMC(std::ptr::null_mut()),
        // 自启开关显示任务计划真实状态，而非配置猜测（SYS-4）
        autostart_on: crate::app::autostart::autostart_enabled(),
        config_dirty: false,
        dib_bmp: Default::default(),
        mem_dc: Default::default(),
        dib_bits: std::ptr::null_mut(),
    };

    let hwnd = match window::create_window(hinstance, &mut panel) {
        Ok(h) => h,
        Err(e) => {
            message_box(None, &e, MB_OK | MB_ICONERROR);
            return;
        }
    };
    panel.hwnd = hwnd;
    // 后台线程（更新检查/下载）通过这个句柄通知面板刷新关于页（SYS-7）
    crate::update::notify::register(hwnd);
    // 标题栏由 DWM 绘制：创建后先按当前主题着色（切换主题时在 `set_theme` 里更新）
    window::apply_dark_titlebar(hwnd, theme::resolve(panel.config.theme) == theme::Theme::Dark);
    let raw = Box::into_raw(Box::new(panel));
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(Some(hwnd), None, false);
        let _ = UpdateWindow(hwnd);
    }

    // 面板消息循环：WM_DESTROY 时 wndproc 调 PostQuitMessage(0) 结束。
    // 每轮检查 config_dirty：热键/主题改动立即写回应用（不等关闭），
    // 避免面板开着时旧热键仍生效、新热键无反应的困惑。
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 != 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
            let p = &mut *raw;
            if p.config_dirty {
                p.config_dirty = false;
                let cfg = p.config.clone();
                app.apply_panel_config(&cfg);
            }
        }
    }

    let panel = unsafe { Box::from_raw(raw) };
    // 面板即将销毁：不再接收更新状态通知（否则消息会投到已销毁窗口）
    crate::update::notify::unregister();
    // 兜底：恢复输入法 + 最后同步一次配置（如改完直接关窗）
    let mut panel = panel;
    panel.restore_ime();
    app.apply_panel_config(&panel.config);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// 创建设置窗口：初始按 scale=1 粗建，创建后按真实 DPI 校正。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // 窗口销毁（DestroyWindow 同步发送）：必须 PostQuitMessage 结束面板消息循环，
    // 否则面板线程/主线程永久阻塞在 GetMessageW（见覆盖层同款修复，M3）。
    if msg == WM_DESTROY {
        unsafe {
            let _ = PostQuitMessage(0);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    let Some(panel) = app_from(hwnd) else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                let _ = BeginPaint(hwnd, &mut ps);
            }
            repaint_full(hwnd, panel);
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // 后台更新线程的状态变更（SYS-7）：只重绘，状态从 `update::state` 现读
        crate::update::notify::WM_UPDATE_STATE => {
            repaint_full(hwnd, panel);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            let (nav_hover, hover) = panel.hover_at(x, y);
            if nav_hover != panel.nav_hover || hover != panel.hover {
                panel.nav_hover = nav_hover;
                panel.hover = hover;
                repaint_full(hwnd, panel);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            panel.on_click(x, y);
            panel.fit_window(hwnd);
            repaint_full(hwnd, panel);
            LRESULT(0)
        }
        WM_SYSKEYDOWN | WM_KEYDOWN => {
            // WM_SYSKEYDOWN：Alt 组合键走系统键路径，同样进入热键捕获（否则 Alt+A 捕获不到）
            if panel.capturing.is_some() {
                panel.on_key(wparam.0 as u32);
                // 捕获结束（键已记录 / Esc 取消）时刷新按钮显示：从等待态切回正常文案
                if panel.capturing.is_none() {
                    repaint_full(hwnd, panel);
                }
                // 捕获态吞掉按键，防止触发系统菜单或其它默认行为
                LRESULT(0)
            } else {
                // 非捕获态交给默认处理（Alt+F4 关闭、Tab 切换焦点等不能吞掉）
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn app_from(hwnd: HWND) -> Option<&'static mut Panel> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr == 0 {
            None
        } else {
            Some(&mut *(ptr as *mut Panel))
        }
    }
}

/// 全量重绘：tiny-skia 绘制 → RGBA→BGRA → BitBlt 上屏。
fn repaint_full(hwnd: HWND, panel: &Panel) {
    let layout = controls::layout(panel.section, panel.scale);
    let pw = (W as f32 * panel.scale) as u32;
    let Some(mut pixmap) = Pixmap::new(pw, layout.height as u32) else {
        return;
    };
    draw::render(&mut pixmap, panel, &layout);
    let w = pixmap.width() as i32;
    let h = pixmap.height() as i32;
    convert_rgba_to_bgra(pixmap.data(), panel.dib_bits as usize, w, 0, 0, w, h, 1);
    unsafe {
        let hdc = GetDC(Some(hwnd));
        if hdc.0.is_null() {
            return;
        }
        let _ = BitBlt(hdc, 0, 0, w, h, Some(panel.mem_dc), 0, 0, SRCCOPY);
        let _ = ReleaseDC(Some(hwnd), hdc);
    }
}
