// SPDX-License-Identifier: GPL-3.0-only
//! 应用壳（SYS-1/2/3/4/6）：单实例、隐藏消息窗口、托盘、热键、自启、主题变更。
//! 拆分：`run` 入口与 `App` 状态见本文件，窗口过程在 `wndproc`，托盘在 `tray`，热键在 `hotkey`，
//! 自启在 `autostart`。

pub mod autostart;
pub mod filedlg;
pub(crate) mod hotkey;
pub mod open;
pub mod popmenu;
mod tray;
mod wndproc;

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::settings::Config;
use crate::theme;
use wndproc::create_message_window;

/// 消息窗口类名（`FindWindowW` 唤起已有实例时用到）。
pub const WINDOW_CLASS: PCWSTR = w!("airsnip.hidden");
const MUTEX_NAME: PCWSTR = w!("Local\\airsnip.single.instance");
const WM_TRAY: u32 = WM_APP + 1;
const WM_ACTIVATE: u32 = WM_APP + 2; // 单实例唤起已有实例
/// 覆盖层线程 → 主窗口：贴图载荷（GlobalAlloc: u32 w + u32 h + RGBA）（PIN-1）。
pub const WM_PIN_FROM_EDITOR: u32 = WM_APP + 3;
/// 覆盖层线程 → 主窗口：截图已结束，重注册被撤销的贴图热键（PIN-1 防 F3 双触发）。
pub const WM_OVERLAY_DONE: u32 = WM_APP + 4;
/// 贴图窗口 → 主窗口：贴图内边框快捷键已切换（更新默认配置，M2b）。
pub const WM_PIN_BORDER_CHANGED: u32 = WM_APP + 5;

/// 截图覆盖层是否在运行（跨线程；截图期间撤销全局贴图热键）。
static OVERLAY_RUNNING: AtomicBool = AtomicBool::new(false);

pub const ID_TRAY: u32 = 1;
pub const ID_HOTKEY_SHOT: i32 = 101;
pub const ID_HOTKEY_PIN: i32 = 102;

/// 托盘菜单项 id（供 `tray` 子模块使用）。
pub const IDM_SHOT: usize = 40001;
pub const IDM_PIN: usize = 40002;
pub const IDM_SETTINGS: usize = 40003;
pub const IDM_EXIT: usize = 40004;

/// 启动后延迟检查更新的定时器 id（SYS-7）：一次性触发后立即 KillTimer，不常驻轮询。
pub const TIMER_UPDATE_CHECK: usize = 0x7B01;

/// 启动后延迟多久才检查更新（毫秒）：错开热键/托盘初始化，不影响启动耗时。
const UPDATE_CHECK_DELAY_MS: u32 = 3_000;

/// 应用运行时状态。
pub struct App {
    pub config: Config,
    pub hwnd: HWND,
    pub hinstance: HINSTANCE,
    /// 设置面板是否已打开（防托盘菜单重入，SYS-3）。
    pub settings_open: bool,
    /// 托盘图标补加重试次数（explorer 未就绪时用定时器重试，SYS-1）。
    pub tray_retry: u8,
}

