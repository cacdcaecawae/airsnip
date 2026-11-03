// SPDX-License-Identifier: GPL-3.0-only
//! 标注状态与输出（EDT-1/2/7、OUT-1/2/3）：当前工具与样式、工具栏动作、
//! 撤销/重做、复制与保存。

use super::{geometry::SelRect, surface::crop_pixmap, toolbar};
use crate::editor::{self, Style};
use crate::{output, render};

impl super::Overlay {
    /// 当前样式（EDT-7：颜色 + 连续线宽）。
    pub(super) fn current_style(&self) -> Style {
        Style::new(editor::PALETTE[self.color_index], self.line_width)
    }

    /// 按下键对应的标注工具（工具切换键配置，M3）。
    pub fn tool_by_key(&self, vk: u32) -> Option<crate::editor::Tool> {
        let k = &self.tool_keys;
        if vk == k.rect {
            Some(crate::editor::Tool::Rect)
        } else if vk == k.arrow {
            Some(crate::editor::Tool::Arrow)
        } else if vk == k.text {
            Some(crate::editor::Tool::Text)
        } else if vk == k.blur {
            Some(crate::editor::Tool::Blur)
        } else if vk == k.highlight {
            Some(crate::editor::Tool::Highlight)
        } else if vk == k.glow {
            Some(crate::editor::Tool::Glow)
        } else {
            None
        }
    }

    /// 切换到指定工具（经工具栏动作统一处理，使其重绘）。
    pub fn switch_tool(&mut self, t: crate::editor::Tool) {
        self.apply_action(toolbar::Action::Tool(t));
    }

    pub(super) fn bar_state(&self) -> toolbar::State {
        let (font_family, font_size, bold, italic) = self.font_state();
        toolbar::State {
            tool: self.tool,
            color: self.color_index,
            filled: self.filled,
            can_undo: self.doc.can_undo(),
            can_redo: self.doc.can_redo(),
            font_family,
            bold,
            italic,
            font_size,
            font_open: self.font_list.is_some(),
        }
    }

    /// 执行工具栏动作（EDT-7）。
    pub(super) fn apply_action(&mut self, action: toolbar::Action) {
        match action {
            toolbar::Action::Tool(t) => self.tool = t,
            toolbar::Action::Color(i) => {
                self.color_index = i;
                // EDT-7：颜色同时作用于选中的对象
                self.style_selected();
            }
            toolbar::Action::ToggleFill => {
                // 文本工具无背景功能：填充开关仅对矩形有效，文本下忽略
                if self.tool != crate::editor::Tool::Text {
                    self.filled = !self.filled;
                    self.style_selected();
                }
            }
            // EDT-7：无删除/层级按钮（用户取舍：删除仅能靠 Ctrl+Z 撤销）
            toolbar::Action::Undo => return self.on_undo(),
            toolbar::Action::Redo => return self.on_redo(),
            // PIN-1：贴图按钮 → 选区贴图后覆盖层自行关闭（不复绘）
            toolbar::Action::Pin => return self.on_pin(),
            // M4：长截图按钮 → 选区作为视口进入拼接（不复绘）
            toolbar::Action::Longshot => return self.on_longshot(),
            // OUT-1：复制按钮 → 复制到剪贴板并关闭覆盖层（不复绘）
            toolbar::Action::Copy => return self.on_copy(),
            // OUT-2：保存按钮 → 按配置保存并关闭覆盖层（不复绘）
            toolbar::Action::Save => return self.on_save(),
            // EDT-4：文本第二行（字体族 / 粗斜体 / 字号）见 `font`
            toolbar::Action::FontPick
            | toolbar::Action::ToggleBold
            | toolbar::Action::ToggleItalic
            | toolbar::Action::FontSmaller
            | toolbar::Action::FontLarger
            | toolbar::Action::FontSize => return self.on_font_action(action),
        }
        // 工具栏自身外观或第二行显隐可能变化
        self.refresh_toolbar();
        self.repaint_toolbar_chrome();
    }

    /// 撤销（Ctrl+Z）。
    pub fn on_undo(&mut self) {
        if self.doc.undo() {
            self.selected = None; // 索引已变，控制柄不再对应
            self.refresh_toolbar(); // 选中文本对象消失时第二行随之收起（EDT-4）
            self.repaint_after_history();
        }
    }

    /// 重做（Ctrl+Y / Ctrl+Shift+Z）。
    pub fn on_redo(&mut self) {
        if self.doc.redo() {
            self.selected = None;
            self.refresh_toolbar();
            self.repaint_after_history();
        }
    }

