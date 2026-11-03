// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层（CAP-2/3）：冻结画面 + 选区暗化（预生成暗图 + 选区回贴原图），支持框选、移动与八向缩放。
//! 选区确认后进入标注模式（EDT-1/EDT-2/EDT-7）：工具栏切换工具与样式，在选区内拖拽即绘制标注。
//!
//! 本模块与平台无关：窗口、上屏、光标、剪贴板、对话框等经 [`Host`] 交给平台层
//! （Windows 见根包 `overlay`，macOS 见 `crates/airsnip-macos`）。坐标为覆盖层像素坐标。
//!
//! 文件分工：`init` 初始化、`geometry` 矩形运算、`surface` 像素缓冲、`cursor` 光标形状、
//! `selection` 脏区重绘、`interaction` 鼠标交互、`pick` 对象选中与控制柄、`objedit` 对象手柄与拖动缩放、
//! `font` 字体设置与字体面板、`annotate` 标注与输出、`toolbar` 工具栏、`ui` 界面缩放。

mod annotate;
mod control;
pub mod cursor;
mod font;
pub mod geometry;
mod init;
mod interaction;
mod objedit;
mod pick;
mod selection;
pub mod shape;
mod snap;
pub mod surface;
mod text_edit_state;
mod textinput;
pub mod toolbar;
pub mod ui;

use std::path::{Path, PathBuf};

use tiny_skia::Pixmap;

use crate::capture::CapturedScreen;
use crate::editor::{Document, Object, Point, Tool};
use crate::theme::Palette;

pub use cursor::Shape;
use geometry::SelRect;
use interaction::{Drag, Phase};
use text_edit_state::TextEdit;

/// 覆盖层运行结果（M4 长截图视口选取复用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayResult {
    /// 截图完成（已复制/保存/贴图）。
    Done,
    /// 用户取消。
    Cancelled,
    /// 长截图视口选取完成：屏幕坐标 (x, y, w, h)。
    Viewport((i32, i32, u32, u32)),
}

/// 修饰键状态：由平台层在分发每个输入事件前写入（Windows 读 `GetKeyState`，macOS 读事件标志）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    /// Shift：绘制与缩放约束（EDT-1/EDT-2/EDT-7）。
    pub shift: bool,
    /// 空格：按住拖动整个选区（CAP-2）。
    pub space: bool,
}

/// 覆盖层所需的平台能力。所有方法都在覆盖层所在线程上同步调用。
pub trait Host {
    /// 把 `display` 的 `r` 区域上屏。
    fn present(&mut self, display: &Pixmap, r: SelRect);
    /// 设置光标形状（自绘光标按 `ui` 缩放）。
    fn set_cursor(&mut self, shape: Shape, ui: ui::Ui);
    /// 结束覆盖层（结果由 [`Overlay::result`] 给出）；实现方不得在本调用内释放覆盖层。
    fn close(&mut self);
    /// 错误提示。
    fn alert(&mut self, msg: &str);
    /// 复制到剪贴板（实现方可异步执行，失败自行提示）。
    fn copy_image(&mut self, img: Pixmap);
    /// 贴图（PIN-1）：选区图及其在覆盖层中的左上角；返回 false 表示失败（覆盖层保留）。
    fn pin(&mut self, img: Pixmap, x: i32, y: i32) -> bool;
    /// 原生「另存为」对话框；`None` 表示取消。
    fn pick_save_file(&mut self, dir: Option<&Path>, name: &str, ext: &str) -> Option<PathBuf>;
    /// 编码并写入文件（实现方可异步执行，失败必须提示）。
    fn save_image(&mut self, img: Pixmap, path: PathBuf);
    /// 输入法开关：文本输入期间开启，其余时间关闭（EDT-4）。
    fn set_ime(&mut self, enabled: bool);
}

