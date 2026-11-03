// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层窗口过程：消息分发、修饰键状态（Ctrl/Shift/空格）。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::UI::Input::Ime::{GCS_COMPSTR, GCS_RESULTSTR, IME_COMPOSITION_STRING, ImmGetVirtualKey};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::cursors::apply_now;
use super::geometry::SelRect;
use super::{Mods, Overlay};

/// 自定义消息：显示后异步设置前台与输入焦点。
/// `SetForegroundWindow` 在窗口刚显示时同步调用会阻塞至 ~45ms（前台锁定），
/// 改到消息循环里执行，让画面先出现（M5 性能）。
pub(super) const WM_FOCUS_OVERLAY: u32 = WM_APP + 1;

pub(super) unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // 窗口销毁（DestroyWindow 同步发送，不经消息队列）：必须 PostQuitMessage(0)
    // 让消息循环的 GetMessageW 返回 0 并退出，否则覆盖层线程永久阻塞在消息循环，
    // Overlay（含 4 份全屏位图）永不释放，每次截图都泄漏一份（实测每张 +57MB）。
    if msg == WM_DESTROY {
        unsafe {
            let _ = PostQuitMessage(0);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    let Some(overlay) = app_from(hwnd) else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    // 修饰键状态在每条消息处理前刷新（核心逻辑只读 `overlay.mods`）
    // 空格：按住拖动可移动整个选区（CAP-2）
    overlay.mods = Mods {
        shift: shift_down(),
        space: key_down(0x20),
    };
    match msg {
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            overlay.on_lbutton_down(x, y);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            overlay.on_mouse_move(x, y);
            // 字体面板悬停高亮（EDT-4）
            overlay.on_font_list_hover(x, y);
            // 光标随鼠标位置变：手柄→方向光标、对象/曲线控制柄→移动光标（否则系统只在
            // 位置变化且命中窗口时发 WM_SETCURSOR，拖动手柄途中光标不会更新）
            overlay.apply_cursor_at(x, y);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            overlay.on_lbutton_up();
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            // lparam 为屏幕坐标（非客户区），转覆盖层坐标由 on_mouse_wheel 完成
            let sx = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
            let sy = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16; // ±120 单位
            // 字体面板上的滚轮用于滚动列表（EDT-4），不调节字号
            let (ox, oy) = (sx - overlay.capture.origin_x, sy - overlay.capture.origin_y);
            if !overlay.on_font_list_wheel(ox, oy, delta) {
                overlay.on_mouse_wheel(sx, sy, delta, ctrl_down());
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            overlay.on_cancel();
            LRESULT(0)
        }
        // 双击：命中对象 → 编辑文本/恢复直线（EDT-3/EDT-4）；
        // 否则选区空白内双击 = 完成截图（复制，M3，替代回车）
        WM_LBUTTONDBLCLK => {
            let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            overlay.on_double_click(x, y);
            LRESULT(0)
        }
        // 主动绘制，不靠系统擦除；只重绘无效区域
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                let _ = BeginPaint(hwnd, &mut ps);
            }
            let rc = ps.rcPaint;
            let w = overlay.display.width() as i32;
            let h = overlay.display.height() as i32;
            let x0 = rc.left.clamp(0, w);
            let y0 = rc.top.clamp(0, h);
            let x1 = rc.right.clamp(0, w);
            let y1 = rc.bottom.clamp(0, h);
            if x1 > x0 && y1 > y0 {
                overlay.present_region(SelRect {
                    x: x0,
                    y: y0,
                    w: x1 - x0,
                    h: y1 - y0,
                });
            }
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // 显示后的异步焦点：避开 SetForegroundWindow 的前台锁定阻塞
        WM_FOCUS_OVERLAY => {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(hwnd));
            }
            LRESULT(0)
        }
        // 光标形状随鼠标位置决定（手柄方向 / 对象移动 / 十字），见 `cursor::cursor_at`
        WM_SETCURSOR => {
            apply_now(overlay, hwnd);
            LRESULT(1)
        }
        WM_KEYDOWN => {
            // 文本输入中：只处理输入相关按键（光标移动/退格/Enter 提交/Esc 取消），
            // 拦截全局快捷键与空格抓手切换（空格作为普通字符经 WM_CHAR 进入文本）
            if overlay.is_editing_text() {
                overlay.on_text_key(wparam.0 as u32, shift_down(), ctrl_down());
                return LRESULT(0);
            }
            // 中文输入法活动时，按键会被改写为 VK_PROCESSKEY(0xE5)，真实键另存。
            // 不禁用 IME（文本工具需要），改用 ImmGetVirtualKey 取回真实 vk，
            // 否则 IME 开启状态下按 S/F3/回车等全部失效（M4 实测缺陷）。
            let vk = if wparam.0 as u32 == 0xE5 {
                unsafe { ImmGetVirtualKey(hwnd) }
            } else {
                wparam.0 as u32
            };
            // 空格按下立即刷新光标：系统只在鼠标移动时发 WM_SETCURSOR，不能等它
            if vk == 0x20 {
                apply_now(overlay, hwnd);
                return LRESULT(0);
            }
            match vk {
                // Esc：先收起字体面板（EDT-4），否则取消截图
                0x1B => {
                    if overlay.font_list_open() {
                        overlay.close_font_list();
                    } else {
                        overlay.on_cancel();
                    }
                }
                0x72 => overlay.on_pin(),  // PIN-1：F3 贴选区（截图期间全局 F3 已撤销）
                0x0D => overlay.on_copy(), // Enter
                0x43 if ctrl_down() => overlay.on_copy(), // Ctrl+C
                0x53 if ctrl_down() => overlay.on_save(), // Ctrl+S 保存（弹另存为对话框）
                // 长截图键（默认 S，设置面板可改）：当前选区作为滚动视口（M4）
                k if k == overlay.longshot_key() => overlay.on_longshot(),
                0x5A if ctrl_down() => {
                    // Ctrl+Z 撤销 / Ctrl+Shift+Z 重做（EDT-7）
                    if shift_down() {
                        overlay.on_redo()
                    } else {
                        overlay.on_undo()
                    }
                }
                0x59 if ctrl_down() => overlay.on_redo(), // Ctrl+Y
                _ => {
                    // 方向键微调（EDT-7 / CAP-2）：1px，Shift 5px；优先于工具切换键
                    if let Some((dx, dy)) = crate::app::hotkey::arrow_step(vk, shift_down()) {
                        overlay.nudge(dx, dy);
                        return LRESULT(0);
                    }
                    // 工具切换键（M3）：任意键都查映射（不限于数字键），
                    // 以便设置面板可把工具键改为字母等
                    if let Some(t) = overlay.tool_by_key(vk) {
                        overlay.switch_tool(t);
                    }
                }
            }
            LRESULT(0)
        }
        // 文本字符（EDT-4）：普通字符经 TranslateMessage 转 WM_CHAR；IME 候选经组合消息
        WM_CHAR => {
            if overlay.is_editing_text() {
                overlay.on_text_char(wparam.0 as u16);
            }
            LRESULT(0)
        }
        // 中文 IME（EDT-4）：组合开始无需处理；组合更新读取候选/结果；结束清空候选
        WM_IME_STARTCOMPOSITION => LRESULT(0),
        WM_IME_COMPOSITION => {
            if overlay.is_editing_text() {
                // 有结果串则提交，否则读组合串（候选）
                let flags = lparam.0 as u32;
                let result = (flags & GCS_RESULTSTR.0 != 0).then(|| ime_string(hwnd, GCS_RESULTSTR));
                let comp = (result.is_none() && flags & GCS_COMPSTR.0 != 0).then(|| ime_string(hwnd, GCS_COMPSTR));
                overlay.on_text_composition(result, comp);
            }
            LRESULT(0)
        }
        WM_IME_ENDCOMPOSITION => {
            if overlay.is_editing_text() {
                overlay.on_text_composition_end();
            }
            LRESULT(0)
        }
        WM_KEYUP => {
            // 空格松开：光标回到按当前位置判定的形状
            if wparam.0 as u32 == 0x20 {
                apply_now(overlay, hwnd);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn app_from(hwnd: HWND) -> Option<&'static mut Overlay> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr == 0 {
            None
        } else {
            Some(&mut *(ptr as *mut Overlay))
        }
    }
}

