// SPDX-License-Identifier: GPL-3.0-only
//! 长截图（LNG-1/3/4/5/6/7/10/11）：框选滚动视口 → 用户手动滚动 → 条带拼接 → 输出。
//!
//! 模块分工：拼接算法与会话状态机在 `airsnip_core::longshot`（`stitch`/`rows`/`engine`/`session`/`debug`），
//! 本模块是 Windows 平台层：`ScreenSource` 屏幕帧源、`window` 执行窗口、`bar` 状态条绘制、
//! `present` 上屏、`border` 视口边框、`preview` 拼接结果预览窗。

mod bar;
mod border;
mod events;
mod present;
mod preview;
mod window;

use airsnip_core::longshot::session;
use airsnip_core::longshot::session::Source;

/// 真实帧源：屏幕区域抓取（GDI）。
struct ScreenSource {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

impl Source for ScreenSource {
    fn capture(&mut self) -> Result<Vec<u8>, String> {
        let screen = crate::capture::capture_region(self.x, self.y, self.w, self.h)?;
        if screen.rgba.len() != (self.w * self.h * 4) as usize {
            return Err("抓帧尺寸不符".into());
        }
        Ok(screen.rgba)
    }
}

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK};

/// 长截图主流程：创建执行窗口阻塞运行，完成后复制到剪贴板并保存。
/// 保存目标：用户点过「另存为」则写到所选路径；否则自动存到默认目录（PNG）。
/// 两种情况都会复制到剪贴板（OUT-1）。
pub fn run(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    hinstance: HINSTANCE,
    max_height: u32,
    palette: &'static crate::theme::Palette,
    ui_scale: crate::settings::UiScale,
    save: crate::settings::SavePrefs,
) {
    let Some(outcome) = window::run(x, y, w, h, hinstance, max_height, palette, ui_scale, save) else {
        return; // 取消或失败（窗口内已提示）
    };
    let window::Outcome { pixmap, save_as } = outcome;
    // 输出：复制 + 保存（与普通截图 Enter / Ctrl+S 行为一致，OUT-1/OUT-2）
    std::thread::spawn(move || {
        if let Err(e) = crate::output::copy_to_clipboard(&pixmap) {
            crate::app::message_box(None, &format!("复制失败：{e}"), MB_OK | MB_ICONERROR);
        }
        // 另存为：用户已选好路径与格式，不再额外往默认目录写一份
        let path = match save_as {
            Some(path) => path,
            None => {
                let dir = crate::output::default_save_dir();
                let _ = std::fs::create_dir_all(&dir);
                dir.join(crate::output::timestamped_filename("png"))
            }
        };
        if let Err(e) = crate::output::save_file(&pixmap, &path) {
            crate::app::message_box(None, &format!("保存失败：{e}"), MB_OK | MB_ICONERROR);
        }
    });
}
