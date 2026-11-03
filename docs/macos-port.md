# AirSnip macOS 原生移植：M0 审计与开发计划

> 状态：M0–M2 完成；M3–M7 自动化部分完成、GUI 待人工验证（记录见 §7）。本文是 M1–M7 的依据，随阶段推进更新。
> 基线：`main` @ `f3e0718`（v0.6.0 + PR #55 覆盖层 DPI 缩放），分支 `feat/macos-port`。

## 1. 实测基线（2026-11-01）

| 项目 | Windows（开发机） | macOS（MacBook，ssh） |
|---|---|---|
| 系统 | Windows 11 Pro 26100 | macOS 26.5（25F71），arm64 |
| Rust | 1.97.1 | 1.96.1（AGENTS.md 写的是 1.98） |
| `cargo test` | **198 通过 / 0 失败**（AGENTS.md 的 186 已过时） | 无法编译（见下） |
| `cargo fmt --check` | 通过 | — |
| `cargo clippy --all-targets` | 无 error，约 40 条存量 warning（collapsible_if、unnecessary_cast 等） | — |
| `cargo deny check` | 未执行：本机未装 cargo-deny | 未装 |
| `cargo check` | 通过 | **失败**：`windows` 是无条件依赖，`windows-future 0.3.2` 在 macOS 上 16 个编译错误 |
| Swift / Xcode | — | Swift 6.3.2（仅 Command Line Tools），**未安装 Xcode**，`xcodebuild` 不可用 |
| 系统框架 | — | SDK 26.5 含 ScreenCaptureKit / AppKit / CoreText / ServiceManagement / Carbon；`SCScreenshotManager.captureImage` 用 `swiftc` 实测可编译 |
| 屏幕录制权限 | — | ssh 会话内 `CGPreflightScreenCaptureAccess()` = false（预期：TCC 授权只能在 Mac 前由用户点） |

源码规模：97 个 `.rs` 文件、18.5k 行（含测试）。各模块行数与测试数：

| 模块 | 行数 | 测试 | 平台耦合 |
|---|---|---|---|
| `editor/` | 1453 | 33 | **纯逻辑**。唯一 Windows 痕迹：`DEFAULT_FONT_FAMILY = "Microsoft YaHei UI"` |
| `render/` | 2479 | 43 | 混合：`mod/bake/blur/mask/region/icon` 纯 tiny-skia；`text.rs`、`text_bitmap.rs`、`fonts.rs` 是 DirectWrite + GDI |
| `longshot/` | 2462 | 27 | 混合：`engine/stitch/rows/session/bar/debug` 纯逻辑（`session` 已有 `Source` 帧源 trait）；`window/border/preview/present` 是 Win32 分层窗口 + `SetTimer` |
| `overlay/` | 6090 | 46 | 混合，见 §2.2 |
| `pin/` | 1545 | 17 | 混合：`scale.rs`、`border.rs` 纯逻辑；其余是分层窗口、CF_DIB 解析、菜单 |
| `settings/` | 2158 | 17 | 混合：`Config` 模型纯 serde，但路径用 `%APPDATA%`、热键存 VK 码 + `MOD_*`；`panel/`（约 1.8k 行）是自绘 Win32 设置窗口 |
| `output/` | 266 | 4 | 混合：PNG/JPEG/WebP 编码纯 Rust；剪贴板 CF_DIB、`GetLocalTime`、`%USERPROFILE%` 是 Windows |
| `capture/` | 296 | 3 | Windows：GDI `BitBlt(CAPTUREBLT)`；`CapturedScreen` 结构与 R/B 交换可复用 |
| `theme/` | 227 | 5 | 混合：`Palette` 纯数据；系统主题读注册表、`WM_SETTINGCHANGE` 解析是 Windows |
| `app/` | 1412 | 3 | 全部 Windows：托盘、`RegisterHotKey`、自绘弹出菜单、schtasks 自启、`IFileSaveDialog`、单实例 mutex、消息循环 |
| `perf.rs` | 88 | 0 | 纯（`eprintln` 埋点） |
| `ocr/` | 4 | 0 | 二期占位，不移植 |

## 2. 关键发现

### 2.1 文本是 render 唯一的平台缝

`editor` 不依赖任何平台；`render` 对外只通过四个入口用文字：`text::measure`、`text::remeasure`、`text::draw`、`fonts::families`（外加 `FontSpec`）。Windows 实现是 DirectWrite 栅格化成黑字白底 DIB 再反色混合进 tiny-skia。macOS 用 Core Text + `CGBitmapContext`（仅灰度 alpha 位图）做同一件事即可，入口签名不变。

### 2.2 覆盖层：交互逻辑可直接复用，Win32 只占外壳

`Overlay` 是一个大状态结构，交互方法分散在各子模块的 `impl Overlay` 中。逐文件统计后：

- **零 Win32 引用**（约 2.6k 行，不含测试）：`interaction/`、`objedit/`、`pick.rs`、`selection/draw.rs`、`toolbar/`、`font/`、`geometry.rs`、`shape.rs`、`control.rs`。
- **少量 Win32 引用**：`selection/mod.rs`（1 处）、`text_edit_state.rs`（1）、`textinput.rs`（IME 字符串读取）、`ui.rs`（光标所在显示器的 DPI 查询）、`annotate.rs`（关窗 `PostMessage`、`MessageBox`、保存对话框、贴图载荷 `GlobalAlloc` 发给主窗口）。
- **纯 Win32**：`wndproc.rs`（消息分发 + VK 键码）、`setup.rs`、`init.rs`（HWND/HIMC 字段）、`surface.rs` 的上屏部分（DIB section + `BitBlt`）、`cursor/`（HCURSOR 生成）、`ime.rs`。

