// SPDX-License-Identifier: GPL-3.0-only
//! 光标（CAP-2 / EDT-7）：核心给出的形状（`airsnip_core::overlay::cursor`）→ Win32 光标句柄。
//! 自绘形状（十字 / 抓手）见 `paint`、圆角见 `radius`，方向与移动光标用系统内置。

mod paint;
mod radius;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, HCURSOR, IDC_ARROW, IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE,
    LoadCursorW,
};
use windows::core::PCWSTR;

use super::ui::Ui;
use super::{Overlay, Shape};

/// 光标画布基准边长（100% DPI）：各形状按 `ui.px(CANVAS_BASE)` 等比缩放。
pub(super) const CANVAS_BASE: f32 = 32.0;

/// 形状 → 光标句柄。
pub(super) fn hcursor(shape: Shape, ui: Ui) -> HCURSOR {
    match shape {
        Shape::Arrow => load(IDC_ARROW),
        Shape::Cross => paint::cross_cursor(ui),
        Shape::Hand => paint::hand_cursor(ui),
        Shape::Move => load(IDC_SIZEALL),
        Shape::NS => load(IDC_SIZENS),
        Shape::WE => load(IDC_SIZEWE),
        Shape::NWSE => load(IDC_SIZENWSE),
        Shape::NESW => load(IDC_SIZENESW),
        Shape::Radius => radius::radius_cursor(ui),
    }
}

/// 系统光标句柄（进程共享，`LoadCursorW` 只是查表，无需额外缓存）。
fn load(id: PCWSTR) -> HCURSOR {
    unsafe { LoadCursorW(None, id).unwrap_or_default() }
}

/// 窗口类默认光标（系统十字）：真正的光标由 `WM_SETCURSOR` 主动 `SetCursor`，
/// 这里只做兜底——窗口类不持有会随 DPI 重建的自绘光标句柄（CAP-5）。
pub(super) fn class_cursor() -> HCURSOR {
    load(IDC_CROSS)
}

/// 按当前鼠标屏幕位置刷新光标（`WM_SETCURSOR`、空格按下/松开时用）。
pub(super) fn apply_now(overlay: &mut Overlay, hwnd: HWND) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
        let _ = ScreenToClient(hwnd, &mut p);
    }
    overlay.apply_cursor_at(p.x, p.y);
}