    /// 撤销/重做后重绘：被撤销的对象可能位于选区之外，而另一份历史快照里的
    /// 对象包围盒不可得，故整屏重绘（仅按键/按钮触发，不在拖拽路径上）。
    fn repaint_after_history(&mut self) {
        let (vw, vh) = (self.display.width() as i32, self.display.height() as i32);
        self.repaint(SelRect {
            x: 0,
            y: 0,
            w: vw,
            h: vh,
        });
    }

    /// 右键取消（OUT-3）。
    pub fn on_cancel(&mut self) {
        self.cancelled = true;
        self.host.close();
    }

    /// 输出用像素图：裁剪选区 → 把标注对象烘焙进结果图（OUT-1/OUT-2）。
    /// 标注坐标是覆盖层坐标系，需按裁剪原点平移到裁剪图坐标。
    fn baked_selection(&self, sel: SelRect) -> Option<tiny_skia::Pixmap> {
        let mut cropped = crop_pixmap(&self.original, sel)?;
        render::bake_objects(&mut cropped, self.doc.objects(), (sel.x, sel.y));
        Some(cropped)
    }

    /// PIN-1：F3 / 工具栏贴图按钮 → 选区（含标注）交给平台层贴图，并关闭覆盖层。
    /// 平台层失败（如内存不足）时保留覆盖层，用户可重试。
    pub fn on_pin(&mut self) {
        if let Some(sel) = self.selection
            && let Some(cropped) = self.baked_selection(sel)
            && !self.host.pin(cropped, sel.x, sel.y)
        {
            return;
        }
        // 与复制/保存一致：贴图即完成，关闭覆盖层
        self.cancelled = false;
        self.host.close();
    }

    /// 长截图（M4，LNG-1）：记录屏幕坐标选区并关闭覆盖层，由平台层启动拼接。
    pub fn on_longshot(&mut self) {
        if let Some(sel) = self.selection {
            self.viewport = Some((
                sel.x + self.capture.origin_x,
                sel.y + self.capture.origin_y,
                sel.w as u32,
                sel.h as u32,
            ));
            self.cancelled = false;
            self.host.close();
        }
    }

    /// Enter / Ctrl+C：复制到剪贴板并关闭（OUT-1）。
    pub fn on_copy(&mut self) {
        if let Some(sel) = self.selection {
            let Some(cropped) = self.baked_selection(sel) else {
                return;
            };
            self.host.copy_image(cropped);
        }
        self.cancelled = false;
        self.host.close();
    }

    /// Ctrl+S / 工具栏保存按钮：弹原生另存为对话框，按所选格式保存（OUT-2）。
    /// 用户取消时保留覆盖层（可继续标注），不算截图完成。
    pub fn on_save(&mut self) {
        let Some(sel) = self.selection else {
            return;
        };
        // 预填文件名按设置里的命名模板；格式用上次保存的（首次为 PNG）
        let ext = self.save.last_ext.as_deref().unwrap_or("png").to_string();
        let name = format!("{}.{ext}", output::base_name(self.save.name_style));
        // 初始目录：上次保存过的目录优先（OUT-2），否则默认截图目录；
        // 都不存在时传 None，交给系统自己记住的位置。
        let dir = self
            .save
            .last_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(output::default_save_dir);
        let dir = dir.is_dir().then_some(dir);
        let chosen = self.host.pick_save_file(dir.as_deref(), &name, &ext);
        // 对话框盖在覆盖层之上：关闭后整屏重绘，否则留下对话框残影
        let (vw, vh) = (self.display.width() as i32, self.display.height() as i32);
        self.repaint(SelRect {
            x: 0,
            y: 0,
            w: vw,
            h: vh,
        });
        let Some(path) = chosen else {
            return;
        };
        // 记住本次格式与目录：下次打开对话框时默认选中/定位（OUT-2）
        if let Some(e) = path.extension().and_then(|e| e.to_str()) {
            crate::settings::last_save_ext_store(e);
        }
        if let Some(parent) = path.parent() {
            crate::settings::last_save_dir_store(parent);
        }
        let Some(cropped) = self.baked_selection(sel) else {
            return;
        };
        // 保存失败必须提示：静默吞掉会让用户以为截图已保存（OUT-2），由平台层负责
        self.host.save_image(cropped, path);
        self.cancelled = false;
        self.host.close();
    }
}
