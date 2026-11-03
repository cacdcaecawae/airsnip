# AirSnip（轻截）开发指南

## 项目概览

轻量原生截图与标注工具（Windows 11，Rust + windows-rs，纯 CPU 渲染），面向无独显迷你主机。目标：常驻 ≈0 开销、触发即用、框选/箭头/文本/高斯模糊/高亮标注、长截图、贴图。

软件名 **AirSnip（轻截）**：展示层用 `AirSnip` / `AirSnip 轻截`，技术标识（crate、exe、仓库、配置目录、任务计划、窗口类、文件名前缀）一律小写 `airsnip`。v0.5.0 之前叫 `jietu`，改名决策与存量影响见 `docs/adr/0014-rename-to-airsnip.md`。

完整需求见 `screenshot-tool-PRD.md`，架构决策见 `docs/adr/`。

## 项目状态

- 当前版本 **v0.7.0**：M0–M4 与 M5a/M5b 全部交付并合并，`main` 已合并 PR #32–#66（release 版黑窗修复、设置窗口比例与图标、v0.3.1–v0.6.0 发布、开机自启托盘图标消失修复 + `WM_SETTINGCHANGE` 指针加固、`app/mod.rs` 拆分、长截图改纯手动滚动 + 拼接预览 + 输入法屏蔽、保存对话框记住上次目录、界面图标统一、圆角内部控制柄 + 扩展到四类、README 重写 + 头图、提权进程托盘图标自愈、改名 AirSnip + 新应用图标、选区尺寸提示收尾、快捷键清空为「未设置」+ 恢复默认 + 托盘入口、长截图另存为、自动更新），工作区干净。M5a 交付要点：另存为对话框（PNG / JPEG / WebP 无损 + 文件名模板）、选中对象包围框与八向手柄 + `Shift` 约束（`overlay/objedit/`、`editor/transform.rs`）、方向键微调（1px / `Shift` 5px）、系统字体枚举 + 工具栏第二行字体面板（`render::fonts`、`overlay::font`、`overlay::toolbar::fontlist`）；决策见 `docs/adr/0007-drop-delete-and-layering.md`、`0008-font-picker.md`、`0010-radius-handle.md`、`0011-corner-all-kinds.md`；`cargo test` 220 项绿（另 3 项需手动触发的集成测试：真实网络请求、发布清单验签闭环、线上 Release 端到端，`--ignored`）。
- M5 打磨进度：✅ 许可证合规（`deny.toml` + `cargo deny check` 四类全绿；`THIRD-PARTY.md` 由 `scripts/gen-third-party.ps1` 生成；设置面板「关于」页含项目地址、微信赞赏码（`assets/reward-code.png` 编译期嵌入）与第三方声明入口）；✅ 性能剖析（`docs/perf-m5.md`：热键→可见 124ms、交互 4.6ms/帧、待机 0% CPU / 1.2MB，全部红线达标；埋点 `src/perf.rs`，采集 `scripts/perf-capture.ps1` + `scripts/perf.ps1`）；✅ 打包分发（`scripts/package.ps1` → `dist\airsnip-v{版本}-win-x64.zip` + 自动更新用 `.exe`，图标 16–256 八尺寸）；⬜ 多屏混合 DPI 测试（**当前无多屏环境，暂跳**）。
- 自动更新（v0.7.0 交付，看板 #59，ADR 0018）：`src/update/` 九个子模块 —— `manifest`（清单 + ed25519 验签）、`version`（三段式版本比较，预发布不推正式版）、`endpoints`（三个 gh-proxy 镜像 + GitHub 直连兜底）、`http`（WinHTTP，不引 reqwest）、`install`（边下边算 SHA-256 + cmd 脚本替换 exe）、`check`（按镜像顺序取清单）、`state`（跨线程进度）、`notify`（后台线程唤醒界面）。
  - **验签优先于一切字段**：签名载荷为 `airsnip-update-v1\n<版本>\n<小写摘要>`，版本号参与签名，把「新版本号 + 旧包」拼一起会验签失败；纯 hash 不作为安全判断（清单与包同源，能改包的人也能改 hash）。
  - **清单存 GitHub 规范地址**，加速前缀由客户端套用（`endpoints::candidates`），换镜像域名不用重新发版 —— 与 issue 建议的「清单里写死加速地址」不同，理由见 ADR 0018 §4。
  - **更新包是裸 exe**：`package.ps1` 同时产出 zip（手动下载，含文档）与 exe（自动更新），清单指向后者。
  - **替换流程**：下载到 `%TEMP%` → 校验和比对 → 写 `.cmd` → 主进程退出 → 脚本等进程消失后 `copy /y` 覆盖并重启 → 自删；`copy` 失败仍启旧版（避免「更新完打不开」）。
  - **触发**：启动后 3s 一次性 `SetTimer`（回调里立刻 `KillTimer`，不引入常驻轮询），失败静默，发现新版本弹托盘气泡；设置面板「关于」页显示状态并可手动检查 / 一键更新，另有「启动时自动检查」开关。
  - **密钥管理**：私钥在 `%USERPROFILE%\.airsnip\update.key`（**不在仓库**），公钥写死在 `src/update/manifest.rs` 的 `UPDATE_PUBKEY_HEX`；生成用 `cargo run --example update-keygen`，签名用 `cargo run --example update-sign -- --version <ver> --sha256 <摘要>`。**换密钥等于切断旧版本的自动更新能力**。