渲染管线对移植很友好：所有重绘都先画进 `display: Pixmap`（RGBA 预乘），再调 `present_region(rect)` 上屏。macOS 只需把 `present_region` 换成「通知 NSView 该矩形需要重绘，`draw(_:)` 时把 `display` 包成 `CGImage` 画出来」。tiny-skia 的预乘 RGBA 可直接对应 `kCGImageAlphaPremultipliedLast`，不需要转换像素。

**结论**：把 `Overlay` 拆成「平台无关的 `Session`（状态 + 全部交互方法）」和「平台宿主 `Host` trait（上屏、关闭、光标、提示框、保存对话框、复制、贴图、IME 开关）」。Windows 的 `wndproc` 改为调用同一个 `Session`，行为保持不变；macOS 的 NSView 把事件翻译后调用同样的方法。这样两端的编辑器是同一份代码，而不是 macOS 另写一套。

### 2.3 坐标模型差异（最大的架构问题）

Windows 版把整个虚拟桌面当作一个物理像素平面（Per-Monitor V2），覆盖层是一个横跨所有屏幕的窗口。macOS 的全局坐标是 points，各显示器的缩放比例可以不同（内置 Retina 2x + 外接 1x），**不存在统一的像素平面**。

方案：**每块显示器一个覆盖窗口、一个 `Session`**，每个 Session 的坐标就是该显示器的像素坐标（原点 0,0）。所有屏幕同时冻结和暗化；选区只能在一块显示器内（在哪块屏上按下就属于哪块）。这样 Session 内部继续「坐标统一为物理像素」，core 不用引入 points 概念；points ↔ pixels 只在 AppKit 视图里换算一次（`locationInWindow` 翻转 y 后乘 `backingScaleFactor`）。

行为差异：Windows 允许框选跨越两块屏幕，macOS 版不允许。这一点与 macOS 系统截图（⌘⇧4）的习惯一致，记入已知差异。

### 2.4 其他需要注意的事实

- **ADR 0007 已移除对象删除与层级调整**，所以「图层顺序」的基准是：对象按创建顺序叠放，新对象在上，不能调层级。macOS 不新增该功能。
- 发布配置 `panic = "abort"`。macOS 静态库需要 `catch_unwind` 把 panic 转成错误码（任务书要求不跨 FFI 传播 panic），因此要为 macOS 单独定义 `[profile.release-macos] inherits = "release"`、`panic = "unwind"`。Windows 发布配置不动。
- 热键与工具键都以平台原生码存储（Windows VK + `MOD_*`）。macOS 不能复用这些数值（例如数字键 `1`：VK `0x31`，macOS `kVK_ANSI_1` = `0x12`）。
- `output::timestamped_filename` 用 Win32 `GetLocalTime`，`default_save_dir` 用 `%USERPROFILE%`，`Config::config_dir` 用 `%APPDATA%`：都需要平台分支。
- Windows 的管理员运行、schtasks、UIPI、`TaskbarCreated` 自愈（ADR 0003/0013）是 Windows 专属问题，macOS 没有对应物，不移植。

## 3. 目标架构

```
Cargo.toml                 # [workspace]；根包仍是 Windows 版 airsnip（路径、build.rs、package.ps1 不变）
src/                       # Windows 外壳：app/、Win32 宿主、pin 窗口、设置面板、长截图窗口
crates/airsnip-core/       # 平台无关：editor、render（含 text 后端：dwrite / coretext 按 cfg 选择）、
                           #   overlay Session 与工具栏、longshot 算法与会话、编码、Config 模型、pin 缩放算法、theme Palette
crates/airsnip-macos/      # staticlib：C ABI（ffi.rs），头文件手写 airsnip.h（不引入 cbindgen）
macos/                     # Swift 包（AppKit）：菜单栏、热键、覆盖窗口、贴图、长截图、设置、权限引导
scripts/package-macos.sh   # cargo build → swift build → 组装 AirSnip.app → 签名 → zip/dmg
```

迁移方式是**移动文件 + 改路径**，不是复制：模块搬进 `airsnip-core` 后，Windows 根包通过 `use airsnip_core::…` 引用。每搬一批就在 Windows 上跑 `cargo test` + `cargo fmt --check`，保证测试数不减、全部通过。

### 3.1 平台替换表

