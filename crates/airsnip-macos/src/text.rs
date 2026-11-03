// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层文字输入 C ABI（EDT-4）：Swift 的 NSTextInputClient 把输入法组合串与上屏文字送进核心，
//! 并取光标位置给候选框定位。编辑键（退格、回车、方向、Esc）仍走 `airsnip_overlay_key`。

use std::ffi::{CStr, c_char};

use airsnip_core::overlay::Overlay;

use crate::overlay::{AsOverlay, ov, with};

/// 输入法文字：`commit` 非 0 为上屏（插入光标处，控制字符丢弃），0 为组合串（空串 = 组合结束）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL；`text` 为 NUL 结尾 UTF-8 或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_text(p: *mut AsOverlay, text: *const c_char, commit: i32) {
    if text.is_null() {
        return;
    }
    let s = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    with(p, |o| apply(o, &s, commit != 0));
}

fn apply(o: &mut Overlay, s: &str, commit: bool) {
    if commit {
        o.on_text_composition(Some(s.chars().filter(|c| !c.is_control()).collect()), None);
    } else if s.is_empty() {
        o.on_text_composition_end();
    } else {
        o.on_text_composition(None, Some(s.to_owned()));
    }
}

/// 输入中的光标竖线（像素，左上原点）写入 `out`：x, y, 高；不在输入中返回 0。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL；`out` 为 NULL 或指向 3 个 int32。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_caret(p: *mut AsOverlay, out: *mut i32) -> i32 {
    let Some((x, y, h)) = unsafe { ov(p) }.and_then(|o| o.text_caret()) else {
        return 0;
    };
    if out.is_null() {
        return 0;
    }
    let out = unsafe { std::slice::from_raw_parts_mut(out, 3) };
    out.copy_from_slice(&[x as i32, y as i32, h.ceil() as i32]);
    1
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::keys::on_key;
    use crate::overlay::testing::test_overlay;
    use airsnip_core::editor::Tool;

    /// 拼音组合 → 选词上屏 → 回车提交 → 双击再编辑，光标停在原文字末尾。
    #[test]
    fn ime_composition_commit_and_reedit() {
        let (mut ov, _) = test_overlay(800, 600);
        ov.on_lbutton_down(100, 100);
        ov.on_mouse_move(500, 300);
        ov.on_lbutton_up();
        ov.switch_tool(Tool::Text);
        ov.on_lbutton_down(150, 150);
        ov.on_lbutton_up();
        let (x0, _, h) = ov.text_caret().expect("点击选区内应进入文字输入");
        assert!(h > 0.0);

        apply(&mut ov, "ni", false);
        let (x_comp, _, _) = ov.text_caret().unwrap();
        assert!(x_comp > x0, "组合串应推动光标");

        apply(&mut ov, "你好", true);
        let (x_done, _, _) = ov.text_caret().unwrap();
        assert!(x_done > x0, "上屏文字应推动光标");

        assert!(on_key(&mut ov, 0x24, false, false), "回车应提交");
        assert!(ov.text_caret().is_none(), "提交后不在输入中");

        ov.on_double_click(155, 155);
        let (x_again, _, _) = ov.text_caret().expect("双击文字应再编辑");
        assert_eq!(x_again, x_done, "再编辑时光标在原文字末尾");
    }
}
