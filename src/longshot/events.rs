// SPDX-License-Identifier: GPL-3.0-only
//! 长截图执行窗口的消息处理（LNG-1）：按钮/键位分发、完成与另存为。
//! 从 `window` 拆出，保证单文件不超过 300 行；窗口创建与 `Win` 定义仍在 `window`。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::bar::Button;
use super::present::{ctrl_down, redraw, win_from};
use super::window::{HK_ENTER, HK_ESC, HK_SAVE, HK_UNDO, TIMER_ID, Win};

/// 完成：先补抓一帧（用户滚动后立刻按 Enter 时，最后一屏还没被定时拍捕获），再收尾。
fn finish(win: &mut Win) {
    win.session.tick();
    win.session.finished = true;
    win.done = true;
}

/// 另存为（LNG/OUT-2）：停下拼接 → 弹系统另存为对话框 → 记录路径。
/// 用户取消时恢复拼接（已拼内容仍在画布上，可以继续滚动）。
fn save_as(win: &mut Win, hwnd: HWND) {
    // 同 Enter：补抓最后一帧，否则最后一屏可能没拼进去
    win.session.tick();
    // 抓屏失败（画布不可用）：直接结束，不弹对话框（选了路径也存不出东西）
    if win.session.cancelled {
        win.done = true;
        return;
    }
    win.session.finished = true;
    // 预填文件名按设置里的命名模板；格式用上次保存的（首次为 PNG）
    let ext = win.save.last_ext.as_deref().unwrap_or("png").to_string();
    let name = format!("{}.{ext}", crate::output::base_name(win.save.name_style));
    // 初始目录：上次保存过的目录优先，否则默认截图目录；不存在时交给系统
    let dir = win
        .save
        .last_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(crate::output::default_save_dir);
    let dir = dir.is_dir().then_some(dir);
    match crate::app::filedlg::pick_save_file(hwnd, dir.as_deref(), &name, &ext) {
        Some(path) => {
            // 记住本次格式与目录：下次对话框默认选中/定位（OUT-2）
            if let Some(e) = path.extension().and_then(|e| e.to_str()) {
                crate::settings::last_save_ext_store(e);
            }
            if let Some(parent) = path.parent() {
                crate::settings::last_save_dir_store(parent);
            }
            win.save_as = Some(path);
            win.done = true;
        }
        // 取消：继续拼接（`done` 仍为 false，窗口留着）
        None => win.session.finished = false,
    }
}

/// 窗口过程：定时抓帧拼接，按钮/键位驱动完成、另存为、撤销与取消。
/// 窗口过程：定时抓帧拼接，按钮/键位驱动完成、另存为、撤销与取消。
pub(super) unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DESTROY => {
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(win) = win_from(hwnd) {
                let changed = win.session.tick();
                if changed {
                    win.dirty = true;
                }
                if win.dirty {
                    redraw(hwnd);
                }
                if win.done {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(win) = win_from(hwnd) {
                let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                match win.bar.hit(x, y) {
                    Some(Button::Undo) => win.session.undo(),
                    // 另存为对话框是模态的，期间 WM_TIMER 仍会进来：靠 `done` 避免提前关窗
                    Some(Button::SaveAs) => save_as(win, hwnd),
                    Some(Button::Done) => finish(win),
                    Some(Button::Cancel) => {
                        win.session.cancelled = true;
                        win.done = true;
                    }
                    None => {}
                }
                win.dirty = true;
                redraw(hwnd);
                if win.done {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if let Some(win) = win_from(hwnd) {
                match wparam.0 as u32 {
                    0x0D => finish(win),
                    0x1B => {
                        win.session.cancelled = true;
                        win.done = true;
                    }
                    0x5A if ctrl_down() => win.session.undo(),
                    0x53 if ctrl_down() => save_as(win, hwnd),
                    _ => {}
                }
                win.dirty = true;
                redraw(hwnd);
                if win.done {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            if let Some(win) = win_from(hwnd) {
                match wparam.0 as i32 {
                    HK_ENTER => finish(win),
                    HK_ESC => {
                        win.session.cancelled = true;
                        win.done = true;
                    }
                    HK_UNDO => win.session.undo(),
                    HK_SAVE => save_as(win, hwnd),
                    _ => {}
                }
                win.dirty = true;
                redraw(hwnd);
                if win.done {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
