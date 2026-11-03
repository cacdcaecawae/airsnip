# ADR 0019：macOS 移植——共享核心 + 平台宿主接口

- 状态：已采纳（macOS 移植 M1–M2）
- 日期：2026-11-01
- 相关：CAP-1、CAP-2、CAP-3、CAP-5、EDT-4、OUT-1、OUT-2、SYS-2；审计与阶段记录见 `docs/macos-port.md`

## 背景

AirSnip 原本只有 Windows 版：Rust + windows-rs，覆盖层是一个横跨整个虚拟桌面的 Win32 窗口，
交互逻辑（框选、手柄、标注、工具栏、字体面板）与 Win32 调用写在同一个 `Overlay` 结构的各个方法里。
要做 macOS 原生版，又不能另写一套行为不同的编辑器，也不能破坏 Windows 版。

审计发现 Win32 只占覆盖层的外壳：约 2.6k 行交互代码没有任何 Win32 调用，
真正依赖平台的是上屏（DIB + BitBlt）、光标句柄、窗口关闭、提示框、剪贴板、保存对话框、贴图载荷、
输入法开关，以及交互代码里直接调用 `GetKeyState` 读 Shift / 空格。

## 决策

1. **Cargo workspace 三个包**：根包 `airsnip` 保持为 Windows 外壳（路径、`build.rs`、`package.ps1` 不变）；
   `crates/airsnip-core` 放平台无关代码；`crates/airsnip-macos` 是给 Swift 调用的 C ABI 静态库。
   模块用 `git mv` 迁移并在根包以同名模块再导出，Windows 业务代码的 `crate::…` 路径不变。

2. **覆盖层拆成「核心 `Overlay` + 平台 `Host` trait」**。`Host` 只有 9 个方法：
   `present`、`set_cursor`、`close`、`alert`、`copy_image`、`pin`、`pick_save_file`、`save_image`、`set_ime`。
   Windows 的 `WinHost` 持有原来的 HWND / DIB / HIMC，逐字搬运原实现；macOS 的 `MacHost` 转发给 Swift 回调表。
   修饰键改为 `Overlay::mods` 字段，由平台层在分发每个输入事件前写入。

3. **macOS 每块显示器一个覆盖层**。macOS 全局坐标是 points，各屏缩放比例可以不同，不存在 Windows 那样统一的
   物理像素平面。每个覆盖层的坐标就是所在显示器的像素坐标，核心继续「坐标统一为物理像素」。
   代价：选区不能跨屏（与系统 ⌘⇧4 一致）。

4. **少量平台差异留在核心里用 `cfg` 隔离**，而不是再抽一层 trait：文字后端（DirectWrite / Core Text）、
   Windows 的「光标所在显示器 DPI」查询、配置与默认保存路径、本地时间、默认键位。

5. **macOS 文字输入用 `NSTextInputClient`（M3 实现），不用 NSTextView**：编辑中的文字仍由核心栅格化，
   预览与导出是同一套渲染；输入法组合串经 `Overlay::on_text_composition(result, comp)` 传入，
   与 Windows 的 `WM_IME_COMPOSITION` 共用同一模型。

6. **键码按平台原生存储**：配置里的热键与工具键在 Windows 是 VK + `MOD_*`，在 macOS 是 `kVK_*` + Carbon 修饰位，
   默认值按平台区分（macOS：⌃⌘A 截图、⌃⌥P 贴图、数字 1–6 工具、S 长截图）。两平台配置文件路径不同，互不读取。

## 后果

- 两个平台的编辑器是同一份代码，标注、手柄、撤销、工具栏的行为天然一致；修一处两边生效。
- Windows 测试数不减（198 → 201），clippy warning 不增（47 → 41），release exe 体积不变（约 1.07 MB）。
- 核心对外暴露的方法从 `pub(super)` 改为 `pub`（被 Windows 消息分发与 macOS FFI 调用），封装边界变宽；
  约定只有平台外壳调用这些入口。
- 开发构建用临时签名并把指定要求固定为 bundle id，「屏幕录制」授权可跨构建保留；正式分发需 Developer ID 签名与公证。
