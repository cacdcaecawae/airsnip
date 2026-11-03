// SPDX-License-Identifier: GPL-3.0-only
//! macOS 覆盖层按键分发：与 Windows `overlay::wndproc` 的 `WM_KEYDOWN` 规则一致，
//! 编辑快捷键换成 ⌘（⌘C 复制、⌘S 保存、⌘Z 撤销、⌘⇧Z 重做、⌘A 全选整屏）。键码为 `kVK_*`。

use airsnip_core::overlay::Overlay;

const KEY_A: u32 = 0x00;
const KEY_S: u32 = 0x01;
const KEY_Z: u32 = 0x06;
const KEY_C: u32 = 0x08;
const KEY_RETURN: u32 = 0x24;
const KEY_DELETE: u32 = 0x33; // Backspace
const KEY_ESCAPE: u32 = 0x35;
const KEY_ENTER: u32 = 0x4C; // 小键盘回车
const KEY_HOME: u32 = 0x73;
const KEY_FORWARD_DELETE: u32 = 0x75;
const KEY_END: u32 = 0x77;
const KEY_LEFT: u32 = 0x7B;
const KEY_RIGHT: u32 = 0x7C;
const KEY_DOWN: u32 = 0x7D;
const KEY_UP: u32 = 0x7E;

/// 处理一次按键；返回是否已处理（未处理的交给 Swift 继续分发，如文字输入）。
pub(crate) fn on_key(ov: &mut Overlay, code: u32, cmd: bool, shift: bool) -> bool {
    if ov.is_editing_text() {
        return on_text_key(ov, code, cmd, shift);
    }
    match code {
        // Esc：先收起字体面板（EDT-4），否则取消截图
        KEY_ESCAPE if ov.font_list_open() => ov.close_font_list(),
        KEY_ESCAPE => ov.on_cancel(),
        KEY_RETURN | KEY_ENTER => ov.on_copy(),
        KEY_C if cmd => ov.on_copy(),
        KEY_A if cmd => ov.select_all(), // 全选整屏（CAP-7）
        KEY_S if cmd => ov.on_save(),
        KEY_Z if cmd && shift => ov.on_redo(),
        KEY_Z if cmd => ov.on_undo(),
        // 方向键微调（EDT-7 / CAP-2）：1px，Shift 5px；优先于工具切换键
        KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN => {
            let s = if shift { 5 } else { 1 };
            let (dx, dy) = match code {
                KEY_LEFT => (-s, 0),
                KEY_RIGHT => (s, 0),
                KEY_UP => (0, -s),
                _ => (0, s),
            };
            ov.nudge(dx, dy);
        }
        _ if cmd => return false,
        // 长截图键（默认 S）：当前选区作为滚动视口（M4）
        k if k == ov.longshot_key() => ov.on_longshot(),
        // 工具切换键（M3）：任意键都查映射，设置里可改成字母等
        k => match ov.tool_by_key(k) {
            Some(t) => ov.switch_tool(t),
            None => return false,
        },
    }
    true
}

/// 文本输入中的编辑键：映射到核心使用的 Windows 虚拟键码语义（`Overlay::on_text_key`）。
/// 字符本身经 `NSTextInputClient` 送入，不在这里处理。
fn on_text_key(ov: &mut Overlay, code: u32, cmd: bool, shift: bool) -> bool {
    let vk = match code {
        KEY_DELETE => 0x08,
        KEY_FORWARD_DELETE => 0x2E,
        // ⌘← / ⌘→：行首 / 行尾（Mac 键盘没有 Home / End）
        KEY_LEFT if cmd => 0x24,
        KEY_RIGHT if cmd => 0x23,
        KEY_LEFT => 0x25,
        KEY_RIGHT => 0x27,
        KEY_HOME => 0x24,
        KEY_END => 0x23,
        KEY_RETURN | KEY_ENTER => 0x0D,
        KEY_ESCAPE => 0x1B,
        KEY_Z if cmd => 0x5A,
        _ => return false,
    };
    ov.on_text_key(vk, shift, cmd)
}