| 能力 | Windows（现状） | macOS（计划） |
|---|---|---|
| 抓屏 | GDI `BitBlt(CAPTUREBLT)` 虚拟屏 | ScreenCaptureKit `SCScreenshotManager.captureImage`，每显示器一次，`SCContentFilter` 排除自身 App 窗口，宽高 = frame × backingScaleFactor |
| 权限 | 无 | `CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess`；拒绝时弹引导，按钮打开「系统设置 › 隐私与安全性 › 屏幕录制」 |
| 覆盖层 | 置顶分层窗口 + DIB + `BitBlt` | 每屏一个无边框 `NSWindow`（level 高于菜单栏，`canJoinAllSpaces` + `fullScreenAuxiliary`），自定义 `NSView` 画 `display` 像素 |
| 文本 | DirectWrite → GDI DIB | Core Text `CTLine/CTFrame` → `CGBitmapContext`（灰度），在 Rust 内用 C ABI 直接调用，不加新 crate |
| 输入法 | `WM_IME_COMPOSITION` 直读组合串（ADR 0004） | 覆盖 NSView 实现 `NSTextInputClient`，`setMarkedText` / `insertText` 喂给现有 `text_edit_state`，见 §3.3 |
| 字体枚举 | DirectWrite 系统字体集合 | `CTFontManagerCopyAvailableFontFamilyNames`；默认字体 PingFang SC |
| 全局热键 | `RegisterHotKey` | Carbon `RegisterEventHotKey`（不需要辅助功能权限），注册失败即提示冲突 |
| 剪贴板写 | CF_DIB | `NSPasteboard`：PNG + TIFF 两种表示 |
| 剪贴板读（贴图） | CF_DIB / CF_DIBV5 解析 | `NSPasteboard` → `NSImage` → `CGImage` → RGBA |
| 保存对话框 | `IFileSaveDialog` | `NSSavePanel`（格式下拉、记住上次目录与格式） |
| 托盘 / 菜单 | 托盘 + 自绘弹出菜单 | `NSStatusItem` + `NSMenu`（模板图标，自动适配深浅色） |
| 开机自启 | schtasks（管理员） | `SMAppService.mainApp.register()`，不需要管理员 |
| 单实例 | 命名 mutex | 启动时查同 bundle id 的 `NSRunningApplication`，激活已有实例后退出 |
| 贴图窗口 | `WS_EX_TOPMOST` 分层窗口 | `NSPanel`（`.floating` 级别、`nonactivatingPanel`），不使用私有 API |
| 长截图状态条 / 视口框 / 预览 | 分层窗口 + `WS_EX_TRANSPARENT` + `SetTimer` | `NSPanel` + `ignoresMouseEvents`（让滚轮穿透到目标窗口）+ 仅会话期间运行的 `DispatchSourceTimer`；抓帧用 `SCContentFilter` 排除自身窗口，状态条不会被拍进去 |
| 设置面板 | 自绘 tiny-skia 窗口 | AppKit 原生窗口（任务书要求原生设置界面）；数据仍走 core 的 `Config` |
| 配置路径 | `%APPDATA%\airsnip\config.toml` | `~/Library/Application Support/AirSnip/config.toml` |
| 默认保存目录 | `%USERPROFILE%\Pictures\airsnip` | `~/Pictures/airsnip`（技术标识小写，同 ADR 0014） |
| 主题 | 注册表 + `WM_SETTINGCHANGE` | `NSApp.effectiveAppearance` + KVO |

### 3.2 FFI 契约（M1 落地为 `airsnip.h`）

- 只用 C ABI，函数前缀 `airsnip_`。会话等对象以不透明指针传递（`AsSession *`），由 `airsnip_*_new` 创建、`airsnip_*_free` 释放，谁创建谁释放的规则写在头文件注释里。
- 返回值：`int32_t` 状态码（0 成功，负数错误）；错误详情用 `airsnip_last_error()`（线程局部 UTF-8 字符串，下次调用前有效）。
- 图像：`{ uint8_t *data; uint32_t width, height, stride; }`，RGBA8 预乘（tiny-skia 原生格式）。Swift 传入的抓屏缓冲由 Rust 复制；Rust 返回的图像（裁剪、烘焙结果）由 `airsnip_image_free` 释放。覆盖层的 `display` 缓冲以只读借用方式给出，仅在下一次调用该 Session 前有效。
- 字符串：入参为 NUL 结尾 UTF-8（Swift `String` 可直接传）；出参为静态或线程局部存储，调用方不释放。今后若需要返回 Rust 分配的字符串，再增加 `airsnip_string_free`。
- panic：每个导出函数体包在 `catch_unwind` 里，panic 转为错误码；macOS 构建使用 `panic = "unwind"` 配置（§2.4）。
- 线程：Session 只在主线程调用（Swift 侧 `@MainActor`）；Rust 内部的并行（暗化、模糊、格式转换）继续用 `std::thread::scope`，调用返回前全部结束。
- 回调（Session → 宿主）：用一张 C 函数指针表（`present`、`close`、`set_cursor`、`copy_image`、`pin_image`、`save_dialog`…）加 `void *ctx`，在 `airsnip_session_new` 时传入；回调只在调用线程（主线程）上同步触发。

### 3.3 文本输入的取舍（需要 ADR）

任务书建议「优先使用 NSTextView」。审计后建议改为**覆盖 NSView 实现 `NSTextInputClient`**，理由：

1. Windows 版的文字编辑框、光标与组合串都由 tiny-skia 绘制，用的就是最终导出的同一套栅格化。如果改用 NSTextView，编辑时是 AppKit 渲染、导出时是 Core Text + tiny-skia，两套渲染会有偏差，违背「预览与导出一致」。
2. `NSTextInputClient` 就是 macOS 的原生输入法协议：候选框位置（`firstRect(forCharacterRange:)`）、组合串高亮、退格和确认都由系统输入法驱动。它和 Windows 的 `WM_IME_COMPOSITION` 一一对应，`text_edit_state` 里的组合串模型可以直接复用。

若实测中文输入法有问题（例如候选框定位、搜狗/微信输入法兼容性），再退回到隐藏的 NSTextView 做输入代理，渲染仍走 tiny-skia。

### 3.4 键位

- Core 定义平台无关的 `Key` 枚举（Esc、Enter、方向键、空格、字母、数字、F 键）和修饰键位，交互代码只认 `Key`；Windows 的 `wndproc` 把 VK 翻译成 `Key`，Swift 把 `NSEvent.keyCode` 翻译成 `Key`。
- 编辑快捷键在 macOS 用 ⌘：⌘C 复制、⌘S 保存、⌘Z 撤销、⌘⇧Z 重做。Core 只接收「主修饰键」这一个概念（Windows = Ctrl，macOS = ⌘）。
- `Config` 里的热键字段保持 `key: u32 + modifiers: u32`，**按平台解释**：Windows 是 VK + `MOD_*`（现有配置文件不受影响），macOS 是 `kVK_*` + Carbon 修饰位；默认值按 `cfg` 区分。macOS 默认：⌃⌘A 截图、⌃⌥P 贴图，工具键 1–6，长截图键 S。两端配置文件路径不同，不存在互相读取的问题。

