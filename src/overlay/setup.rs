// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层窗口创建（CAP-2）：注册窗口类并创建覆盖整个虚拟屏幕的无边框置顶窗口。
//! 与 `mod.rs` 的运行/消息循环分开，便于控制单文件规模。

use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, RegisterClassW, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::{cursors, wndproc};

const WINDOW_CLASS: PCWSTR = w!("airsnip.overlay");

/// 注册窗口类并创建覆盖窗口。
pub(super) fn create_window(hinstance: HINSTANCE) -> Result<HWND, String> {
    unsafe {
        let wc = WNDCLASSW {
            // CS_DBLCLKS：否则系统不会发 WM_LBUTTONDBLCLK（双击控制柄恢复直线，EDT-3）
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc::wndproc),
            hInstance: hinstance,
            hCursor: cursors::class_cursor(),
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
        let t_reg = std::time::Instant::now();
        let hwnd = CreateWindowExW(
            // 截图期间需接收键盘（Esc 取消 / 回车复制 / Ctrl+C 等）：不用 WS_EX_NOACTIVATE，
            // 否则窗口无输入焦点，WM_KEYDOWN/WM_HOTKEY 之外的键（如 Esc、回车）收不到（M3）。
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            WINDOW_CLASS,
            w!("airsnip.overlay"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
        .map_err(|e| format!("创建覆盖层窗口失败：{e}"))?;
        if crate::perf::enabled() {
            eprintln!("[airsnip] 建窗：CreateWindowEx {:?}", t_reg.elapsed());
        }
        Ok(hwnd)
    }
}
