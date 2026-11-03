// SPDX-License-Identifier: GPL-3.0-only
//! 编辑文档（EDT-7）：对象列表 + 快照式撤销/重做。
//!
//! 快照式（每次编辑保存整份对象列表）而非逆操作命令：
//! 对象数量少（每个约 40 字节，文本对象共享字符串），快照成本可忽略，
//! 且不会出现逆操作实现的隐蔽错误。

use super::Object;

/// 撤销栈深度上限，防止长时间标注累积内存。
pub const MAX_HISTORY: usize = 100;

/// 编辑文档：对象列表 + 撤销/重做历史。
#[derive(Default)]
pub struct Document {
    objects: Vec<Object>,
    undo: Vec<Vec<Object>>,
    redo: Vec<Vec<Object>>,
    /// `begin` 记录的快照，`commit` 时归档。
    pending: Option<Vec<Object>>,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    /// 可变对象列表（控制柄拖动用；不自动进历史，调用方负责 `begin`/`commit`）。
    pub fn objects_mut(&mut self) -> &mut [Object] {
        &mut self.objects
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// 开始一次可撤销编辑（拖拽/新建/编辑文本前调用）。
    pub fn begin(&mut self) {
        self.pending = Some(self.objects.clone());
    }

    /// 结束编辑：`changed` 为真时把 `begin` 时的快照压入撤销栈。
    /// 拖拽中每帧调用 `begin` 会污染历史，调用方应在按下时 `begin`、松开时 `commit`。
    pub fn commit(&mut self, changed: bool) {
        let Some(before) = self.pending.take() else {
            return;
        };
        if !changed {
            return;
        }
        self.push_history(before);
    }

    /// 直接提交一次变更（无 `begin` 的简单操作，如添加对象后立即归档）。
    fn push_history(&mut self, before: Vec<Object>) {
        self.undo.push(before);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// 追加对象（不自动进历史，需调用方自行 `begin`/`commit`）。
    pub fn push(&mut self, obj: Object) {
        self.objects.push(obj);
    }

    /// 移除指定索引的对象（不自动进历史）。
    pub fn remove(&mut self, index: usize) -> Object {
        self.objects.remove(index)
    }

    /// 撤销；返回是否有变化。
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.objects, prev));
        true
    }

    /// 重做；返回是否有变化。
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.objects, next));
        true
    }
}
