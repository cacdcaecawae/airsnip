// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层初始化（CAP-2/3）：原图 / 暗图 / 上屏图三份像素缓冲，
//! 以及运行时字段的初始值。从 `mod.rs` 拆出，控制单文件规模。

use tiny_skia::Pixmap;

use crate::capture::CapturedScreen;
use crate::editor::{self, Document, Tool};
use crate::theme::Palette;

use super::interaction::Phase;
use super::surface;
use super::{Host, Mods, Overlay};

impl Overlay {
    /// 由抓屏结果创建覆盖层；`ui` 为界面缩放（平台层按显示器解析），`host` 提供平台能力。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut capture: CapturedScreen,
        palette: &'static Palette,
        tool_keys: crate::settings::ToolKeys,
        longshot_key: u32,
        default_tool: Tool,
        save: crate::settings::SavePrefs,
        ui: super::ui::Ui,
        host: Box<dyn Host>,
    ) -> Result<Self, String> {
        let (w, h) = (capture.width, capture.height);
        let Some(size) = tiny_skia::IntSize::from_wh(w, h) else {
            return Err("虚拟屏幕尺寸非法".into());
        };
        let rgba = std::mem::take(&mut capture.rgba);
        let original = Pixmap::from_vec(rgba, size).ok_or("创建原图像素图失败")?;
        let timing = crate::perf::enabled();
        let t_init = std::time::Instant::now();
        let mut dimmed = original.clone();
        let t_clone1 = t_init.elapsed();
        // 暗化（CAP-3：预生成暗图）：覆盖黑色 alpha=120，多线程分块整数乘法
        surface::dim_pixmap(&mut dimmed, 255 - 120);
        let t_dim = t_init.elapsed();
        let display = dimmed.clone();
        if timing {
            eprintln!(
                "[airsnip] 初始化：原图 clone {t_clone1:?}，暗化 {:?}，暗图 clone {:?}",
                t_dim - t_clone1,
                t_init.elapsed() - t_dim
            );
        }

        Ok(Self {
            capture,
            original,
            dimmed,
            display,
            palette,
            ui,
            selection: None,
            windows: None,
            hover: None,
            mods: Mods::default(),
            first_paint: true,
            phase: Phase::Adjusting,
            drag: None,
            doc: Document::new(),
            tool: default_tool, // 记住上次工具（M3），首次为矩形
            tool_keys,
            longshot_key,
            save,
            color_index: 1, // 默认红色
            line_width: editor::DEFAULT_LINE_WIDTH,
            blur_radius: editor::DEFAULT_BLUR_RADIUS,
            font_size: editor::DEFAULT_FONT_SIZE,
            font_family: std::sync::Arc::from(editor::DEFAULT_FONT_FAMILY),
            bold: false,
            italic: false,
            font_list: None,
            filled: false,
            draft: None,
            down_started_draw: false,
            selected: None,
            ctrl_drag: None,
            obj_drag: None,
            text_edit: None,
            bar: None,
            cancelled: false,
            viewport: None,
            host,
        })
    }
}