- 界面图标统一（ADR 0009）：覆盖层工具栏与长截图状态条的图标改为 `assets/icons/ui/` 下的 SVG 资源（`render/icon/` 解析 + 按包围盒归一化绘制），删掉 17 个手绘几何图标与重复实现；`overlay/shape.rs` 承接共用绘制小工具。规格与素材来源见 `assets/icons/ui/README.md`，视觉重量由 `render::icon` 的 4 项单测守门。**v0.6.0 换源（ADR 0015）**：原先 11 个图标来自 iconfont 多作者素材、来源与授权无法确认；现全组 19 个图标统一换成 **Material Symbols（outlined，Apache-2.0）**，风格从「粗细不一的混搭」变为统一的线框风，许可证链也干净了（已在 `THIRD-PARTY.md` 登记）。
- 圆角改为对象上的内部控制柄（ADR 0010）：删掉工具栏二值「圆角」开关（`Action::ToggleRound` / `State.round` / `round.svg`），选中对象时在包围框内侧右上角显示一个白色小圆，朝中心拖增大、朝角拖减小，上限短边一半；新增自绘圆角光标（`overlay/cursor`）。圆角读写集中在 `editor/corner.rs`。**已扩展到四类**（`feat/corner-all-kinds`）：矩形 / 高亮 / 高斯模糊 / 聚光灯共用同一 `corner` 字段与手柄，三处渲染（高亮逐像素插值、模糊备份后按遮罩恢复、聚光恢复亮度）统一走 `render/mask.rs`；`corner == 0` 仍走原快速路径。
- 长截图与输入法打磨（待验证）：自动滚动模式已移除（靠「连续静帧」判定到底会误判，拼接提前结束），改为纯手动滚动 + `Enter` 收尾；长截图键从硬编码 `S` 改为配置项 `hotkey.longshot_key`（默认 `S`，设置面板「快捷键」页可改并参与冲突检测）；截图覆盖层打开后禁用 IME，仅在文本输入时恢复，避免 `S`/回车等按键被输入法截走。
- 长截图结果预览：`longshot/preview.rs` 在视口左/右剩余空间开一个分层预览窗（底边与视口底边对齐、背景透明，只绘缩略图），显示拼接结果的等比缩略图（`Canvas::thumbnail` 按目标像素抽样，成本与图高无关）；状态条不再显示像素计数，只留操作提示。
- 已知问题记录：覆盖层线程曾因 `WM_DESTROY` 同步派发导致 `GetMessageW` 永久阻塞，每次截图泄漏一份全屏位图集合（每张 +57MB），已在 `wndproc.rs` 的 `WM_DESTROY` 分支加 `PostQuitMessage(0)` 修复；修复后截图退出常驻内存稳定在 ~24MB（debug，代码页 + 堆缓存）。
- 已知问题记录（已修）：开机自启后托盘图标消失的根因是 `theme::is_theme_change` 把 `WM_SETTINGCHANGE` 的 `lParam` 无条件当宽字符串读（`PCWSTR::to_string` → `wcslen`），登录阶段系统/第三方广播 `lParam = 0` 时 `wcslen(NULL)` 访问冲突（`0xC0000005`，`ucrtbase.dll+0x7088f`）直接杀掉常驻进程 —— 任务计划本身是成功的。已改为 `VirtualQuery` 校验内存可读性 + 区域内限长读取（不再走 `wcslen`，垃圾指针与无终止符字符串均安全返回 false），并补 `TaskbarCreated` 广播兜底（explorer 就绪/重启后重新 `NIM_ADD`，见 `app::tray::is_taskbar_created`）。
- 开发由 AI 驱动，本文件是进入此仓库的 agent 的上下文。
- 改名 AirSnip（ADR 0014，v0.5.0 之后）：`jietu` → `airsnip` 全量替换 32 个文件 / 86 处，含 crate 名、exe、仓库地址、配置目录 `%APPDATA%\airsnip`、任务计划名、单实例 mutex、窗口类、保存目录 `图片\airsnip`、文件名模板、环境变量 `AIRSNIP_TIMING` / `AIRSNIP_DEBUG`。**未做存量迁移**：升级后需手动删旧任务计划（`schtasks /delete /tn jietu /f`，需管理员）、退出旧版 exe（mutex 已改名，新旧可并存抢热键）、重设一次设置；旧截图留在 `图片\jietu`。`scripts/perf-capture.ps1` 依赖窗口类名与 `[airsnip]` 日志前缀，脚本须与程序同版本。
- 选区提示只留「宽 × 高」+ 字体加大（PR #50，v0.6.0 收尾）：
  - 删掉 HUD 后半段的 `@(x,y)` —— 那是虚拟桌面物理像素，普通用户看不懂。`surface::draw_info_text` 与 `geometry::text_rect()` 两处格式串同步改为 `format!("{} × {}", sel.w, sel.h)`（后者用于估算文本宽度供脏区与工具栏避让），需求侧 `screenshot-tool-PRD.md` 的 `CAP-2` 同步改为「显示选区尺寸」。
  - 字号从 `DEFAULT_GUI_FONT`（11px）提到 **16px 加粗**（`INFO_FONT_SIZE`），信息块高 `INFO_TEXT_H` 22 → 28。
  - **不再用 GDI 文本**：原 `TextOutW` + 灰度抗锯齿在 16px 下 hinting 会把笔画吸到整像素，同一串数字里有的 1px 有的 2px（看起来粗细不均）；关 ClearType 避彩边后更明显。现改为走 `render::text`（DirectWrite 栅格化，与工具栏/标注文字同一套），画在 `display` Pixmap 上、随脏区一起 `present_region` 上传，`surface::draw_info_text` 不再接 `HDC`。
  - 文字与工具栏的避让抽为 `geometry::info_text_below`（两处共用同一侧向决策），`text_rect` 增 `screen_h` / `toolbar_h` 参数；工具栏仍是**选区下方优先**，下方塞不下才翻到上方，并新增 `toolbar_defaults_below_selection` 测试守住该行为。
