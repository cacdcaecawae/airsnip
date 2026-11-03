// SPDX-License-Identifier: GPL-3.0-only
//! 行缓冲（LNG-4）：定宽、可变高的 RGBA 行队列，作为拼接画布。
//! 用 `VecDeque<Vec<u8>>` 而非连续 `Vec<u8>`：向上滚动（LNG-10）需要频繁在头部插入，
//! 连续缓冲每次前置都是 O(n) 拷贝（多帧累积成 O(n²)），行队列两端都是 O(1)。

use std::collections::VecDeque;

use tiny_skia::Pixmap;

/// 定宽、可变高的行缓冲（每行 `w * 4` 字节 RGBA）。
pub struct RowBuffer {
    w: u32,
    rows: VecDeque<Vec<u8>>,
}

impl RowBuffer {
    pub fn new(w: u32) -> Self {
        Self {
            w,
            rows: VecDeque::new(),
        }
    }

    pub fn height(&self) -> u32 {
        self.rows.len() as u32
    }

    /// 取第 `y` 行（调用方保证 `y < height`）。
    pub fn row(&self, y: u32) -> &[u8] {
        &self.rows[y as usize]
    }

    fn stride(&self) -> usize {
        self.w as usize * 4
    }

    /// 把 `src` 的 `[y0, y1)` 行追加到末尾。
    pub fn append(&mut self, src: &[u8], y0: u32, y1: u32) {
        let s = self.stride();
        for y in y0..y1 {
            let off = y as usize * s;
            if off + s > src.len() {
                break;
            }
            self.rows.push_back(src[off..off + s].to_vec());
        }
    }

    /// 把 `src` 的 `[y0, y1)` 行插入到开头（向上滚动用，LNG-10）。
    /// 倒序 `push_front` 以保持 `[y0, y1)` 的原始顺序。
    pub fn prepend(&mut self, src: &[u8], y0: u32, y1: u32) {
        let s = self.stride();
        for y in (y0..y1).rev() {
            let off = y as usize * s;
            if off + s > src.len() {
                continue;
            }
            self.rows.push_front(src[off..off + s].to_vec());
        }
    }

    /// 删除开头 `n` 行（固定页眉检测后从画布剔除，LNG-5）。
    pub fn remove_top(&mut self, n: u32) {
        for _ in 0..n.min(self.height()) {
            self.rows.pop_front();
        }
    }

    /// 删除末尾 `n` 行（固定页脚检测后从画布剔除，LNG-5）。
    pub fn remove_bottom(&mut self, n: u32) {
        for _ in 0..n.min(self.height()) {
            self.rows.pop_back();
        }
    }

    /// 把另一行缓冲的所有行追加到末尾（输出时拼装 页眉 + 内容 + 页脚）。
    pub fn append_buffer(&mut self, other: &RowBuffer) {
        for row in &other.rows {
            self.rows.push_back(row.clone());
        }
    }

    /// 转为像素图（尺寸非法时返回 None）。
    pub fn into_pixmap(self) -> Option<Pixmap> {
        let h = self.height();
        if self.w == 0 || h == 0 {
            return None;
        }
        let mut data = Vec::with_capacity(self.stride() * h as usize);
        for row in &self.rows {
            data.extend_from_slice(row);
        }
        Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(self.w, h)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_bytes(w: u32, v: u8) -> Vec<u8> {
        vec![v; w as usize * 4]
    }

    #[test]
    fn append_prepend_trim_keeps_rows_in_order() {
        let w = 2;
        let mut buf = RowBuffer::new(w);
        let src: Vec<u8> = (1u8..=4).flat_map(|v| row_bytes(w, v)).collect();

        buf.append(&src, 0, 2); // 行 1,2
        assert_eq!(buf.height(), 2);
        assert_eq!(buf.row(0), row_bytes(w, 1));
        assert_eq!(buf.row(1), row_bytes(w, 2));

        buf.prepend(&src, 2, 4); // 头部插入 行 3,4
        assert_eq!(buf.height(), 4);
        assert_eq!(buf.row(0), row_bytes(w, 3));
        assert_eq!(buf.row(1), row_bytes(w, 4));
        assert_eq!(buf.row(3), row_bytes(w, 2));

        buf.remove_top(1);
        assert_eq!(buf.height(), 3);
        assert_eq!(buf.row(0), row_bytes(w, 4));

        buf.remove_bottom(2);
        assert_eq!(buf.height(), 1);
        assert_eq!(buf.row(0), row_bytes(w, 4));

        let pixmap = buf.into_pixmap().expect("尺寸合法");
        assert_eq!((pixmap.width(), pixmap.height()), (w, 1));
    }

    #[test]
    fn out_of_range_rows_are_ignored() {
        let mut buf = RowBuffer::new(1);
        let src = row_bytes(1, 7);
        buf.append(&src, 5, 9); // 越界，忽略
        buf.prepend(&src, 3, 1); // 逆序，忽略
        assert_eq!(buf.height(), 0);
        assert!(buf.into_pixmap().is_none());
    }
}