/// 入口：单实例检查 → 创建消息窗口 → 托盘 / 热键 → 消息循环。
/// 返回进程退出码。
pub fn run() -> i32 {
    if !acquire_single_instance() {
        // 已有实例在运行，已向其发送唤起消息，本进程直接退出。
        return 0;
    }

    let config = Config::load();
    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }.unwrap_or_default().into();

    let hwnd = match create_message_window(hinstance) {
        Ok(hwnd) => hwnd,
        Err(msg) => {
            message_box(None, &msg, MB_OK | MB_ICONERROR);
            return 1;
        }
    };

    let mut app = Box::new(App {
        config,
        hwnd,
        hinstance,
        settings_open: false,
        tray_retry: 0,
    });
    // 先存窗口用户数据：wndproc 通过它访问 App，而 ensure_tray / register_hotkeys
    // 都可能触发消息（如 explorer 刚就绪时的 TaskbarCreated），早于此刻会丢图标。
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *app as *mut App as isize);
    }
    app.ensure_tray();
    app.maybe_warn_no_modifier_clash();
    app.register_hotkeys();
    // 更新检查的后台线程要通知主窗口弹气泡（SYS-7）
    crate::update::notify::register_main(hwnd);
    // 贴图内边框快捷键（M2b，非全局热键：仅贴图窗口聚焦时生效）
    crate::pin::set_border_key(app.config.hotkey.border_key, app.config.hotkey.border_modifiers);
    // 启动后延迟检查更新（SYS-7）：一次性定时器，不引入常驻轮询（其他性能红线不变）
    if app.config.auto_check_update {
        unsafe {
            SetTimer(Some(hwnd), TIMER_UPDATE_CHECK, UPDATE_CHECK_DELAY_MS, None);
        }
    }

    unsafe {
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 == 0 {
                break; // WM_QUIT
            }
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
    // 窗口随 WM_CLOSE 销毁（WM_DESTROY → PostQuitMessage 退出循环），这时
    // 不能再经 GetWindowLongPtrW 取回 Box（句柄已无效，返回值不可靠），
    // 直接用局部变量恢复 Box 并移除托盘图标。
    app.remove_tray();
    0
}

/// 单实例：命名 Mutex 已存在说明已有实例，唤起后返回 false。
fn acquire_single_instance() -> bool {
    match unsafe { CreateMutexW(None, false, MUTEX_NAME) } {
        Ok(mutex) => {
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                // 句柄无 Drop（Copy 类型），let _ 保持句柄打开直到进程退出，
                // 确保 mutex 对象存活，单实例有效。
                let _ = mutex;
                let hwnd = unsafe { FindWindowW(WINDOW_CLASS, None) }.unwrap_or_default();
                let _ = unsafe { PostMessageW(Some(hwnd), WM_ACTIVATE, WPARAM(0), LPARAM(0)) };
                false
            } else {
                let _ = mutex; // 保持到进程退出
                true
            }
        }
        Err(_) => true, // 创建失败不阻塞运行，退化为无单实例保护
    }
}

impl App {
    pub fn start_capture(&self) {
        crate::perf::mark_capture_start();
        let hinstance = self.hinstance.0 as usize;
        let main_hwnd = self.hwnd.0 as usize; // HWND 非 Send，跨线程传原生值
        let max_height = self.config.longshot_max_height;
        // 截图期间撤销贴图全局热键：F3 在覆盖层内走「贴选区」（PIN-1），
        // 否则 WM_HOTKEY 与覆盖层 WM_KEYDOWN 同时收到 F3 会双重触发（一次贴选区一次贴剪贴板）。
        OVERLAY_RUNNING.store(true, Ordering::Release);
        let _ = unsafe { UnregisterHotKey(Some(self.hwnd), ID_HOTKEY_PIN) };
        // 截图开始时解析一次主题调色板（M3：跟随设置，浅色模式工具栏为浅色）
        let palette = crate::theme::palette(crate::theme::resolve(self.config.theme));
        // 工具切换键配置 + 默认工具（记住上次使用，M3）
        let tool_keys = self.config.tool_keys.clone();
        // 长截图键（覆盖层内按下，默认 S）
        let longshot_key = self.config.hotkey.longshot_key;
        // 覆盖层界面缩放（CAP-5）：随配置快照传入覆盖层线程
        let ui_scale = self.config.ui_scale;
        let default_tool = crate::settings::last_tool();
        // 命名模板与上次保存格式/目录（OUT-2）：保存对话框预填文件名、默认筛选器与初始位置
        let save = crate::settings::SavePrefs {
            name_style: self.config.name_style,
            last_ext: self.config.last_save_ext.clone(),
            last_dir: self.config.last_save_dir.clone(),
        };
        // 长截图完成后也要弹同一个对话框（「另存为」入口），单独快照一份
        let longshot_save = save.clone();
        std::thread::spawn(move || {
            let hinstance = HINSTANCE(hinstance as *mut _);
            let main_hwnd = HWND(main_hwnd as *mut _);
            match crate::capture::capture_virtual_screen() {
                Ok(screen) => {
                    let result = crate::overlay::run(
                        screen,
                        palette,
                        tool_keys,
                        longshot_key,
                        default_tool,
                        save,
                        ui_scale,
                        hinstance,
                        main_hwnd,
                    );
                    // M4：工具栏长截图按钮 / 长截图键返回视口选区，进入长截图拼接。
                    if let crate::overlay::OverlayResult::Viewport((x, y, w, h)) = result {
                        crate::longshot::run(x, y, w, h, hinstance, max_height, palette, ui_scale, longshot_save);
                    }
                }
                Err(e) => message_box(None, &format!("抓屏失败：{e}"), MB_OK | MB_ICONERROR),
            }
            OVERLAY_RUNNING.store(false, Ordering::Release);
            // 通知主线程重注册贴图热键（截图已结束）
            let _ = unsafe { PostMessageW(Some(main_hwnd), WM_OVERLAY_DONE, WPARAM(0), LPARAM(0)) };
        });
    }

