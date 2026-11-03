// SPDX-License-Identifier: GPL-3.0-only
// AirSnip 共享核心 C ABI（实现：crates/airsnip-macos/src/）。
//
// 约定（docs/macos-port.md §3.2）：
// - 返回 int32_t 状态码：AIRSNIP_OK 成功；失败时 airsnip_last_error() 给出说明
//   （本线程最近一次失败，UTF-8，指针在本线程下次失败前有效，调用方不释放）。
// - Rust 内部 panic 不会跨越本接口，统一转为 AIRSNIP_ERR_PANIC（无返回值的函数则忽略该次操作）。
// - 图像为 RGBA8 预乘；传入的缓冲只在调用期间借用，Rust 会复制所需数据。
// - 字符串入参为 NUL 结尾 UTF-8。
// - 覆盖层（AsOverlay）的所有函数只在主线程调用。

#ifndef AIRSNIP_H
#define AIRSNIP_H

#include <stddef.h>
#include <stdint.h>

#define AIRSNIP_OK 0
#define AIRSNIP_ERR_FAILED (-1)
#define AIRSNIP_ERR_PANIC (-2)

/// 图像缓冲（借用）：data 至少 stride * height 字节，每行前 width * 4 字节有效。
typedef struct {
    const uint8_t *data;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
} AsImage;

/// 核心版本号（静态字符串）。
const char *airsnip_version(void);

/// 本线程最近一次失败的说明（无错误时为空串）。
const char *airsnip_last_error(void);

/// 按扩展名（png / jpg / jpeg / webp）编码并写入 path。
int32_t airsnip_save_image(const AsImage *img, const char *path);

// ---------------------------------------------------------------- 覆盖层

/// 光标形状（airsnip_core::overlay::Shape 的判别值）。
enum {
    AIRSNIP_CURSOR_ARROW = 0,
    AIRSNIP_CURSOR_CROSS = 1,
    AIRSNIP_CURSOR_HAND = 2,
    AIRSNIP_CURSOR_MOVE = 3,
    AIRSNIP_CURSOR_NS = 4,
    AIRSNIP_CURSOR_WE = 5,
    AIRSNIP_CURSOR_NWSE = 6,
    AIRSNIP_CURSOR_NESW = 7,
    AIRSNIP_CURSOR_RADIUS = 8,
};

/// 修饰键位（airsnip_overlay_set_mods / _wheel / _key 的 mods 参数）。
#define AIRSNIP_MOD_SHIFT 1u
#define AIRSNIP_MOD_SPACE 2u
#define AIRSNIP_MOD_CMD 4u

/// 宿主回调表。带 ctx 的回调在主线程、在 airsnip_overlay_* 调用内同步触发；
/// clipboard_png / notify 不带 ctx，可能在后台线程调用，实现方需自行回主线程。
/// 任何回调都可为 NULL（视为不支持）。
typedef struct {
    void *ctx;
    /// 上屏：覆盖层像素坐标的矩形需要重绘（从 airsnip_overlay_display 取像素）。
    void (*present)(void *ctx, int32_t x, int32_t y, int32_t w, int32_t h);
    void (*set_cursor)(void *ctx, int32_t shape);
    /// 结束覆盖层：只能安排异步释放，不得在回调内调用 airsnip_overlay_free。
    void (*close)(void *ctx);
    void (*alert)(void *ctx, const char *msg);
    /// 贴图（PIN-1）：img 只在回调期间有效；返回非 0 表示成功。
    int32_t (*pin)(void *ctx, const AsImage *img, int32_t x, int32_t y);
    /// 另存为对话框：选定路径写入 out（NUL 结尾）并返回非 0；取消返回 0。dir 可为 NULL。
    int32_t (*pick_save_file)(void *ctx, const char *dir, const char *name, const char *ext, char *out, size_t out_len);
    /// 文本输入期间开启输入法（EDT-4）。
    void (*set_ime)(void *ctx, int32_t enabled);
    /// 复制完成：PNG 数据只在回调期间有效（后台线程）。
    void (*clipboard_png)(const uint8_t *png, size_t len, uint32_t width, uint32_t height);
    /// 后台失败提示（复制 / 保存）。
    void (*notify)(const char *msg);
} AsHost;

typedef struct AsOverlay AsOverlay;

/// 创建覆盖层：screen 为该显示器像素的 RGBA 预乘图；backing_scale 为 backingScaleFactor；
/// dark 为系统当前是否深色外观。失败返回 NULL。host->ctx 须在覆盖层释放前保持有效。
AsOverlay *airsnip_overlay_new(const AsImage *screen, float backing_scale, int32_t dark, const AsHost *host);
/// 释放覆盖层（同时记住本次的工具、保存格式与目录）。
void airsnip_overlay_free(AsOverlay *ov);
/// 借用上屏缓冲（RGBA 预乘），在下一次调用该覆盖层的函数前有效。
AsImage airsnip_overlay_display(AsOverlay *ov);

