// SPDX-License-Identifier: GPL-3.0-only
//! 设置窗口创建与尺寸校正：窗口类注册（含 exe 图标）、客户区按 DPI 计算、
//! 上屏 DIB 重建。窗口过程见 `super::wndproc`，交互见 `interact`。

use windows::Win32::Foundation::{HINSTANCE, HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::app::icon_resource;
use crate::overlay::dib::create_dib_surface;
use crate::overlay::ui::Ui;

use super::controls::{self, Section, W};
use super::{Panel, WINDOW_CLASS};

/// 创建设置窗口：初始按 scale=1 粗建，创建后按真实 DPI 校正。
pub(super) fn create_window(hinstance: HINSTANCE, panel: &mut Panel) -> Result<HWND, String> {
    unsafe {
        // 标题栏图标取自 exe 资源（app.ico）；不设则显示系统默认图标
        let icon = LoadIconW(Some(hinstance), icon_resource()).unwrap_or_default();
        let icon_sm = LoadImageW(
            Some(hinstance),
            icon_resource(),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
        .map(|h| HICON(h.0))
        .unwrap_or(icon);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(super::wndproc),
            hInstance: hinstance,
            hIcon: icon,
            hIconSm: icon_sm,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            // 面板可多次打开：类已注册不算错误
            if windows::Win32::Foundation::GetLastError().0 != 1410 {
                return Err("设置窗口类注册失败".into());
            }
        }

        // 固定尺寸窗口：标题栏 + 系统菜单 + 最小化（禁调整大小，避免黑边）
        const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0);
        let (cw, ch) = (W, controls::layout(Section::General, 1.0).height);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: cw,
            bottom: ch,
        };
        let _ = AdjustWindowRectEx(&mut rect, STYLE, false, WINDOW_EX_STYLE(0));
        let (sw, sh) = (rect.right - rect.left, rect.bottom - rect.top);
        let cx = (GetSystemMetrics(SM_CXSCREEN) - sw) / 2;
        let cy = (GetSystemMetrics(SM_CYSCREEN) - sh) / 2;

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WINDOW_CLASS,
            w!("AirSnip 设置"),
            STYLE,
            cx,
            cy,
            sw,
            sh,
            None,
            None,
            Some(hinstance),
            None,
        )
        .map_err(|e| format!("创建设置窗口失败：{e}"))?;

        // 面板与截图覆盖层共用同一套界面缩放（ADR 0016）：面板内改设置能当场看到效果
        panel.scale = Ui::resolve(panel.config.ui_scale).scale();
        panel.fit_window(hwnd);
        Ok(hwnd)
    }
}

/// 深色主题下让系统标题栏跟随（`DWMWA_USE_IMMERSIVE_DARK_MODE`，Win10 20H1+）。
/// 标题栏由 DWM 绘制，自绘只能覆盖客户区，必须显式告知主题。
pub(super) fn apply_dark_titlebar(hwnd: HWND, dark: bool) {
    let value: i32 = i32::from(dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

impl Panel {
    /// 内容区总宽（含 DPI 缩放）。
    pub(super) fn content_w(&self) -> i32 {
        (W as f32 * self.scale) as i32
    }

    /// 导航栏右缘 x 坐标（含 DPI 缩放）。
    pub(super) fn nav_w(&self) -> i32 {
        (super::controls::NAV_W as f32 * self.scale) as i32
    }

    /// 按当前节重建上屏 DIB，并把窗口外框调到「客户区 = 绘制位图」尺寸。
    /// 非客户区用 GetWindowRect/GetClientRect 实测差值补齐，窗口中心保持不动；
    /// 界面缩放变大后可能超出屏幕，夹回工作区内。
    pub(super) fn fit_window(&mut self, hwnd: HWND) {
        let w = self.content_w();
        let ch = controls::layout(self.section, self.scale).height;
        unsafe {
            if !self.dib_bmp.0.is_null() {
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(self.dib_bmp.into());
            }
            if !self.mem_dc.0.is_null() {
                let _ = windows::Win32::Graphics::Gdi::DeleteDC(self.mem_dc);
            }
        }
        match create_dib_surface(w as u32, ch as u32) {
            Ok((dc, bmp, bits)) => {
                self.mem_dc = dc;
                self.dib_bmp = bmp;
                self.dib_bits = bits;
            }
            Err(_) => return,
        }
        unsafe {
            let mut wr = RECT::default();
            let mut cr = RECT::default();
            let _ = GetWindowRect(hwnd, &mut wr);
            let _ = GetClientRect(hwnd, &mut cr);
            // SetWindowPos 的宽高是窗口外框：必须加上非客户区，否则客户区比绘制位图窄、右侧被裁
            let nc_w = (wr.right - wr.left) - (cr.right - cr.left);
            let nc_h = (wr.bottom - wr.top) - (cr.bottom - cr.top);
            let (outer_w, outer_h) = (w + nc_w, ch + nc_h);
            let nx = wr.left + ((cr.right - cr.left) - w) / 2;
            let ny = wr.top + ((cr.bottom - cr.top) - ch) / 2;
            // 缩放变大后窗口可能超出屏幕：夹回屏幕内（放不下时贴左上角）
            let sw = GetSystemMetrics(SM_CXSCREEN);
            let sh = GetSystemMetrics(SM_CYSCREEN);
            let nx = nx.clamp(0, (sw - outer_w).max(0));
            let ny = ny.clamp(0, (sh - outer_h).max(0));
            let _ = SetWindowPos(hwnd, None, nx, ny, outer_w, outer_h, SWP_NOZORDER);
        }
    }
}
