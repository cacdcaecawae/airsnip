// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层 C ABI（macOS，每块显示器一个）：创建、事件转发、上屏缓冲借用与结果读取。
//! 所有函数只在主线程调用；`AsOverlay` 由 `airsnip_overlay_new` 创建、`airsnip_overlay_free` 释放，
//! 不得在宿主回调执行期间释放（`close` 回调里只能安排异步释放）。

use airsnip_core::capture::CapturedScreen;
use airsnip_core::overlay::geometry::SelRect;
use airsnip_core::overlay::ui::Ui;
use airsnip_core::overlay::{Mods, Overlay, OverlayResult};
use airsnip_core::settings::{self, Config, SavePrefs, ThemeSetting};
use airsnip_core::theme::{Theme, palette};

use crate::host::{AsHost, MacHost};
use crate::{AsImage, guard};

/// 不透明句柄。
pub struct AsOverlay(Overlay);

/// 修饰键位（`airsnip_overlay_set_mods` / `airsnip_overlay_key` 的 `mods` 参数）。
const MOD_SHIFT: u32 = 1;
const MOD_SPACE: u32 = 2;
const MOD_CMD: u32 = 4;

pub(crate) unsafe fn ov<'a>(p: *mut AsOverlay) -> Option<&'a mut Overlay> {
    unsafe { p.as_mut() }.map(|o| &mut o.0)
}

/// 由配置构造覆盖层。`backing_scale` 为显示器 backingScaleFactor（界面缩放的基准），
/// 手动倍率按其相对值生效（Retina 上「150%」= 3.0）；`dark` 为系统当前是否深色外观。
fn build(screen: CapturedScreen, backing_scale: f32, dark: bool, host: AsHost) -> Result<Overlay, String> {
    let cfg = Config::load();
    let theme = match cfg.theme {
        ThemeSetting::Follow if dark => Theme::Dark,
        ThemeSetting::Follow | ThemeSetting::Light => Theme::Light,
        ThemeSetting::Dark => Theme::Dark,
    };
    let ui = match cfg.ui_scale.factor() {
        Some(f) => Ui::from_scale(backing_scale * f),
        None => Ui::auto(backing_scale, screen.width, screen.height),
    };
    let save = SavePrefs {
        name_style: cfg.name_style,
        last_ext: cfg.last_save_ext.clone(),
        last_dir: cfg.last_save_dir.clone(),
    };
    Overlay::new(
        screen,
        palette(theme),
        cfg.tool_keys,
        cfg.hotkey.longshot_key,
        settings::last_tool(),
        save,
        ui,
        Box::new(MacHost(host, backing_scale)),
    )
}

/// 创建覆盖层：复制 `screen`（RGBA 预乘，显示器像素坐标），失败返回 NULL（详情见 `airsnip_last_error`）。
///
/// # Safety
/// `screen` 指向有效图像；`host` 指向有效回调表，其中 `ctx` 在覆盖层释放前保持有效。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_new(
    screen: *const AsImage,
    backing_scale: f32,
    dark: i32,
    host: *const AsHost,
) -> *mut AsOverlay {
    let mut out = std::ptr::null_mut();
    guard(|| {
        let pixmap = unsafe { crate::pixmap_from(screen) }?;
        let host = unsafe { host.as_ref() }.copied().ok_or("宿主回调表为空")?;
        let (width, height) = (pixmap.width(), pixmap.height());
        let screen = CapturedScreen {
            width,
            height,
            origin_x: 0,
            origin_y: 0,
            rgba: pixmap.take(),
        };
        let overlay = build(screen, backing_scale, dark != 0, host)?;
        out = Box::into_raw(Box::new(AsOverlay(overlay)));
        Ok(())
    });
    out
}

/// 释放覆盖层，并记住本次使用的工具、保存格式与目录（下次截图沿用，M3/OUT-2）。
///
/// # Safety
/// `p` 来自 `airsnip_overlay_new` 且未释放过；不得在该覆盖层的宿主回调内调用。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_free(p: *mut AsOverlay) {
    if p.is_null() {
        return;
    }
    guard(|| {
        let ov = unsafe { Box::from_raw(p) };
        // 多屏时每块屏各有一个覆盖层：只记有选区的那块（用户在那里选的工具），
        // 否则后释放的空白屏会把工具改回默认
        if ov.0.selection.is_some() {
            settings::last_tool_store(ov.0.tool());
        }
        drop(ov);
        let (ext, dir) = (settings::last_save_ext_take(), settings::last_save_dir_take());
        if ext.is_some() || dir.is_some() {
            let mut cfg = Config::load();
            cfg.last_save_ext = ext.or(cfg.last_save_ext);
            cfg.last_save_dir = dir.or(cfg.last_save_dir);
            cfg.save();
        }
        Ok(())
    });
}