## 4. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| Mac 上没有 Xcode | 无法 `xcodebuild`，无法产出 `.xcodeproj` 的构建验证 | 用 Swift Package（`swift build`，Xcode 可直接打开 `Package.swift`），`package-macos.sh` 手工组装 `.app`。装上 Xcode 后补 `xcodebuild` 验证。**需要用户决定是否安装 Xcode** |
| 屏幕录制授权与签名绑定 | 临时签名（ad-hoc）每次重编 cdhash 都会变，授权失效，每次构建都要重新授权 | 在钥匙串建一个本地自签名证书「AirSnip Dev」固定签名身份，授权可跨构建保留；需要用户在 Mac 上操作一次 |
| GUI 无法远程验证 | 我只能通过 ssh 构建和启动，看不到屏幕，也不能代点权限 | 所有 GUI 项列为「待人工验证」并给出操作步骤与预期；不做未经同意的按键注入 |
| Overlay 拆分影响 Windows | 交互回归 | 先只做机械搬迁（字段换位置、方法签名不变），每步 Windows 全量测试；Windows 手动回归清单交给用户 |
| 混合缩放多屏 | 选区不能跨屏（§2.3） | 记录为已知差异；无外接屏时多屏项待人工验证 |
| ScreenCaptureKit 延迟 | 热键到覆盖层 Windows 红线是 150 ms（1080p），多屏或 5K 屏可能超标 | 并行抓各屏；先显示已抓到的屏幕；实机测量，不套用 Windows 数据 |
| 长截图每拍抓帧开销 | `SCScreenshotManager` 单帧调用若太慢，拼接会丢帧 | 先用单帧 API 实测；不够再换会话期间开一个 `SCStream` |
| Mac 的 Rust 是 1.96.1 | 代码若用到更新语法会编译失败 | M1 先按现有版本试编，失败再 `rustup update` |
| 部署目标 macOS 14，本机是 26 | 无法实测 14/15 | 编译时设 `-target arm64-apple-macos14.0`，可用性检查交给编译器；低版本实机项待人工验证 |

## 5. 阶段计划与验收

每阶段结束都跑：Windows `cargo test` + `cargo fmt --check` + `cargo clippy`（不新增 warning）；macOS `cargo test -p airsnip-core -p airsnip-macos` + `swift build`。GUI 项按清单人工验证。

| 阶段 | 内容 | 自动化验收 | 人工验收 |
|---|---|---|---|
| **M1** 共享核心 | workspace；`windows`/`windows-core`/`embed-resource` 改为 `cfg(windows)` 依赖；搬 `editor`、`render`（纯部分 + text 后端缝）、`longshot` 算法、编码、`Config` 模型进 `airsnip-core`；Core Text 测量/绘制；`airsnip-macos` 静态库 + `airsnip.h`；Swift 包能链接并调用一个函数 | Windows 测试 ≥198 全过；macOS core 测试全过（含 Core Text 渲染测试）；`swift build` 通过 | 无 |
| **M2** 截图 MVP | `Overlay` → `Session` + `Host` 拆分（Windows 行为不变）；菜单栏、⌃⌘A 热键、权限引导、每屏抓屏 + 冻结暗化、框选 / 八向调整 / 空格拖动 / 方向键、Esc 与右键取消、Enter / 双击复制 | 同上 + Session 的框选单测在 macOS 跑通 | Retina 截图像素正确、复制到微信/浏览器尺寸正确、授权拒绝→重新授权流程 |
| **M3** 标注 | 六种标注、对象选中 / 移动 / 缩放 / 圆角柄 / 曲线控制点、撤销重做、滚轮、工具键、Shift 约束、字体面板、文字二次编辑与中文输入法 | core 中 editor / render / objedit 测试 | 中文输入（拼音、选词、退格、改字、换行）、各标注视觉与 Windows 对照 |
| **M4** 输出 | 复制、PNG/JPEG/WebP、NSSavePanel、文件名模板、记住目录与格式 | 编码往返测试（尺寸、通道、透明度） | 保存对话框交互、大图导出 |
| **M5** 贴图 | NSPanel 贴图（截图贴、剪贴板贴）、拖动、滚轮缩放、透明度、边框开关、单张 / 全部关闭、上限 20 | `pin::scale` 测试 | 多贴图管理、缩放清晰度 |
| **M6** 长截图 | 视口选取、状态条、视口框、预览、抓帧排除自身窗口、撤销上一段、结束保存 | `longshot` 拼接测试（含模拟帧源） | 浏览器/文档实滚、固定页眉页脚识别 |
| **M7** 设置与集成 | 设置窗口（快捷键、保存、外观、贴图、关于）、自启、单实例、配置持久化、`package-macos.sh`、CI（Windows + macOS）、README | CI 两平台绿 | 重启后配置保留、开机自启、深浅色、休眠唤醒、显示器插拔、性能实测（启动延迟、拖动帧耗时、待机 CPU/内存、连续截图内存） |

## 6. 决定与待定事项

