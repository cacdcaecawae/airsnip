// SPDX-License-Identifier: GPL-3.0-only
//! 长截图会话（LNG-1/3/6/10）：纯手动模式 —— 用户自己滚动页面，会话每拍抓帧拼接；
//! 完成/取消/撤销由状态条或按键驱动。
//!
//! 自动滚动模式已移除（M5 按用户反馈取消）：自动模式靠「连续静帧」判定到底，
//! 页面加载停顿或平滑滚动途中就会被误判为到底，拼接提前结束、内容缺失。

use super::engine::{Canvas, Step};
use super::stitch::Params;

/// 连续定位失败达到该拍数时在状态条提示「放慢滚动」。
const FAIL_HINT: u32 = 2;

/// 帧源抽象：截取当前视口帧。真实实现由平台层走屏幕抓取，测试实现模拟一个可滚动长页面
/// （无需 GUI 即可验证拼接循环）。
pub trait Source {
    /// 截取当前视口像素（长度 = w*h*4）。
    fn capture(&mut self) -> Result<Vec<u8>, String>;
}

/// 长截图会话状态。
pub struct Session {
    w: u32,
    h: u32,
    canvas: Canvas,
    /// 帧源（真实屏幕或测试模拟）。
    source: Box<dyn Source>,
    /// 是否已采集首帧（不能用 canvas.height()==0 判断：seed 只缓冲、高度仍为 0）。
    seeded: bool,
    /// 连续定位失败拍数（仅用于状态条提示）。
    fail_run: u32,
    pub finished: bool,
    pub cancelled: bool,
    /// 最近一条状态文本（调试/测试用）。
    pub status: String,
    /// 抓帧序号（调试用）。
    frame_seq: u32,
}

impl Session {
    /// 用指定帧源构造（真实屏幕由平台层提供，测试注入模拟页面）。
    pub fn with_source(source: Box<dyn Source>, w: u32, h: u32, max_height: u32) -> Result<Self, String> {
        if w == 0 || h == 0 {
            return Err("长截图视口尺寸非法".into());
        }
        Ok(Self {
            w,
            h,
            canvas: Canvas::new(w, Params::default(), max_height),
            source,
            seeded: false,
            fail_run: 0,
            finished: false,
            cancelled: false,
            status: String::new(),
            frame_seq: 0,
        })
    }

    /// 抓取当前视口帧。
    pub fn capture(&mut self) -> Result<Vec<u8>, String> {
        let frame = self.source.capture()?;
        self.frame_seq += 1;
        if std::env::var_os("AIRSNIP_DEBUG").is_some() {
            // 简单哈希（异或逐 8 字节）+ 采样首像素，看每帧是否真在变。
            let mut h: u64 = 0;
            for c in frame.chunks_exact(8).step_by(97) {
                h ^= u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                h = h.rotate_left(5);
            }
            super::debug::log(&format!("capture#{} hash={h:016x}", self.frame_seq));
        }
        // 调试：导出首帧供目视核对抓取区域。
        if !self.seeded && std::env::var_os("AIRSNIP_DEBUG").is_some() {
            let size = tiny_skia::IntSize::from_wh(self.w, self.h);
            if let Some(size) = size
                && let Some(px) = tiny_skia::Pixmap::from_vec(frame.clone(), size)
            {
                let path = std::env::temp_dir().join("airsnip-longshot-frame0.png");
                let _ = px.save_png(&path);
            }
        }
        Ok(frame)
    }

    /// 一拍（定时器回调）：首帧只缓冲，之后每拍抓帧拼接。
    /// 返回是否需要重绘状态条。
    pub fn tick(&mut self) -> bool {
        if self.finished || self.cancelled {
            return false;
        }
        if !self.seeded {
            return self.seed();
        }
        self.tick_frame()
    }

    fn seed(&mut self) -> bool {
        match self.capture() {
            Ok(frame) => {
                self.canvas.seed(&frame);
                self.seeded = true;
                self.status = format!("已拼 {} px · 滚轮滚动页面，Enter 完成", self.canvas.height());
            }
            Err(e) => {
                self.cancelled = true;
                self.status = format!("抓屏失败：{e}");
            }
        }
        true
    }

