// SPDX-License-Identifier: GPL-3.0-only
//! 自绘弹出菜单（托盘右键菜单用）：
//! 无边框 + 柔和阴影（替代系统菜单的固定边框）；交互：单击选中高亮、双击执行、
//! Esc/方向键、点击外部关闭。整窗带 UpdateLayeredWindow 一次上屏。
//! 文件分工：`wnd` 窗口创建与消息分发、`draw` 上屏绘制。

mod draw;
mod wnd;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DEFAULT_GUI_FONT, DeleteDC, DeleteObject, GetDC, GetStockObject, GetTextExtentPoint32W, HBITMAP, HDC, ReleaseDC,
    SelectObject,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::overlay::dib::create_dib_surface;

pub(super) const WINDOW_CLASS: PCWSTR = w!("airsnip.popmenu");
/// 单位（未缩放）行高。
pub(super) const ITEM_H: i32 = 34;
/// 文字左右内边距。
pub(super) const PAD_X: i32 = 18;
/// 分隔线占位高度。
pub(super) const SEP_H: i32 = 9;
/// 内容圆角。
pub(super) const CORNER: f32 = 8.0;
/// 菜单最小宽度。
const MIN_W: i32 = 150;

/// 菜单项。
pub struct Item {
    pub id: usize,
    pub text: &'static str,
    /// true 时在本项上方绘制分隔线。
    pub separator_before: bool,
}

/// 弹出菜单运行上下文。
pub(super) struct Ctx {
    pub items: Vec<Item>,
    pub palette: &'static crate::theme::Palette,
    pub scale: f32,
    pub w: i32,
    pub h: i32,
    /// 每项内容区 y（含分隔线占位，已缩放）。
    pub ys: Vec<i32>,
    pub hover: Option<usize>,
    pub result: Option<usize>,
    pub hwnd: HWND,
    pub mem_dc: HDC,
    pub dib_bmp: HBITMAP,
    pub dib_bits: *mut u8,
}

impl Drop for Ctx {
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

/// 弹出菜单并阻塞，返回双击选择的菜单项 id（未选择返回 None）。
pub fn popup(owner: HWND, items: Vec<Item>, palette: &'static crate::theme::Palette) -> Option<usize> {
    let scale = unsafe { GetDpiForWindow(owner) as f32 / 96.0 };
    let item_h = (ITEM_H as f32 * scale) as i32;
    let sep_h = (SEP_H as f32 * scale) as i32;
    let pad_x = (PAD_X as f32 * scale) as i32;

    // 测量文本宽度（默认 UI 字体）决定菜单宽
    let hdc = unsafe { GetDC(Some(owner)) };
    let max_w = {
        let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        unsafe {
            let _ = SelectObject(hdc, font.into());
        }
        let mut mw = 0i32;
        for it in &items {
            if it.text.is_empty() {
                continue;
            }
            let buf: Vec<u16> = it.text.encode_utf16().collect();
            let mut sz = SIZE::default();
            unsafe {
                let _ = GetTextExtentPoint32W(hdc, &buf, &mut sz);
            }
            mw = mw.max(sz.cx);
        }
        mw
    };
    unsafe {
        let _ = ReleaseDC(Some(owner), hdc);
    }
    let w = (max_w + pad_x * 2).max(MIN_W);
    let mut ys = Vec::with_capacity(items.len());
    let mut y = 0i32;
    for it in &items {
        if it.separator_before {
            y += sep_h;
        }
        ys.push(y);
        y += item_h;
    }
    let h = y;

    // 窗口位置：光标左上方（托盘菜单向上弹出），限定在屏幕内
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = (pt.x - w + 8).clamp(0, (sw - w).max(0));
    let ypos = (pt.y - h + 8).clamp(0, (sh - h).max(0));

    let hwnd = match wnd::create_window(owner, x, ypos, w, h) {
        Ok(h) => h,
        Err(_) => return None,
    };
    let mut ctx = Ctx {
        items,
        palette,
        scale,
        w,
        h,
        ys,
        hover: None,
        result: None,
        hwnd,
        mem_dc: Default::default(),
        dib_bmp: Default::default(),
        dib_bits: std::ptr::null_mut(),
    };
    // 上屏 DIB
    match create_dib_surface(w as u32, h as u32) {
        Ok((dc, bmp, bits)) => {
            ctx.mem_dc = dc;
            ctx.dib_bmp = bmp;
            ctx.dib_bits = bits;
        }
        Err(_) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return None;
        }
    }
    let raw = Box::into_raw(Box::new(ctx));
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        let _ = SetFocus(Some(hwnd));
    }
    unsafe {
        draw::present(&*raw);
    }

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 != 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
    let ctx = unsafe { Box::from_raw(raw) };
    let result = ctx.result;
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    result
}

impl Ctx {
    /// 命中菜单项（内容区外返回 None）。
    pub(super) fn hit_item(&self, x: i32, y: i32) -> Option<usize> {
        let item_h = (ITEM_H as f32 * self.scale) as i32;
        if x < 0 || x >= self.w || y < 0 || y >= self.h {
            return None;
        }
        self.ys.iter().position(|yy| y >= *yy && y < *yy + item_h)
    }

    pub(super) fn close(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}