- **Xcode**：开发机不安装（已定）。本地用 Swift Package（`swift build`）开发；`xcodebuild`、签名与打包放到 GitHub Actions 的 macOS runner（自带 Xcode），M7 落地。
- **Fork**：目前在上游克隆上的本地分支 `feat/macos-port` 工作，推送前需要先创建 fork。
- **待确认**（M2/M3 开工前）：文本输入用 `NSTextInputClient`（§3.3）；多屏选区不跨屏（§2.3）。

## 7. 阶段记录

### M1 共享核心与可编译工程（2026-11-01）

**完成内容**

- Cargo workspace：根包 `airsnip`（Windows 外壳，所有依赖改为 `cfg(windows)`，非 Windows 平台编译为一个提示用的空入口）+ `crates/airsnip-core` + `crates/airsnip-macos`。`default-members` 为根包与 core，所以 `cargo test` 在两个平台都会跑 core 的测试。
- 用 `git mv` 移入 `airsnip-core`（保留历史）：`editor/`、`render/`（含图标）、`longshot/{engine,stitch,rows,session,debug}`、`settings` 配置模型、`output` 编码部分。根包以 `pub use airsnip_core::{editor, render}` 及各模块内的 `pub use` 再导出，原有 `crate::…` 路径不变，Windows 业务代码零修改。
- 文字后端拆分：`render/text/mod.rs` 是平台无关入口（`FontSpec`、`measure`、`remeasure`、`draw`、覆盖度混合）；`dwrite.rs` + `dwrite_dib.rs` 是原 DirectWrite 实现（逻辑不变，改为输出覆盖度）；`coretext.rs` + `coretext_ffi.rs` 是新的 Core Text 实现（直接声明系统 C 接口，无新增 crate）。默认字体：Windows 微软雅黑 UI，macOS PingFang SC；族名不存在时回退默认字体；苹方无斜体时用倾斜矩阵合成，缺粗体字重时用描边合成。
- 长截图 `Session` 只保留 `with_source`；屏幕帧源 `ScreenSource` 移回 Windows 根包。
- 配置目录按平台：macOS `~/Library/Application Support/AirSnip/`。
- `crates/airsnip-macos`：C ABI 静态库，导出 `airsnip_version` / `airsnip_last_error` / `airsnip_save_image`，`catch_unwind` 守卫；头文件 `include/airsnip.h`。新增 `[profile.release-macos]`（panic=unwind，不 strip）。
- `macos/`：Swift Package（`CAirsnip` 系统库模块 + `AirSnip` 可执行目标），M1 入口为桥接自检。
- 根包去掉已不直接使用的依赖（image-webp / jpeg-encoder / serde / toml）与 `Win32_Graphics_DirectWrite` 特性（core 自己开启）。

**实际执行的命令与结果**

| 平台 | 命令 | 结果 |
|---|---|---|
| Windows | `cargo test` | 根包 97 + core 103 = **200 通过**（原 198 + 新增 2：多行测量、字形落在测量框内） |
| Windows | `cargo test -p airsnip-macos` | 4 通过（版本号、按 stride 拷贝写 PNG、非法参数报错、panic 转错误码） |
| Windows | `cargo fmt --all --check` | 通过 |
| Windows | `cargo clippy --workspace --all-targets` | 47 条 warning，与 `main` 基线完全一致（全部是搬移前就有的），新代码 0 条 |
| Windows | `cargo build --release` | 通过；exe 1,119,232 字节（`main` 为 1,120,768 字节） |
| Windows | `cargo deny check`（targets 增加 `aarch64-apple-darwin`） | advisories / bans / licenses / sources 全部 ok；无新增第三方依赖，`THIRD-PARTY.md` 不变 |
| macOS | `cargo test` | 根包 0（空入口）+ core **102 通过**（比 Windows 少 1 项 DirectWrite 专属测试） |
| macOS | `cargo test -p airsnip-macos` | 4 通过 |
| macOS | `cargo clippy -p airsnip-core -p airsnip-macos --all-targets` | 仅 5 条已有 warning，Core Text 新代码 0 条 |
| macOS | `cargo build -p airsnip-macos --profile release-macos` | 通过，`libairsnip_macos.a` 5.9 MB；需链接 CoreFoundation / CoreText / CoreGraphics |
| macOS | `cd macos && swift build && .build/debug/AirSnip --selftest` | 通过：打印核心版本，经 Rust 写出 PNG，错误路径返回中文说明 |

开发中发现并修复的问题：`Cf::new` 最初写成 `then_some(Cf(p))`，空指针时临时对象被 Drop 导致 `CFRelease(NULL)` 崩溃（SIGTRAP）；改为惰性 `then(|| …)` 并加注释。

**未执行 / 待人工**

- `xcodebuild`：开发机无 Xcode，M7 在 CI 执行。
- Windows GUI 回归：文字相关路径（文本标注、字体面板、尺寸提示、长截图状态条、设置面板文字）改为经覆盖度混合，算法等价且单测覆盖，但需要人工确认一次显示效果（见下）。
- Core Text 与 DirectWrite 的排版并非逐像素一致（字体与 hinting 不同），这是预期差异；macOS 端的视觉检查放在 M3。

**Windows 兼容性影响**：业务代码路径未改；文字栅格化改为「后端输出覆盖度 → 统一混合」，混合公式原样搬移。

**Windows 人工回归清单**（每项 1 分钟内）：
1. F1 截图，框选后看尺寸提示文字是否清晰。
2. 文本工具输入中英文，切换字体、粗体、斜体，Enter 复制，粘贴到画图里对比。
3. 打开设置面板，各页文字正常。
4. 长截图状态条文字正常。

### M2 截图 MVP（2026-11-01，自动化部分完成，GUI 待人工验证）