/// 修饰键是否按下（`GetKeyState` 高位为 1 表示按下）。
fn key_down(vk: i32) -> bool {
    unsafe { GetKeyState(vk) as u16 & 0x8000 != 0 }
}

fn ctrl_down() -> bool {
    key_down(0x11)
}

/// Shift 是否按下（文本输入修饰键 / 绘制约束 EDT-1·EDT-2）。
fn shift_down() -> bool {
    key_down(0x10)
}

/// 从 IME 上下文读取组合串/结果串（UTF-16 → String，EDT-4）。
fn ime_string(hwnd: HWND, what: IME_COMPOSITION_STRING) -> String {
    unsafe {
        use windows::Win32::UI::Input::Ime::{ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext};
        let ctx = ImmGetContext(hwnd);
        if ctx.0.is_null() {
            return String::new();
        }
        let len = ImmGetCompositionStringW(ctx, what, None, 0);
        let out = if len <= 0 {
            String::new()
        } else {
            let mut buf = vec![0u16; (len / 2) as usize];
            ImmGetCompositionStringW(ctx, what, Some(buf.as_mut_ptr() as *mut core::ffi::c_void), len as u32);
            String::from_utf16_lossy(&buf)
        };
        let _ = ImmReleaseContext(hwnd, ctx);
        out
    }
}
