// SPDX-License-Identifier: GPL-3.0-only
//! 长截图核心（LNG-1/3/4/5/6/7/10/11）：与平台无关的条带拼接与会话状态机。
//!
//! 模块分工：`stitch` 条带匹配纯函数、`rows` 行缓冲画布、`engine` 拼接状态机、
//! `session` 会话状态机（帧源由平台层注入）、`debug` 排障日志。
//! 执行窗口、状态条、视口边框与预览窗属于平台层。

pub mod debug;
pub mod engine;
pub mod rows;
pub mod session;
pub mod stitch;