**完成内容**

- 覆盖层拆分（ADR 0019）：交互、选区、工具栏、标注、字体面板、光标形状判定全部移入 `airsnip-core::overlay`；
  平台能力抽为 `Host` trait（上屏、光标、关闭、提示、复制、贴图、保存对话框、后台保存、输入法）。
  Windows 外壳（`src/overlay/`）只剩窗口创建、消息分发、`WinHost`（原 DIB/BitBlt、HCURSOR、IME、贴图载荷逐字搬运）、
  自绘光标与 `dib`。修饰键改为 `Overlay::mods`，由平台层在每条消息/事件前写入。
- 同时移入核心：`theme` 调色板、`perf` 埋点、`CapturedScreen`、默认保存目录与命名模板（本地时间按平台取）、
  保存格式/目录记忆。`Ui::auto(base_scale, w, h)` 供两平台共用。
- macOS 默认键位（kVK + Carbon 修饰位）：⌃⌘A 截图、⌃⌥P 贴图、⌃F3 贴图边框、数字 1–6 工具、S 长截图。
- `crates/airsnip-macos`：覆盖层 C ABI（创建/释放/借用上屏缓冲/鼠标/双击/右键/滚轮/按键/结果）、`MacHost`（回调表 `AsHost`）、
  macOS 按键分发（Esc、回车、⌘C、⌘S、⌘Z、⌘⇧Z、方向键 1/5px、工具键、长截图键；文本输入中的编辑键）。
- Swift 外壳（`macos/Sources/AirSnip/`）：菜单栏图标与菜单、Carbon 全局热键（注册失败提示）、屏幕录制权限检测与引导、
  ScreenCaptureKit 并行抓所有显示器（排除自身、不含光标、按原生像素、转 sRGB）、每屏一个覆盖窗口（屏蔽窗口层级、所有桌面空间）、
  脏区绘制、鼠标/拖动/双击/右键/滚轮（触控板归并成格）/键盘/空格抓手、系统光标映射、剪贴板（PNG + TIFF）、
  基础另存为对话框、提示框层级高于覆盖层、会话结束把焦点还给原应用。
- `scripts/package-macos.sh`：静态库 → `swift build -c release` → `dist/AirSnip.app` → 签名 → zip。开发构建为临时签名，
  指定要求固定为 bundle id `io.github.airsnip`，屏幕录制授权可跨构建保留。

**实际执行的命令与结果**

| 平台 | 命令 | 结果 |
|---|---|---|
| Windows | `cargo test` | 根包 54 + core 147 = **201 通过**（交互代码移入 core 后数量随之迁移；新增 `Ui::auto` 测试 1 项） |
| Windows | `cargo fmt --all --check` / `cargo deny check` | 通过 / 四项 ok |
| Windows | `cargo clippy --workspace --all-targets` | 41 条，全部为已有 warning（逐条核对过改动文件），新代码 0 条 |
| Windows | `cargo build --release` | exe 1,121,280 字节（`main` 1,120,768） |
| macOS | `cargo test` | core **144 通过**（Windows 专属 3 项不编译：DirectWrite 工厂、DPI 手动倍率、旧配置迁移） |
| macOS | `cargo test -p airsnip-macos` | 8 通过，含「拖出选区 → 回车复制，复制尺寸 = 选区」闭环与 ⌘ 组合/工具键分发 |
| macOS | `cargo clippy -p airsnip-core -p airsnip-macos --all-targets` | FFI 新代码 0 条 |
| macOS | `swift build`、`scripts/package-macos.sh` | 通过；`AirSnip.app` 948 KB，zip 500 KB；`--selftest` 通过 |
| macOS | `open dist/AirSnip.app` | 常驻运行，CPU 0%，无崩溃报告 |

**待人工验证（需要在 Mac 前操作）**

| 步骤 | 预期 |
|---|---|
| 首次按 ⌃⌘A | 系统弹「屏幕录制」授权框，随后 AirSnip 提示框；点「打开系统设置」跳到对应设置页 |
| 打开 AirSnip 开关 → 菜单栏剪刀图标 › 退出 → 重新打开 `dist/AirSnip.app` → 按 ⌃⌘A | 所有屏幕冻结并变暗，菜单栏与 Dock 也被盖住 |
| 拖出选区；拖八个手柄；按住空格拖动；方向键 / Shift+方向键 | 选区原图亮度、边框与手柄、「宽 × 高」（像素，Retina 下为 points × 2）；调整与移动跟手 |
| 回车 / 双击选区内 / ⌘C | 覆盖层消失、焦点回到原应用；粘贴到微信、备忘录、浏览器，图像清晰、内容与选区一致 |
| Esc / 右键 | 取消，不改剪贴板 |
| 外接屏（如有） | 两屏同时冻结；一块屏已有选区时，另一块屏按下无效 |
| 连续截图 10 次 | 无卡顿；活动监视器里 AirSnip 内存不持续增长 |
| 拒绝授权后再按 ⌃⌘A | 不重复弹系统框，只弹 AirSnip 指引 |
| 截图中点工具栏「贴图」 | 提示框显示在覆盖层之上，点「好」后可继续操作 |

**人工验证中发现并修复**

- 覆盖层上弹提示框（如点「贴图」）后卡死：`NSAlert.runModal` / `NSSavePanel.runModal` 会把窗口层级重置为模态面板级（8），
  事先设的 level 无效，提示框被压在覆盖层（屏蔽窗口层级）下面，看不见也点不到，模态循环又挡住覆盖层事件。
  改为进入模态循环后再抬层级（`Alerts.runAbove`）。排查方法：`sample <pid>` 看主线程是否停在 `runModal`，
  `CGWindowListCopyWindowInfo` 看各窗口的 layer。