    pub fn not_yet(&self, feature: &str, milestone: &str) {
        let text = format!("{feature}尚未实现（{milestone}）。");
        message_box(Some(self.hwnd), &text, MB_OK | MB_ICONINFORMATION);
    }

    /// 打开设置面板（M3）：主线程模态运行，防重入。
    pub fn open_settings(&mut self) {
        if self.settings_open {
            return;
        }
        self.settings_open = true;
        crate::settings::panel::run(self);
        self.settings_open = false;
    }

    /// 应用设置面板的工作副本：写回配置、保存、重注册热键。
    pub fn apply_panel_config(&mut self, cfg: &Config) {
        // 保存格式与目录是运行时记忆（面板里没有对应设置项），不能被面板的旧副本覆盖（OUT-2）
        let last_save_ext = self.config.last_save_ext.clone();
        let last_save_dir = self.config.last_save_dir.clone();
        self.config = cfg.clone();
        self.config.last_save_ext = last_save_ext;
        self.config.last_save_dir = last_save_dir;
        self.config.save();
        self.register_hotkeys();
        // 贴图内边框快捷键（M2b）：立即下发给贴图窗口（含已存在的）
        crate::pin::set_border_key(cfg.hotkey.border_key, cfg.hotkey.border_modifiers);
    }

    /// 贴图边框开关（贴图窗口内快捷键 → 本消息触发）：
    /// 切全部贴图的边框显示，并把新状态写入默认配置（后续新贴图跟随）。
    pub fn toggle_pin_border(&self) {
        let next = crate::pin::toggle_border();
        if let Some(next) = next {
            let mut cfg = self.config.clone();
            cfg.pin_border = next;
            cfg.save();
        }
    }

    /// WM_SETTINGCHANGE：主题变更时刷新（SYS-6）。
    fn handle_setting_change(&self, lparam: LPARAM) {
        if theme::is_theme_change(lparam.0 as usize) {
            // M0 记录当前主题；设置面板（M3）与覆盖层（M1）消费。
            let _ = theme::system_theme();
        }
    }

    /// 启动后静默检查更新（SYS-7）：失败不打扰用户（国内网络下加速站挂掉是常态）。
    pub fn check_update_silently(&mut self) {
        let skipped = self.config.skipped_version.clone();
        crate::update::check::check_in_background(skipped);
    }
}

/// 程序主图标资源（app.rc 中的 ID=1）。
pub fn icon_resource() -> PCWSTR {
    PCWSTR::from_raw(1 as *const u16)
}

/// 弹消息框（UTF-8 → UTF-16 缓冲随调用存活，避免临时指针悬垂）。
/// `owner` 为 None 时使用当前活动窗口；`flags` 为 MB_* 组合。
pub fn message_box(owner: Option<HWND>, text: &str, flags: MESSAGEBOX_STYLE) {
    let ws: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe { MessageBoxW(owner, PCWSTR(ws.as_ptr()), w!("AirSnip"), flags) };
}

/// 将字符串写入固定长度 u16 数组（用于 NOTIFYICONDATAW.szTip 等）。
pub fn wide_array<const N: usize>(s: &str) -> [u16; N] {
    let mut arr = [0u16; N];
    let mut i = 0;
    for u in s.encode_utf16().take(N - 1) {
        arr[i] = u;
        i += 1;
    }
    arr
}