/// 借用上屏缓冲（RGBA 预乘）：在下一次调用该覆盖层的函数前有效。`p` 为空时返回空图像。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_display(p: *mut AsOverlay) -> AsImage {
    match unsafe { ov(p) } {
        Some(o) => crate::host::as_image(&o.display),
        None => AsImage {
            data: std::ptr::null(),
            width: 0,
            height: 0,
            stride: 0,
        },
    }
}

/// 在覆盖层上执行一次操作（空句柄忽略，panic 转错误码）。
/// 仅供本文件的 `unsafe extern "C"` 函数调用：`p` 的有效性由它们的调用方保证。
pub(crate) fn with(p: *mut AsOverlay, f: impl FnOnce(&mut Overlay)) {
    if let Some(o) = unsafe { ov(p) } {
        guard(|| {
            f(o);
            Ok(())
        });
    }
}

fn mods_of(mods: u32) -> Mods {
    Mods {
        shift: mods & MOD_SHIFT != 0,
        space: mods & MOD_SPACE != 0,
    }
}

/// 更新修饰键状态（Shift / 空格），并按当前位置刷新光标（空格抓手）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_set_mods(p: *mut AsOverlay, mods: u32, x: i32, y: i32) {
    with(p, |o| {
        o.mods = mods_of(mods);
        o.apply_cursor_at(x, y);
    });
}

/// 左键按下（覆盖层像素坐标）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_mouse_down(p: *mut AsOverlay, x: i32, y: i32) {
    with(p, |o| o.on_lbutton_down(x, y));
}

/// 鼠标移动 / 拖动：交互、字体面板悬停与光标形状。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_mouse_move(p: *mut AsOverlay, x: i32, y: i32) {
    with(p, |o| {
        o.on_mouse_move(x, y);
        o.on_font_list_hover(x, y);
        o.apply_cursor_at(x, y);
    });
}

/// 左键弹起。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_mouse_up(p: *mut AsOverlay) {
    with(p, |o| o.on_lbutton_up());
}

/// 双击（EDT-3/EDT-4/M3）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_double_click(p: *mut AsOverlay, x: i32, y: i32) {
    with(p, |o| o.on_double_click(x, y));
}

/// 右键：取消截图（OUT-3）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_cancel(p: *mut AsOverlay) {
    with(p, |o| o.on_cancel());
}

/// 截图时屏幕上各窗口的矩形（CAP-6）：`rects` 为 `count` 组 x, y, w, h（覆盖层像素，前到后），开启窗口识别。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL；`rects` 为 NULL 或指向 `count * 4` 个 int32。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_set_windows(p: *mut AsOverlay, rects: *const i32, count: usize) {
    let flat = if rects.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(rects, count * 4) }
    };
    let list = flat
        .chunks_exact(4)
        .map(|c| SelRect {
            x: c[0],
            y: c[1],
            w: c[2],
            h: c[3],
        })
        .collect();
    with(p, |o| o.set_windows(list));
}

/// 鼠标离开本屏：撤掉窗口识别的悬停高亮（CAP-6）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_leave(p: *mut AsOverlay) {
    with(p, |o| o.clear_hover());
}

/// 贴选区并结束截图（PIN-1）：截图期间按贴图热键，同 Windows 截图时按 F3。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_pin(p: *mut AsOverlay) {
    with(p, |o| o.on_pin());
}

/// 滚轮一格（`steps` 为带符号格数，Swift 侧已把触控板连续滚动归并成格）；⌘ 时 3 倍步进。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_wheel(p: *mut AsOverlay, x: i32, y: i32, steps: i32, mods: u32) {
    with(p, |o| {
        for _ in 0..steps.unsigned_abs().min(20) {
            let delta: i16 = if steps > 0 { 120 } else { -120 };
            // 字体面板上的滚轮用于滚动列表（EDT-4），不调节字号
            if !o.on_font_list_wheel(x, y, delta) {
                o.on_mouse_wheel(x, y, delta, mods & MOD_CMD != 0);
            }
        }
    });
}