- 覆盖层界面按显示器缩放（v0.7.0，ADR 0016）：设置面板「外观」页新增「界面缩放」（自动 / 100% / 125% / 150% / 175% / 200%，默认自动），`overlay/ui.rs` 的 `Ui` 把 1080p / 100% 基准像素换算成当前屏幕物理像素（`ui.px()` / `ui.f()`），所有界面尺寸常量改为 `*_BASE`。
  - **自动 = `max(显示器DPI/96, (屏幕面积/1920×1080)^0.3)`**：纯 DPI 方案在 **4K@100%** 下系数仍是 1.0（DPI 就是 96），分辨率项兜底到约 1.52；取较大值保证 4K@150% / 200% 不被重复放大。指数 0.3 而非 0.5 —— 4K@100% 要的是「别太小」，不是「等比放大一倍」。
  - 系数按**光标所在显示器**取（`GetCursorPos` + `MonitorFromPoint` + `GetDpiForMonitor` + `GetMonitorInfoW`）；覆盖层横跨虚拟屏，`GetDpiForWindow` 只会给主屏。夹在 `[1.0, 3.0]`。
  - 覆盖范围：工具栏、选区/对象手柄、尺寸提示、字体面板、长截图状态条 / 视口边框 / 预览窗、自绘光标（十字 / 抓手 / 圆角）。**标注内容（线宽、字号、文字）不缩放**，它们是导出内容，跟着 DPI 变会改输出结果。
  - 自绘光标按画布尺寸缓存，换屏导致尺寸变化时先 `DestroyCursor` 再重建；窗口类 `hCursor` 改用系统 `IDC_CROSS` 兜底，避免窗口类持有会被销毁的句柄。
  - 设置面板也用同一套系数（`Panel::scale = Ui::resolve(config.ui_scale).scale()`，不再是 `GetDpiForWindow/96`）：在面板里选完倍率，面板当场就缩放（`fit_window` 重建 DIB + 重排窗口 + 夹回屏幕内、清空 `hover`），不必关面板再截图。