/// 覆盖层运行时状态（完整实现分散在各子模块的 `impl Overlay` 中）。
pub struct Overlay {
    pub capture: CapturedScreen,
    pub original: Pixmap,
    pub dimmed: Pixmap,
    pub display: Pixmap,
    /// 首帧标记：首帧 `display` 已是暗图，跳过回贴（M5 性能）。
    pub first_paint: bool,
    /// 当前主题调色板（截图开始时由设置解析，M3）。
    pub palette: &'static Palette,
    /// 界面缩放（按显示器 DPI，CAP-5）：工具栏、手柄、尺寸提示与光标随之缩放。
    pub ui: ui::Ui,
    pub selection: Option<SelRect>,
    /// 窗口识别（CAP-6）：截图时屏幕上各窗口的矩形（覆盖层像素，前到后）；`None` = 平台未提供，功能关闭。
    windows: Option<Vec<SelRect>>,
    /// 还没有选区时鼠标下识别到的窗口（或整屏），单击即选中（CAP-6/7）。
    hover: Option<SelRect>,
    /// 修饰键状态（平台层在分发输入事件前写入）。
    pub mods: Mods,
    phase: Phase,
    drag: Option<Drag>,
    /// 标注文档（对象列表 + 撤销/重做，EDT-1/EDT-2/EDT-7）。
    doc: Document,
    /// 当前工具与样式（EDT-7）。
    tool: Tool,
    /// 工具切换键配置（M3：数字键 1-6 切换标注工具；平台原生键码）。
    tool_keys: crate::settings::ToolKeys,
    /// 长截图键（覆盖层内按下，默认 S；设置面板可改；平台原生键码）。
    longshot_key: u32,
    /// 保存偏好（OUT-2）：文件名模板、上次格式与上次目录。
    save: crate::settings::SavePrefs,
    color_index: usize,
    /// 新建对象的默认线宽（滚轮 / 二级工具栏可连续调整）。
    line_width: f32,
    /// 新建模糊对象的默认半径（EDT-5）。
    blur_radius: f32,
    /// 新建文本对象的默认字号（EDT-4，滚轮 / 第二行可调）。
    font_size: f32,
    /// 新建文本对象的默认字体族、粗体、斜体（EDT-4，第二行工具栏可改）。
    font_family: std::sync::Arc<str>,
    bold: bool,
    italic: bool,
    /// 展开中的字体列表面板（EDT-4）。
    font_list: Option<toolbar::fontlist::FontPanel>,
    filled: bool,
    /// 正在拖拽、尚未提交的标注对象。
    draft: Option<Object>,
    /// 上次左键按下是否启动了选区内空白绘制（双击完成截图判据，M3）。
    /// 不用 `draft.is_some()`：双击第一击的草稿会在弹起时被提交/丢弃。
    pub down_started_draw: bool,
    /// 当前选中的对象索引（EDT-7）；控制柄仅在选中时显示（EDT-3）。
    selected: Option<usize>,
    /// 正在拖动的曲线控制柄：(控制点索引, 按下时的坐标)（EDT-3）。
    ctrl_drag: Option<(usize, Point)>,
    /// 正在拖动的选中对象（EDT-7 移动/缩放）。
    obj_drag: Option<objedit::ObjDrag>,
    /// 文本输入中状态（EDT-4）。
    text_edit: Option<TextEdit>,
    /// 当前工具栏布局（选区存在时才有）。
    bar: Option<toolbar::Toolbar>,
    pub cancelled: bool,
    /// 长截图触发（M4）：工具栏按钮 / 长截图键设置屏幕坐标选区，随后覆盖层关闭。
    pub viewport: Option<(i32, i32, u32, u32)>,
    /// 平台能力（上屏、光标、剪贴板、对话框……）。
    host: Box<dyn Host>,
}

impl Overlay {
    /// 覆盖层关闭后的结果。
    pub fn result(&self) -> OverlayResult {
        if self.cancelled {
            OverlayResult::Cancelled
        } else if let Some(vp) = self.viewport {
            OverlayResult::Viewport(vp)
        } else {
            OverlayResult::Done
        }
    }

    /// 当前工具（会话结束时记住，下次截图沿用，M3）。
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// 长截图键（平台原生键码）。
    pub fn longshot_key(&self) -> u32 {
        self.longshot_key
    }

    /// 双击（EDT-3/EDT-4/M3）：命中文本对象 → 二次编辑；命中曲线控制柄 → 恢复直线；
    /// 否则选区空白内双击 = 完成截图（复制）。
    pub fn on_double_click(&mut self, x: i32, y: i32) {
        let edited = self.hit_object(x, y).map(|i| self.begin_edit_text(i)).unwrap_or(false);
        if edited || self.reset_curve_at(x, y) {
            return;
        }
        // 双击第一击在选区内空白处按下（start_draw），作废草稿并完成截图
        if let Some(sel) = self.selection
            && sel.contains(x, y)
            && self.down_started_draw
        {
            self.draft = None;
            self.on_copy();
        }
    }
}