/// 按键（`kVK_*` 键码）；返回 1 表示已处理，0 交回 Swift（如文字输入）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_key(p: *mut AsOverlay, code: u32, mods: u32) -> i32 {
    let mut handled = false;
    with(p, |o| {
        handled = crate::keys::on_key(o, code, mods & MOD_CMD != 0, mods & MOD_SHIFT != 0)
    });
    handled as i32
}

/// 是否正在文本输入（EDT-4）：此时空格是字符而不是「按住拖动选区」。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_editing_text(p: *mut AsOverlay) -> i32 {
    unsafe { ov(p) }.is_some_and(|o| o.is_editing_text()) as i32
}

/// 当前是否有选区（多屏时决定哪块屏幕处于激活状态）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_has_selection(p: *mut AsOverlay) -> i32 {
    unsafe { ov(p) }.is_some_and(|o| o.selection.is_some()) as i32
}

/// 关闭后的结果：0 完成、1 取消、2 长截图（`viewport` 写入 x, y, w, h）。
///
/// # Safety
/// `p` 为有效覆盖层或 NULL；`viewport` 为 NULL 或指向 4 个 int32。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_overlay_result(p: *mut AsOverlay, viewport: *mut i32) -> i32 {
    let Some(o) = (unsafe { ov(p) }) else { return 1 };
    match o.result() {
        OverlayResult::Done => 0,
        OverlayResult::Cancelled => 1,
        OverlayResult::Viewport((x, y, w, h)) => {
            if !viewport.is_null() {
                let out = unsafe { std::slice::from_raw_parts_mut(viewport, 4) };
                out.copy_from_slice(&[x, y, w as i32, h as i32]);
            }
            2
        }
    }
}

/// 测试用：带记录宿主的覆盖层（不经 FFI；默认键位为 macOS 键码，只在 macOS 上编译）。
#[cfg(all(test, target_os = "macos"))]
pub(crate) mod testing {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use airsnip_core::capture::CapturedScreen;
    use airsnip_core::overlay::geometry::SelRect;
    use airsnip_core::overlay::ui::Ui;
    use airsnip_core::overlay::{Host, Overlay, Shape};
    use tiny_skia::Pixmap;

    /// 宿主调用记录。
    #[derive(Default)]
    pub(crate) struct Log {
        pub closed: u32,
        pub presents: u32,
        pub copied: Option<(u32, u32)>,
    }

    struct Recorder(Rc<RefCell<Log>>);

    impl Host for Recorder {
        fn present(&mut self, _: &Pixmap, _: SelRect) {
            self.0.borrow_mut().presents += 1;
        }
        fn set_cursor(&mut self, _: Shape, _: Ui) {}
        fn close(&mut self) {
            self.0.borrow_mut().closed += 1;
        }
        fn alert(&mut self, _: &str) {}
        fn copy_image(&mut self, img: Pixmap) {
            self.0.borrow_mut().copied = Some((img.width(), img.height()));
        }
        fn pin(&mut self, _: Pixmap, _: i32, _: i32) -> bool {
            false
        }
        fn pick_save_file(&mut self, _: Option<&Path>, _: &str, _: &str) -> Option<PathBuf> {
            None
        }
        fn save_image(&mut self, _: Pixmap, _: PathBuf) {}
        fn set_ime(&mut self, _: bool) {}
    }

    /// 纯灰屏幕的覆盖层（默认配置、深色主题、1x 缩放）。
    pub(crate) fn test_overlay(w: u32, h: u32) -> (Overlay, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let screen = CapturedScreen {
            width: w,
            height: h,
            origin_x: 0,
            origin_y: 0,
            rgba: vec![128; (w * h * 4) as usize],
        };
        let ov = Overlay::new(
            screen,
            airsnip_core::theme::palette(airsnip_core::theme::Theme::Dark),
            Default::default(),
            airsnip_core::settings::DEFAULT_LONGSHOT_KEY,
            airsnip_core::editor::Tool::Rect,
            Default::default(),
            Ui::from_scale(1.0),
            Box::new(Recorder(log.clone())),
        )
        .expect("测试覆盖层应能创建");
        (ov, log)
    }
}