    /// 抓一帧并拼入画布（用户手动滚动后画面变化即被检测到）。
    fn tick_frame(&mut self) -> bool {
        match self.capture() {
            Ok(frame) => match self.canvas.push(&frame, self.h) {
                Step::Appended { rows, .. } => {
                    self.fail_run = 0;
                    self.status = format!(
                        "已拼 {} px(+{rows}) · Enter 完成 / Esc 取消 / Ctrl+Z 撤销",
                        self.canvas.height()
                    );
                    super::debug::log(&format!("appended +{rows}, total {}", self.canvas.height()));
                }
                Step::Still => {
                    // 画面静止：之前的错位已不相关，清掉失败提示
                    self.fail_run = 0;
                    self.status = format!("已拼 {} px · 滚轮滚动页面，Enter 完成", self.canvas.height());
                }
                Step::Failed => {
                    self.fail_run += 1;
                    self.status = "检测到滚动但拼接失败，请放慢滚动速度".to_string();
                    super::debug::log("failed: locate mismatch");
                }
                Step::Limit => {
                    self.finished = true;
                    self.status = "已达高度上限，长截图完成".to_string();
                }
            },
            Err(e) => {
                self.cancelled = true;
                self.status = format!("抓屏失败：{e}");
            }
        }
        true
    }

    /// 当前已拼高度（供状态条显示）。
    pub fn height(&self) -> u32 {
        self.canvas.height()
    }

    /// 生成整图缩略图（预览窗用，LNG-3）。
    pub fn thumbnail(&self, max_w: u32, max_h: u32) -> Option<tiny_skia::Pixmap> {
        self.canvas.thumbnail(max_w, max_h)
    }

    /// 是否有可撤销的段。
    pub fn can_undo(&self) -> bool {
        self.canvas.can_undo()
    }