// 默认工具键是 macOS 键码，测试只在 macOS 上有意义
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::overlay::testing::test_overlay;
    use airsnip_core::overlay::OverlayResult;

    #[test]
    fn escape_cancels_and_return_copies() {
        let (mut ov, log) = test_overlay(64, 48);
        assert!(on_key(&mut ov, KEY_ESCAPE, false, false));
        assert_eq!(ov.result(), OverlayResult::Cancelled);
        assert_eq!(log.borrow().closed, 1);

        let (mut ov, log) = test_overlay(64, 48);
        assert!(on_key(&mut ov, KEY_RETURN, false, false));
        assert_eq!(ov.result(), OverlayResult::Done);
        assert_eq!(log.borrow().closed, 1);
    }

    /// ⌘ 组合只认编辑快捷键：⌘S 是保存而不是长截图键 S，其余 ⌘ 组合不吞掉。
    #[test]
    fn cmd_combos_do_not_trigger_plain_keys() {
        let (mut ov, log) = test_overlay(64, 48);
        assert!(!on_key(&mut ov, 0x12, true, false), "⌘1 不应切工具");
        assert!(on_key(&mut ov, KEY_S, true, false), "⌘S 应处理（无选区时保存为空操作）");
        assert_eq!(log.borrow().closed, 0, "无选区保存不应关闭覆盖层");
    }

    /// M2 闭环：拖出选区 → 回车复制，复制的图像尺寸等于选区（覆盖层像素）。
    #[test]
    fn drag_select_then_return_copies_selection() {
        let (mut ov, log) = test_overlay(200, 120);
        ov.on_lbutton_down(10, 20);
        ov.on_mouse_move(70, 60);
        ov.on_lbutton_up();
        assert!(ov.selection.is_some(), "拖动后应有选区");
        assert!(log.borrow().presents > 0, "框选过程应上屏");
        assert!(on_key(&mut ov, KEY_RETURN, false, false));
        assert_eq!(log.borrow().copied, Some((60, 40)));
        assert_eq!(ov.result(), OverlayResult::Done);
    }

    fn sel(ov: &Overlay) -> Option<(i32, i32, i32, i32)> {
        ov.selection.map(|s| (s.x, s.y, s.w, s.h))
    }

    /// 窗口识别（CAP-6/7）：单击（含小抖动）选中鼠标下的窗口；空白处单击 = 整屏；
    /// 拖出容差仍是手动框选；⌘A 全选整屏；未提供窗口列表时单击不出选区（旧行为）。
    #[test]
    fn click_selects_window_or_full_screen() {
        use airsnip_core::overlay::geometry::SelRect;
        let win = |ov: &mut Overlay| {
            ov.set_windows(vec![SelRect {
                x: 50,
                y: 40,
                w: 120,
                h: 80,
            }])
        };

        let (mut ov, _) = test_overlay(400, 300);
        win(&mut ov);
        ov.on_mouse_move(60, 50);
        ov.on_lbutton_down(60, 50);
        ov.on_mouse_move(62, 53);
        ov.on_lbutton_up();
        assert_eq!(sel(&ov), Some((50, 40, 120, 80)), "单击窗口应选中整个窗口");

        let (mut ov, _) = test_overlay(400, 300);
        win(&mut ov);
        ov.on_lbutton_down(300, 250);
        ov.on_lbutton_up();
        assert_eq!(sel(&ov), Some((0, 0, 400, 300)), "空白处单击应选中整屏");

        let (mut ov, _) = test_overlay(400, 300);
        win(&mut ov);
        ov.on_lbutton_down(60, 50);
        ov.on_mouse_move(160, 150);
        ov.on_lbutton_up();
        assert_eq!(sel(&ov), Some((60, 50, 100, 100)), "拖动应手动框选");

        assert!(on_key(&mut ov, KEY_A, true, false));
        assert_eq!(sel(&ov), Some((0, 0, 400, 300)), "⌘A 应全选整屏");

        let (mut ov, _) = test_overlay(400, 300);
        ov.on_lbutton_down(60, 50);
        ov.on_lbutton_up();
        assert_eq!(sel(&ov), None, "没有窗口列表时单击不出选区");
    }

    #[test]
    fn tool_keys_switch_and_unknown_keys_pass_through() {
        let (mut ov, _) = test_overlay(64, 48);
        assert!(on_key(&mut ov, 0x13, false, false), "数字 2 应切到箭头");
        assert_eq!(ov.tool(), airsnip_core::editor::Tool::Arrow);
        assert!(!on_key(&mut ov, 0x00, false, false), "A 不是工具键，应交回 Swift");
    }
}