- 默认截图快捷键按用户习惯从 ⌃⌥A 改为 ⌃⌘A。

**已知限制（后续阶段处理）**

- 文本工具只能进入输入态，字符输入（含中文输入法）在 M3 接 `NSTextInputClient`。
- 光标：四向移动用「张开的手」；macOS 14 没有公开的斜向缩放光标，退回十字（15 起用系统 frameResize）；圆角柄用手指。M3 改为核心自绘光标。
- 剪贴板图像未写入分辨率信息，粘贴到按 points 排版的应用（Pages / Keynote）会按像素显示成 2 倍大；M4 处理。
- 贴图、长截图按钮目前提示「将在 M5 / M6 提供」。
- 抓屏统一转 sRGB，P3 广色域外的颜色会被裁剪。

M2 人工验证已通过（2026-11-01），M1–M2 提交为 `29d1de3`。

### M3–M7 标注、输出、贴图、长截图、设置（2026-11-01，自动化部分完成，GUI 待人工验证）

**完成内容**

- **M3 标注**：标注、对象编辑、撤销重做、滚轮、工具键在 M2 已随覆盖层进入核心，这里补文字输入。
  覆盖视图实现 `NSTextInputClient`（`OverlayTextInput.swift`）：文字输入中按键先交 `interpretKeyEvents`，
  组合串经 `setMarkedText`、上屏经 `insertText` 送核心（`airsnip_overlay_text`），输入法不要的编辑键经 `doCommand`
  按原始键码交回核心按键分发；其余时候不经输入法，工具键、长截图键不会被中文输入法截走。
  候选框跟随光标（`airsnip_overlay_caret` ← 核心 `TextEdit::caret_rect`，与绘制光标共用）；
  `windowLevel()` 返回覆盖层层级，让输入法把候选框放在屏蔽窗口层级之上。⌘← / ⌘→ 为行首 / 行尾。
  光标保留系统光标（Retina 下本来清晰，Windows 自绘是为了随界面缩放）；圆角柄用手指，不移植 Windows 的自绘圆弧光标。
- **M4 输出**：另存为对话框加「格式」下拉（PNG / JPEG / WebP 无损，同 Windows），切换时扩展名随之更新；
  选过的格式与目录在释放覆盖层时写回配置（`airsnip_overlay_free`，M2 已有）。复制的 PNG 写入 pHYs（72 × scale DPI，
  `output::encode_png_scaled`），TIFF 随之带分辨率，Pages / Keynote 按屏幕上的大小显示。
- **M5 贴图**（`Pins.swift`）：`NSPanel`（浮动、不激活应用、所有桌面空间、`canHide = false` 不随应用隐藏），
  内容是图层，缩放由 Core Animation 合成，不移植 Windows 的重采样与预览 / 精修。截图贴图（原位置或居中，读配置）、
  ⌃⌥P 剪贴板贴图（按图片自带分辨率定大小）、拖动、滚轮 / 双指捏合缩放 0.1–8 倍、⌘ + 滚轮透明度、方向键、
  双击 / Esc 关闭、右键菜单（复制 / 保存 / 关闭 / 关闭全部）、⌃F3 切换全部边框并记住、上限 20 张。
  抓屏改为排除本应用除贴图外的窗口（贴图与 Windows 一样会被截进去）。
- **M6 长截图**（`Longshot.swift`、`LongshotViews.swift`）：核心 `Session` 的帧源是同步拉取，ScreenCaptureKit 是异步的，
  FFI 用「待取帧」槽位衔接（`airsnip_longshot_push`）。每 90ms 一拍（上一帧未处理完则跳过，只在会话期间运行），
  `SCStreamConfiguration.sourceRect` 只抓视口；视口外 2px 蓝框（不挡鼠标）、视口下方（放不下则上方）原生状态条
  [提示 · 撤销 · 完成 · 取消]、旁边缩略预览。会话期间注册 Return / Esc / ⌘Z 全局热键，焦点留在被滚动的应用。
  完成时等在途帧并补抓一帧，结果复制（带 Retina 分辨率）并保存 PNG 到默认目录（同 Windows）。
- **M7 设置与集成**：原生设置窗口（`Settings.swift`，工具栏切页：通用 / 快捷键 / 保存 / 外观 / 关于，选项同 Windows 面板），
  改动即时保存（`airsnip_settings_load` / `_save`），快捷键录制（`ShortcutField`，Esc 取消，录制期间暂停全局热键）
  与冲突检查（规则同 Windows `conflict_for`）。开机自启用 `SMAppService.mainApp`，以系统实际状态为准。
  再次打开 App（双击）→ `applicationShouldHandleReopen` 打开设置；直接运行二进制时若已有实例，交给它后退出。
  菜单栏菜单：截图 / 设置… / 退出。应用图标由 `assets/icons/logo-512.png` 经 `sips` + `iconutil` 生成，赞赏码随包分发。
  新增 `.github/workflows/ci.yml`：Windows（fmt / test / release）+ macOS（test / 打包 / 自检，上传 zip 构建产物）。

**实际执行的命令与结果**