    /// 状态条提示（空串 = 正常进度）。
    pub fn hint(&self) -> &'static str {
        if self.cancelled {
            "已取消"
        } else if self.finished {
            "已完成"
        } else if self.fail_run >= FAIL_HINT {
            "拼接定位失败，请放慢滚动"
        } else {
            ""
        }
    }

    /// 撤销最后一段。
    pub fn undo(&mut self) {
        if self.canvas.undo_last() {
            self.fail_run = 0;
            self.status = format!("已撤销，当前 {} px", self.canvas.height());
        } else {
            self.status = "没有可撤销的段".to_string();
        }
    }

    /// 导出拼接结果；失败返回错误文本。
    pub fn result(&self) -> Result<tiny_skia::Pixmap, String> {
        if self.canvas.height() == 0 {
            return Err("画布为空".into());
        }
        self.canvas.to_pixmap().ok_or_else(|| "画布转像素图失败".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_rejects_zero_size() {
        struct Never;
        impl Source for Never {
            fn capture(&mut self) -> Result<Vec<u8>, String> {
                Err("构造期间不应抓帧".into())
            }
        }
        assert!(Session::with_source(Box::new(Never), 0, 0, 30000).is_err());
        assert!(Session::with_source(Box::new(Never), 100, 200, 30000).is_ok());
    }

    #[test]
    fn max_height_constant_sane() {
        // 高度上限已从配置传入（LNG-7），此处保留常量合理性断言。
        assert!(crate::settings::Config::default().longshot_max_height > 0);
    }

    /// 模拟一个可滚动长页面：拥有超过一屏的内容，每次抓帧前画面已按 STEP 像素下移
    /// （等价于用户在两拍之间滚动一次）。这能在无 GUI 下验证手动拼接整条链路。
    struct FakePage {
        page: Vec<u8>,
        w: u32,
        h: u32,
        offset: u32,
        step: u32,
        max_offset: u32,
    }

    impl FakePage {
        fn new(w: u32, h: u32, total: u32, step: u32) -> Self {
            // 每行内容由全局行号决定（保证行间可区分）
            let mut page = vec![0u8; (w * total * 4) as usize];
            for y in 0..total {
                for x in 0..w {
                    let o = ((y * w + x) * 4) as usize;
                    page[o] = (y & 0xff) as u8;
                    page[o + 1] = ((y >> 8) & 0xff) as u8;
                    page[o + 2] = ((x * 7 + y * 3) & 0xff) as u8;
                    page[o + 3] = 255;
                }
            }
            let max_offset = total.saturating_sub(h);
            Self {
                page,
                w,
                h,
                offset: 0,
                step,
                max_offset,
            }
        }

        fn window(&self) -> Vec<u8> {
            let stride = self.w as usize * 4;
            let start = self.offset as usize * stride;
            self.page[start..start + self.h as usize * stride].to_vec()
        }
    }

    impl Source for FakePage {
        fn capture(&mut self) -> Result<Vec<u8>, String> {
            let out = self.window();
            // 模拟用户在两拍之间滚一格（到底后停在最大偏移）。
            self.offset = (self.offset + self.step).min(self.max_offset);
            Ok(out)
        }
    }

    /// 手动模式端到端：逐拍滚动拼完整页，断言结果与原始长页面一致。
    /// 这是长截图核心链路的回归保护。
    #[test]
    fn manual_scroll_stitches_full_page() {
        let (w, h) = (64u32, 50u32);
        let (total, step) = (300u32, 20u32);
        let page = FakePage::new(w, h, total, step);
        let expect = page.page.clone();
        let mut s = Session::with_source(Box::new(page), w, h, 30000).expect("会话创建");

        // 首帧 seed + 覆盖整页所需的拍数（每拍画面下移 step 行）
        let ticks = ((total - h) / step) as usize + 4;
        for _ in 0..ticks {
            assert!(!s.cancelled);
            s.tick();
        }
        assert!(!s.finished, "手动模式不会自行结束");
        let img = s.result().expect("有拼接结果");
        assert!(
            (img.height() as i32 - total as i32).abs() <= step as i32,
            "拼接高度 {} 应接近整页高度 {}",
            img.height(),
            total
        );
        // 顶部若干行与页面顶部一致
        let stride = w as usize * 4;
        for y in 0..(h as usize / 2) {
            let o = y * stride;
            assert_eq!(
                &img.data()[o..o + stride],
                &expect[o..o + stride],
                "拼接图第 {y} 行应与页面一致"
            );
        }
    }

    /// 页面到底后继续抓帧：画布不再增长，且不会自动结束（由用户 Enter 收尾）。
    #[test]
    fn still_page_does_not_grow() {
        let page = FakePage::new(64, 50, 120, 20);
        let mut s = Session::with_source(Box::new(page), 64, 50, 30000).expect("会话创建");
        for _ in 0..40 {
            s.tick();
        }
        let settled = s.height();
        for _ in 0..10 {
            s.tick();
        }
        assert_eq!(s.height(), settled, "到底后画布不应继续增长");
        assert!(!s.finished, "手动模式不自动结束");
        assert_eq!(s.hint(), "", "到底后不应显示失败提示");
    }

    /// 撤销最后一段：高度回退，且无段可撤时不再变化。
    #[test]
    fn undo_removes_last_segment() {
        let page = FakePage::new(64, 50, 200, 20);
        let mut s = Session::with_source(Box::new(page), 64, 50, 30000).expect("会话创建");
        s.tick(); // 首帧 seed
        s.tick();
        s.tick();
        let before = s.height();
        assert!(s.can_undo(), "拼入内容后应可撤销");
        s.undo();
        assert!(s.height() < before, "撤销后高度应回退");
    }

    /// 图片较短（不足两屏）时，拼接不应崩溃，且高度接近单屏。
    #[test]
    fn short_page_does_not_crash() {
        let page = FakePage::new(64, 50, 55, 20);
        let mut s = Session::with_source(Box::new(page), 64, 50, 30000).expect("会话创建");
        for _ in 0..20 {
            s.tick();
        }
        let img = s.result().expect("有结果");
        assert!(img.height() >= 50 && img.height() <= 55 + 20);
    }
}