## 技术栈

- 语言：Rust 1.98（edition 2024）
- 系统 API：`windows` crate（windows-rs 0.62.2）
- 渲染：`tiny-skia` 0.12（纯 CPU 矢量绘制）+ DirectWrite 文本栅格化
- 配置/编码：`serde` + `toml`、`png`

## 常用命令

- 编译检查：`cargo check`
- 运行：`cargo run`（GUI 桌面程序，需 Windows 桌面会话；启动后托盘常驻，F1 截图 / F3 贴图）
- 构建发布版：`cargo build --release`（已配 `lto="fat"`、`opt-level="z"`、`panic="abort"`、`strip=true`）
- 测试：`cargo test`（workspace 默认成员 = 根包 + `airsnip-core`；Windows 当前 224 个：根包 76 + core 148（macOS 上 core 145；根包另有 3 个需手动触发的更新集成测试），覆盖抓屏/剪贴板/对象模型与撤销栈/矢量栅格化/曲线与命中/文本测量与字体（粗斜体渲染、族间测量、未知族名回退、多行、落点）/系统字体枚举/长截图拼接/WebP/变换与手柄/光标映射/布局/字体面板/设置面板布局与 DPI 缩放/设置变更消息内存校验/图标 SVG 解析与视觉重量/圆角读写与手柄/圆角遮罩覆盖率与回贴）；FFI 静态库另跑 `cargo test -p airsnip-macos`（12 个，仅 macOS）
- 格式化检查：`cargo fmt --all --check`（提交前必须通过；`--all` 覆盖 workspace 内全部 crate）
- macOS 版（移植中，见 `docs/macos-port.md`）：`cargo build -p airsnip-macos --profile release-macos` 构建静态库，再 `cd macos && swift build`；平台无关代码放 `crates/airsnip-core`，根包 `src/` 只放 Windows 外壳
- 性能采集：`scripts/perf-capture.ps1`（热键延迟 + 帧耗时，需管理员）与 `scripts/perf.ps1`（待机 CPU/内存）；报告见 `docs/perf-m5.md`
- 打包发布：`scripts/package.ps1`（构建 release → 校验体积/图标 → 输出 `dist\airsnip-v{版本}-win-x64.zip` 与自动更新用的 `.exe`）
- 生成更新清单：`scripts/gen-latest-json.ps1 -Notes "说明"`（算 SHA-256 → 调 `update-sign` 签名 → 输出 `dist\latest.json`，需先跑 package.ps1）
- 更新密钥：`cargo run --example update-keygen`（私钥写 `%USERPROFILE%\.airsnip\update.key`，公钥打印出来填进 `UPDATE_PUBKEY_HEX`）
- 许可证与依赖检查：`cargo deny check`（配置在 `deny.toml`；依赖变更后重跑 `scripts/gen-third-party.ps1` 更新 `THIRD-PARTY.md`）