| 平台 | 命令 | 结果 |
|---|---|---|
| Windows | `cargo test` | 根包 54 + core 148 = **202 通过**（新增 PNG 分辨率测试 1 项） |
| Windows | `cargo fmt --all --check` | 通过 |
| macOS | `cargo test -p airsnip-core -p airsnip-macos` | core **145**、FFI **11** 通过（新增：输入法组合 → 上屏 → 提交 → 双击再编辑；长截图推帧拼接 / 撤销 / 尺寸校验 / 缩略图；设置往返与越界夹取） |
| macOS | `scripts/package-macos.sh`、`--selftest` | 通过，无编译警告；`AirSnip.app` 1.2 MB，zip 664 KB |
| macOS | 热键 → 覆盖层可见（日志） | 两块屏：抓屏 72 ms + 建覆盖层并显示 23 ms ≈ **95 ms** |
| macOS | 待机（`vmmap --summary`、`top`） | Physical footprint **11.8 MB**，CPU **0.0%** |
| — | GitHub Actions | 未运行（仓库未推送）；首次推送后确认两个任务是否通过 |

**待人工验证**

| 功能 | 步骤 | 预期 |
|---|---|---|
| 中文输入 | 文字工具点选区 → 拼音输入、选词、退格改拼音、⌘← / ⌘→、⇧Return 换行、Return 提交 | 候选框在光标旁、不被遮住；组合串带下划线；提交后双击文字可再编辑 |
| 标注对照 | 六种工具各画一个，选中后拖手柄、圆角柄、曲线控制点，⌘Z / ⌘⇧Z | 与 Windows 一致 |
| 另存为 | ⌘S → 切格式 → 保存；再次截图 ⌘S | 扩展名随格式变；第二次默认上次的目录与格式 |
| 复制到 Pages / Keynote | 回车复制后粘贴 | 与屏幕上看到的大小一致 |
| 贴图 | 工具栏贴图；⌃⌥P 贴剪贴板；拖动、滚轮、捏合、⌘ + 滚轮、方向键、⌃F3（MacBook 需 fn）、右键四项、双击 / Esc | 截图结束后贴图仍在；第 21 张提示上限；新截图里能截到贴图 |
| 长截图 | 浏览器里框选正文 → S → 滚动 → Return | 蓝框、状态条、预览出现；剪贴板与「图片 › airsnip」各有一份完整长图；⌘Z 撤销一段；Esc 取消 |
| 设置 | 菜单栏 › 设置…；双击 AirSnip.app；改截图键、录到已占用的键、改主题 / 缩放 / 文件名、开机自启 | 改键立刻生效且菜单显示新键；冲突有提示；开机自启在「登录项」里出现 |

**已知差异与限制**

- 光标：四向移动用「张开的手」，圆角柄用手指（系统没有圆弧光标）。
- 长截图期间 Return / Esc / ⌘Z 是全局热键（同 Windows），会话结束即注销。
- 贴图边框画在内容边缘之内（1 物理像素），不像 Windows 向外扩 1px；导出（复制 / 保存）不含边框，与 Windows 相同。
- 构建产物为临时签名、未公证；正式分发需要 Developer ID 签名 + 公证（`AIRSNIP_SIGN_ID`）。
- 抓屏统一转 sRGB，P3 广色域外的颜色会被裁剪。

**代码审查后修复（2026-11-02）**

- 多屏时工具记忆被空白屏覆盖：释放覆盖层时只记有选区那块屏的工具。
- 另存为 / 提示框开着时按贴图热键会重入保存流程并提前释放覆盖层：模态窗口存在或会话收尾中时忽略贴图热键。
- 设置里录快捷键时关窗口，全局热键一直处于暂停：窗口失去焦点即结束录制并重新注册。
- 设置窗口：每次到前台重读开机自启状态；各页设 `preferredContentSize`。
- 长截图状态条与预览按屏幕可用区域（`visibleFrame`）摆放，不再被 Dock 挡住或跑出屏幕。
- 权限提示框开着时再按截图热键不再叠出第二个提示框。
- 截图期间按贴图热键（⌃⌥P）直接贴当前选区（对应 Windows 截图时按 F3）。
- 删除与 `airsnip_overlay_free` 重复的 `airsnip_session_end`。

### 窗口识别与全屏（PRD CAP-6 / CAP-7，2026-11-02，macOS 先行）

两项在 PRD 中为 P1，Windows 版此前也未实现。逻辑放在共享核心 `overlay/snap.rs`，平台层只需提供截图时的窗口矩形：

- 还没有选区时，鼠标下最前面的窗口（不在任何窗口上则为整屏）恢复原图亮度、描边并显示尺寸；
  单击选中，按下后拖出 4px（`CLICK_SLOP`）以上才开始手动框选。选中后在选区内双击即复制，
  所以「双击窗口 / 双击桌面」= 一步截窗口 / 截全屏。⌘A 全选整屏。
- 平台层未调用 `set_windows` 时功能关闭，交互与之前完全一致：**Windows 外壳暂未接入**
  （需要 `EnumWindows` + `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` 取窗口矩形，另加 Ctrl+A），行为不变。
- macOS：截图前用 `CGWindowListCopyWindowInfo` 取当前桌面空间的窗口（层级 0–3、≥ 20pt、不透明度 > 0，
  排除本应用除贴图外的窗口），按 `CGDisplayBounds` 换算成各屏像素交给 `airsnip_overlay_set_windows`。
  多屏时只有鼠标所在屏高亮（`mouseExited` → `airsnip_overlay_leave`），别的屏已有选区时本屏不高亮。
- 测试：`keys::tests::click_selects_window_or_full_screen`（单击含抖动选窗口、空白处整屏、拖动框选、⌘A、未提供窗口列表时保持旧行为）。
- 已知限制：取的是窗口外框，不识别窗口内部控件；全屏模式的 Chrome 顶部有一条 124pt 的工具栏窗口，鼠标停在那一条时只高亮它。
