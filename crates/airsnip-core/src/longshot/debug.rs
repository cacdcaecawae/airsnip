// SPDX-License-Identifier: GPL-3.0-only
//! 长截图调试日志：`AIRSNIP_DEBUG=1` 时把每步状态写入 `%TEMP%\airsnip-longshot.log`，
//! 用于定位滚动/拼接链路问题（M4 排障）。

use std::io::Write;
use std::sync::Mutex;

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// 写一行日志（无 AIRSNIP_DEBUG 时为空操作）。
pub fn log(msg: &str) {
    if std::env::var_os("AIRSNIP_DEBUG").is_none() {
        return;
    }
    let mut guard = LOG.lock().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        let path = std::env::temp_dir().join("airsnip-longshot.log");
        *guard = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
    }
    if let Some(f) = guard.as_mut() {
        let line = format!("[{}] {msg}\n", std::time::Instant::now().elapsed().as_millis());
        let _ = f.write_all(line.as_bytes());
    }
}
