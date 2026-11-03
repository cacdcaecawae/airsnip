// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层 Windows 外壳（CAP-2/3）：覆盖整个虚拟屏幕的无边框置顶窗口、消息循环与 Win32 宿主。
//! 交互与渲染逻辑在 `airsnip_core::overlay`（此处再导出，`crate::overlay::…` 路径不变）。
//!
//! 文件分工：`setup` 窗口创建、`wndproc` 消息分发与修饰键、`host` 平台能力（上屏/光标/剪贴板/
//! 对话框/贴图/输入法）、`cursors` 形状 → Win32 光标、`dib` GDI 上屏缓冲。

mod cursors;
pub(crate) mod dib;
mod host;
mod setup;
mod wndproc;

pub use airsnip_core::overlay::*;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::capture::CapturedScreen;
use crate::editor::Tool;
use crate::theme::Palette;

/// 显示覆盖层并阻塞运行，直到截图完成、取消或触发长截图。
/// 返回类型区分三种结果（普通截图 / 取消 / 长截图视口选取）。
#[allow(clippy::too_many_arguments)]
pub fn run(
    capture: CapturedScreen,
    palette: &'static Palette,
    tool_keys: crate::settings::ToolKeys,
    longshot_key: u32,
    default_tool: Tool,
    save: crate::settings::SavePrefs,
    ui_scale: crate::settings::UiScale,
    hinstance: HINSTANCE,
    main_hwnd: HWND,
) -> OverlayResult {
    let t_show = std::time::Instant::now();
    let (origin_x, origin_y) = (capture.origin_x, capture.origin_y);
    let (vw, vh) = (capture.width as i32, capture.height as i32);
    let t_win = std::time::Instant::now();
    let hwnd = match setup::create_window(hinstance) {
        Ok(h) => h,
        Err(e) => {
            crate::app::message_box(None, &e, MB_OK | MB_ICONERROR);
            return OverlayResult::Cancelled;
        }
    };
    let destroy = || unsafe {
        let _ = DestroyWindow(hwnd);
    };
    let mut host = match host::WinHost::new(hwnd, main_hwnd, capture.width, capture.height) {
        Ok(h) => h,
        Err(e) => {
            destroy();
            crate::app::message_box(None, &e, MB_OK | MB_ICONERROR);
            return OverlayResult::Cancelled;
        }
    };
    // 截图期间禁用输入法：否则 S（长截图）/ 回车等按键被 IME 截走；
    // 进入文本输入时再恢复（`Host::set_ime`，EDT-4）。
    host.set_ime(false);
    // 界面缩放：手动倍率或按光标所在显示器的 DPI + 分辨率自动（CAP-5）
    let ui = ui::Ui::resolve(ui_scale);
    let overlay = Overlay::new(
        capture,
        palette,
        tool_keys,
        longshot_key,
        default_tool,
        save,
        ui,
        Box::new(host),
    );
    let Ok(overlay) = overlay else {
        destroy();
        return OverlayResult::Cancelled;
    };
    let t_init_done = std::time::Instant::now();
    let raw = Box::into_raw(Box::new(overlay));

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let t_pos = std::time::Instant::now();
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), origin_x, origin_y, vw, vh, SWP_SHOWWINDOW);
        let pos = t_pos.elapsed();
        // 显示后强制整窗重绘：隐藏窗口的 DC 内容在显示时会丢失，
        // 用 InvalidateRect + UpdateWindow 同步触发 WM_PAINT 全窗绘制，避免闪烁。
        let _ = InvalidateRect(Some(hwnd), None, false);
        let _ = UpdateWindow(hwnd);
        let paint = t_pos.elapsed() - pos;
        // 前台与焦点改为异步（消息循环处理）：同步调用会阻塞至 ~45ms（M5 性能）
        let _ = PostMessageW(Some(hwnd), wndproc::WM_FOCUS_OVERLAY, WPARAM(0), LPARAM(0));
        if crate::perf::enabled() {
            eprintln!(
                "[airsnip] 显示分解：建窗+初始化 {:?}，SetWindowPos {pos:?}，首帧 {paint:?}，前台+焦点 {:?}",
                t_init_done - t_win,
                t_pos.elapsed() - pos - paint
            );
        }
        if std::env::var_os("AIRSNIP_TIMING").is_some() {
            eprintln!("[airsnip] 覆盖层创建到显示完成 {:?}", t_show.elapsed());
        }
        // M5 性能剖析：热键 → 可见总耗时（含抓屏、线程创建与首帧绘制）
        crate::perf::log_overlay_shown();
    }

    // 局部消息循环：WM_DESTROY 由 wndproc 调 PostQuitMessage(0) 结束（独立线程，
    // 不影响主线程消息队列）。退出后 `Box::from_raw` 释放 Overlay 及其 4 份全屏位图。
    unsafe {
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 == 0 || msg.message == WM_QUIT {
                break;
            }
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
            if msg.hwnd == hwnd && msg.message == WM_DESTROY {
                break;
            }
        }
    }

    // 记录本会话最后使用的工具（下次截图默认沿用，首次为矩形，M3）
    crate::perf::log_frames_summary();
    let overlay = unsafe { Box::from_raw(raw) };
    crate::settings::last_tool_store(overlay.tool());
    let result = overlay.result();
    drop(overlay);
    destroy();
    result
}