## 开发流程（AI 驱动约定）

1. **禁止在主分支（main）直接开发**：任何功能、修复、优化都必须新建分支（命名建议 `feat/*`、`fix/*`、`docs/*`），完成后以 Pull Request 提交、经审查后合并回 main。main 只接受 PR 合并，不允许本地直接 push。
2. **先验证，后提交**：完成实现后先自行编译/测试，然后把改动交给用户手动验证（运行、体验、确认交互）。**未得到用户明确确认前，禁止执行 `git commit`、`git push`、创建 PR 或合并**。只有用户说“验证通过 / 可以提交”后才走提交与 PR 流程。
3. 按里程碑推进（PRD §7：M0 → M5，二期另行），每次只实现当前里程碑需求，不做投机性代码。
4. 实现功能前先同步文档：不可逆/高杠杆决策记入 `docs/adr/NNNN-*.md`；需求变更落在 PRD 对应编号。
5. 每个源文件保留头部 `SPDX-License-Identifier: GPL-3.0-only`。
6. 引入新依赖前确认许可证与 GPL-3.0-only 兼容，并说明用途。
7. `windows` crate 按需开启 features，不一次性全量开启。

## 代码约定

- 目录结构固定为 PRD §6.2 模块划分：`app/`、`capture/`、`overlay/`、`pin/`、`editor/`、`render/`、`longshot/`、`output/`、`settings/`、`theme/`、`ocr/`（二期）。
- **单文件源码 ≤300 行**（硬性；`#[cfg(test)]` 测试块不计入）。超出时按职责拆出子模块（如 `overlay/` 下的 `wndproc/geometry/surface/cursor/selection/interaction/annotate/toolbar/`），子模块内 `pub(super)` 共享父模块实现。
- 模块 doc 注释中标注对应需求编号（SYS-1、CAP-2、EDT-1…），便于追溯。
- 标注采用矢量对象模型（底图不变，对象列表叠加），渲染用脏矩形局部重绘。
- 所有坐标统一为物理像素（Per-Monitor V2 manifest）。
- 一期界面仅简体中文，代码层预留 i18n。

## 性能红线（必须守住）

- 待机 CPU 0%（无轮询、无定时器，长截图期间除外）
- 待机内存（Private Working Set）≤ 15 MB
- 热键到覆盖层显示 ≤ 150 ms（1080p）/ ≤ 250 ms（4K）
- 交互 ≥ 60 FPS，不依赖 GPU 合成

## 注意事项

- **程序始终以管理员权限运行**（manifest `requireAdministrator`，见 ADR 0003）：不要改回普通权限，否则无法覆盖/操作管理员窗口；开机自启必须用任务计划（`schtasks`）而非 HKCU Run 键。
- 无 GPU 可用：模糊、缩放、重采样等重操作需盒式近似 + 局部缓存 + 低质量预览，禁止全帧逐像素重算。
- 全局热键 F1/F3 会抢占其他程序按键，设置面板必须给出提示；注册失败要检测并提示冲突。
- 长截图（M4）是最大风险点：采用纯手动滚动（自动滚动易误判到底，已移除），拼接要处理固定区域（页眉/页脚），失败提供撤销最后一段入口。
- 贴图窗口用 `WS_EX_TOPMOST | WS_EX_TOOLWINDOW` 分层窗口，窗口静止时禁止任何重绘。
- 二期 OCR/翻译涉及第三方服务：默认关闭、首次显式授权、密钥用 DPAPI 加密、不留存请求内容。
- 自动更新私钥（`%USERPROFILE%\.airsnip\update.key`）**绝不入库、不进日志、不外传**；它同时决定「旧版本能否继续自动更新」，丢了只能让用户手动下载新版。
- 发布前跑 `cargo deny check`（配置 `deny.toml`）；依赖变更后用 `scripts/gen-third-party.ps1` 重新生成 `THIRD-PARTY.md`。