void airsnip_overlay_set_mods(AsOverlay *ov, uint32_t mods, int32_t x, int32_t y);
void airsnip_overlay_mouse_down(AsOverlay *ov, int32_t x, int32_t y);
void airsnip_overlay_mouse_move(AsOverlay *ov, int32_t x, int32_t y);
void airsnip_overlay_mouse_up(AsOverlay *ov);
void airsnip_overlay_double_click(AsOverlay *ov, int32_t x, int32_t y);
void airsnip_overlay_cancel(AsOverlay *ov);
/// 贴选区并结束截图（截图期间按贴图热键，同 Windows 截图时按 F3）。
void airsnip_overlay_pin(AsOverlay *ov);
/// 截图时屏幕上各窗口的矩形（rects 为 count 组 x, y, w, h，覆盖层像素，前到后），开启窗口识别（CAP-6）。
void airsnip_overlay_set_windows(AsOverlay *ov, const int32_t *rects, size_t count);
/// 鼠标离开本屏：撤掉窗口识别的悬停高亮。
void airsnip_overlay_leave(AsOverlay *ov);
/// 滚轮：steps 为带符号格数（触控板连续滚动由调用方归并成格）。
void airsnip_overlay_wheel(AsOverlay *ov, int32_t x, int32_t y, int32_t steps, uint32_t mods);
/// 按键（kVK_* 键码）：返回非 0 表示已处理。
int32_t airsnip_overlay_key(AsOverlay *ov, uint32_t code, uint32_t mods);
/// 是否正在文本输入（此时空格是字符，不是「按住拖动选区」）。
int32_t airsnip_overlay_editing_text(AsOverlay *ov);
/// 输入法文字：commit 非 0 为上屏，0 为组合串（空串 = 组合结束）。
void airsnip_overlay_text(AsOverlay *ov, const char *text, int32_t commit);
/// 输入中的光标竖线（像素，左上原点）写入 out[3]：x, y, 高；不在输入中返回 0。
int32_t airsnip_overlay_caret(AsOverlay *ov, int32_t *out);
int32_t airsnip_overlay_has_selection(AsOverlay *ov);
/// 关闭后的结果：0 完成、1 取消、2 长截图（viewport 写入 x, y, w, h，可为 NULL）。
int32_t airsnip_overlay_result(AsOverlay *ov, int32_t *viewport);

// ---------------------------------------------------------------- 贴图

/// 贴图配置（PIN-1~6）：键码 kVK_*，修饰为 Carbon 修饰位
/// （controlKey 0x1000 / optionKey 0x0800 / shiftKey 0x0200 / cmdKey 0x0100）。
typedef struct {
    uint32_t key, mods;               ///< 剪贴板贴图全局热键
    uint32_t border_key, border_mods; ///< 贴图窗口内的边框开关键
    int32_t center;                   ///< 非 0：截图贴图放屏幕中央；0：选区原位置
    int32_t border;                   ///< 非 0：显示 1px 蓝色边框
} AsPinConfig;

void airsnip_config_pin(AsPinConfig *out);
/// 记住边框开关（之后的新贴图沿用）。
void airsnip_config_set_pin_border(int32_t on);
/// 默认截图目录下的新 PNG 路径（时间戳文件名，目录不存在时创建）写入 out；失败返回非 0。
int32_t airsnip_default_save_path(char *out, size_t len);

// ---------------------------------------------------------------- 长截图

/// 长截图（LNG-1/3/6/7）：Swift 抓视口帧推入，核心拼接。只在主线程调用。
typedef struct AsLongshot AsLongshot;
/// 视口 w × h 像素；失败返回 NULL。由 airsnip_longshot_free 释放。
AsLongshot *airsnip_longshot_new(uint32_t w, uint32_t h);
void airsnip_longshot_free(AsLongshot *ls);
/// 推入一帧并拼接：0 继续、1 已达高度上限（应结束）、负数失败。
int32_t airsnip_longshot_push(AsLongshot *ls, const AsImage *frame);
/// 状态条提示（空串 = 显示默认操作提示），下次调用前有效。
const char *airsnip_longshot_hint(AsLongshot *ls);
int32_t airsnip_longshot_can_undo(AsLongshot *ls);
void airsnip_longshot_undo(AsLongshot *ls);
/// 拼接结果缩略图（还没有内容时宽高为 0），下次调用前有效。
AsImage airsnip_longshot_thumbnail(AsLongshot *ls, uint32_t max_w, uint32_t max_h);
/// 完成：保存 PNG 到默认目录，并借出 PNG 字节写剪贴板（释放前有效）；保存失败仍借出 PNG 并返回非 0。
int32_t airsnip_longshot_finish(AsLongshot *ls, float scale, const uint8_t **out_png, size_t *out_len);

// ---------------------------------------------------------------- 设置

/// 设置窗口可改的配置项（SYS-3）。枚举为序号：theme 0 跟随系统 / 1 浅色 / 2 深色；
/// ui_scale 0 自动 / 1–5 = 100%–200%；name_style 0 日期 + 时间 / 1 时间戳 / 2 仅日期。
typedef struct {
    int32_t auto_start, theme, ui_scale, name_style, pin_center;
    uint32_t shot_key, shot_mods, pin_key, pin_mods, border_key, border_mods, longshot_key;
    uint32_t tool_rect, tool_arrow, tool_text, tool_blur, tool_highlight, tool_glow;
} AsSettings;

void airsnip_settings_load(AsSettings *out);
/// 立即写入配置文件；全局热键由调用方重新注册。
int32_t airsnip_settings_save(const AsSettings *s);

#endif
