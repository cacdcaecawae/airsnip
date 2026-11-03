// SPDX-License-Identifier: GPL-3.0-only
// 一次截图会话（CAP-2/CAP-5）：每块显示器一个覆盖窗口 + 一个 Rust 覆盖层，坐标为该显示器的像素。
// 选区只在一块屏幕内：某屏已有选区时，其他屏不接受按下（与系统 ⌘⇧4 一致）。任一覆盖层关闭即结束整个会话。

import AppKit
import CAirsnip

/// 覆盖窗口：无边框、高于菜单栏与 Dock（屏蔽窗口层级）、出现在所有桌面空间（含全屏应用）。
final class OverlayWindow: NSWindow {
    init(frame: NSRect) {
        super.init(contentRect: frame, styleMask: .borderless, backing: .buffered, defer: false)
        level = NSWindow.Level(rawValue: Int(CGShieldingWindowLevel()))
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        isOpaque = true
        backgroundColor = .black
        hasShadow = false
        isReleasedWhenClosed = false
        setFrame(frame, display: false)
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

final class OverlayController {
    private var sessions: [(window: OverlayWindow, view: OverlayView, display: DisplayInfo)] = []
    /// 结束回调：参数为长截图请求（选区转为滚动视口时），否则为 nil。
    private let onEnd: (LongshotRequest?) -> Void
    private var ending = false

    /// 为每张抓屏创建覆盖层；全部失败时返回 nil（详情见 `airsnip_last_error`）。
    /// `windows`：截图时屏幕上的窗口（CG 全局坐标，前到后），换算成各屏像素后用于窗口识别（CAP-6）。
    init?(shots: [DisplayShot], windows: [CGRect], onEnd: @escaping (LongshotRequest?) -> Void) {
        self.onEnd = onEnd
        let dark = NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        for shot in shots {
            let view = OverlayView(
                frame: NSRect(origin: .zero, size: shot.frame.size), scale: shot.scale, pixelHeight: shot.height)
            var host = HostBridge.make(for: view)
            let ov = shot.rgba.withUnsafeBufferPointer { buf -> OpaquePointer? in
                var img = AsImage(
                    data: buf.baseAddress, width: UInt32(shot.width), height: UInt32(shot.height),
                    stride: UInt32(shot.width * 4))
                return airsnip_overlay_new(&img, Float(shot.scale), dark ? 1 : 0, &host)
            }
            guard let ov else { continue }
            view.overlay = ov
            let rects = OverlayController.pixelRects(windows, display: shot.displayID, scale: shot.scale)
            airsnip_overlay_set_windows(ov, rects, rects.count / 4)
            let window = OverlayWindow(frame: shot.frame)
            window.contentView = view
            sessions.append((window, view, DisplayInfo(displayID: shot.displayID, frame: shot.frame, scale: shot.scale)))
        }
        guard !sessions.isEmpty else { return nil }
        sessions.forEach { $0.view.controller = self }
    }

    /// 显示全部覆盖窗口，鼠标所在屏获得键盘焦点。
    func show() {
        let mouse = NSEvent.mouseLocation
        sessions.forEach { $0.window.orderFrontRegardless() }
        let key = sessions.first { NSMouseInRect(mouse, $0.window.frame, false) } ?? sessions[0]
        NSApp.activate(ignoringOtherApps: true)
        key.window.makeKeyAndOrderFront(nil)
        key.window.makeFirstResponder(key.view)
        key.view.hover(at: mouse)  // 不等鼠标移动，先高亮鼠标下的窗口
    }

    /// 窗口矩形（CG 全局坐标，左上原点 points）→ 该显示器像素坐标（x, y, w, h 展平，向外取整）。
    private static func pixelRects(_ windows: [CGRect], display: CGDirectDisplayID, scale: CGFloat) -> [Int32] {
        let origin = CGDisplayBounds(display).origin
        return windows.flatMap { r -> [Int32] in
            let x0 = ((r.minX - origin.x) * scale).rounded(.down)
            let y0 = ((r.minY - origin.y) * scale).rounded(.down)
            let x1 = ((r.maxX - origin.x) * scale).rounded(.up)
            let y1 = ((r.maxY - origin.y) * scale).rounded(.up)
            return [Int32(x0), Int32(y0), Int32(x1 - x0), Int32(y1 - y0)]
        }
    }

    /// 截图期间按贴图热键：贴有选区的那块屏的选区（同 Windows 截图时按 F3）；还没有选区或已在收尾时不处理。
    func pinSelection() {
        guard !ending, let s = sessions.first(where: { airsnip_overlay_has_selection($0.view.overlay) != 0 }) else { return }
        airsnip_overlay_pin(s.view.overlay)
    }

    /// 某屏已有选区时，其他屏不接受按下。
    func mayInteract(_ view: OverlayView) -> Bool {
        !sessions.contains { $0.view !== view && airsnip_overlay_has_selection($0.view.overlay) != 0 }
    }

    /// 核心请求结束：异步收尾（此刻还在核心的调用栈里，不能释放覆盖层）。
    func requestEnd(from view: OverlayView) {
        guard !ending else { return }
        ending = true
        DispatchQueue.main.async { [self] in end(from: view) }
    }

    private func end(from view: OverlayView) {
        var viewport = [Int32](repeating: 0, count: 4)
        let result = airsnip_overlay_result(view.overlay, &viewport)
        // 长截图：选区（该显示器像素）→ 屏幕矩形（points），覆盖层释放前算好
        var longshot: LongshotRequest?
        if result == 2, let s = sessions.first(where: { $0.view === view }) {
            let (x, y, w, h) = (Int(viewport[0]), Int(viewport[1]), Int(viewport[2]), Int(viewport[3]))
            let r = view.viewRect(x: x, y: y, w: w, h: h).offsetBy(dx: s.display.frame.minX, dy: s.display.frame.minY)
            longshot = LongshotRequest(display: s.display, viewport: r, width: w, height: h)
        }
        for s in sessions {
            s.window.orderOut(nil)
            let ov = s.view.overlay
            s.view.overlay = nil
            airsnip_overlay_free(ov)
        }
        sessions.removeAll()
        // 把焦点还给截图前的应用（菜单栏应用自己没有窗口可留）
        NSApp.hide(nil)
        onEnd(longshot)
    }
}
