// SPDX-License-Identifier: GPL-3.0-only
//! 性能埋点（M5）：仅在 `AIRSNIP_TIMING=1` 时记录，常态零开销（无系统调用、无分配）。
//! 采集项：热键 → 覆盖层可见的总耗时、每帧重绘耗时（每 60 帧打印平均与最大值）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static ENABLED: OnceLock<bool> = OnceLock::new();
static CAPTURE_AT: Mutex<Option<Instant>> = Mutex::new(None);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static FRAME_NS: AtomicU64 = AtomicU64::new(0);
static FRAME_MAX_NS: AtomicU64 = AtomicU64::new(0);

/// 是否开启埋点：`AIRSNIP_TIMING` 环境变量存在即开启（只读一次）。
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("AIRSNIP_TIMING").is_some())
}

/// 记录热键触发时刻（`start_capture` 入口，含撤销贴图热键等准备工作之前）。
pub fn mark_capture_start() {
    if !enabled() {
        return;
    }
    if let Ok(mut at) = CAPTURE_AT.lock() {
        *at = Some(Instant::now());
    }
}

/// 覆盖层首帧显示完成：打印「热键 → 可见」总耗时（红线：1080p ≤150ms、4K ≤250ms）。
pub fn log_overlay_shown() {
    if !enabled() {
        return;
    }
    if let Ok(at) = CAPTURE_AT.lock() {
        if let Some(t) = *at {
            eprintln!("[airsnip] 热键到覆盖层显示 {:?}", t.elapsed());
        }
    }
}

/// 单帧计时守卫：离开作用域时累计耗时，`repaint` 的所有出口（含提前返回）都会统计。
pub struct FrameTimer(Option<Instant>);

/// 每累计多少帧打印一次进度（帧数少时靠 `log_frames_summary` 收尾）。
const PRINT_EVERY: u64 = 30;

impl Drop for FrameTimer {
    fn drop(&mut self) {
        let Some(t) = self.0 else { return };
        let ns = t.elapsed().as_nanos() as u64;
        let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
        FRAME_NS.fetch_add(ns, Ordering::Relaxed);
        FRAME_MAX_NS.fetch_max(ns, Ordering::Relaxed);
        if n % PRINT_EVERY == 0 {
            let total = FRAME_NS.load(Ordering::Relaxed);
            let max = FRAME_MAX_NS.load(Ordering::Relaxed);
            eprintln!(
                "[airsnip] 重绘 {n} 帧：平均 {:.1}ms，最大 {:.1}ms",
                total as f64 / n as f64 / 1e6,
                max as f64 / 1e6
            );
        }
    }
}

/// 覆盖层退出时打印帧汇总并重置计数（下次截图从零开始）。
pub fn log_frames_summary() {
    if !enabled() {
        return;
    }
    let n = FRAMES.swap(0, Ordering::Relaxed);
    if n == 0 {
        return;
    }
    let total = FRAME_NS.swap(0, Ordering::Relaxed);
    let max = FRAME_MAX_NS.swap(0, Ordering::Relaxed);
    eprintln!(
        "[airsnip] 本次共重绘 {n} 帧：平均 {:.1}ms，最大 {:.1}ms",
        total as f64 / n as f64 / 1e6,
        max as f64 / 1e6
    );
}

/// 开始一帧计时；未开启埋点时返回空守卫（一次原子读，可忽略）。
pub fn frame() -> FrameTimer {
    FrameTimer(enabled().then(Instant::now))
}
