// SPDX-License-Identifier: GPL-3.0-only
// 覆盖层视图（CAP-2/3）：把 Rust 核心的上屏缓冲画到屏幕上，并把鼠标键盘事件换算成
// 显示器像素坐标转发给核心。视图不翻转（左下原点），核心像素坐标是左上原点：y_px = (高 − y_pt) × scale。

import AppKit
import CAirsnip

final class OverlayView: NSView {
    /// Rust 覆盖层（由 OverlayController 创建与释放；释放前先置空）。
    var overlay: OpaquePointer?
    weak var controller: OverlayController?
    /// 文本输入期间为 true（EDT-4，核心经宿主回调 set_ime 切换）。
    var imeEnabled = false {
        didSet {
            guard !imeEnabled, oldValue else { return }
            marked = ""
            // 此刻还在核心的调用栈里，输入法可能同步回调 insertText，推迟到下一轮
            DispatchQueue.main.async { [weak self] in self?.inputContext?.discardMarkedText() }
        }
    }
    /// 输入法组合串（NSTextInputClient，见 OverlayTextInput.swift）。
    var marked = ""
    var markedSelection = NSRange(location: 0, length: 0)
    let scale: CGFloat
    let pixelHeight: Int
    private var cursor = NSCursor.crosshair
    private var cursorShape: Int32 = -1
    private var spaceDown = false
    private var wheel = WheelSteps()
    private var lastPixel: (x: Int32, y: Int32) = (0, 0)

    init(frame: NSRect, scale: CGFloat, pixelHeight: Int) {
        self.scale = scale
        self.pixelHeight = pixelHeight
        super.init(frame: frame)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("不从 nib 创建") }

    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(
            NSTrackingArea(
                rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    // MARK: 坐标

    private func pixel(of event: NSEvent) -> (x: Int32, y: Int32) {
        let p = convert(event.locationInWindow, from: nil)
        lastPixel = (Int32((p.x * scale).rounded(.down)), Int32(((bounds.height - p.y) * scale).rounded(.down)))
        return lastPixel
    }

    /// 像素矩形（左上原点）→ 视图矩形（points，左下原点）。
    func viewRect(x: Int, y: Int, w: Int, h: Int) -> NSRect {
        NSRect(
            x: CGFloat(x) / scale, y: CGFloat(pixelHeight - y - h) / scale,
            width: CGFloat(w) / scale, height: CGFloat(h) / scale)
    }

    // MARK: 绘制

    /// 只画脏区：脏区换算成像素（向外取整），从上屏缓冲裁出子图画回原位（不复制整屏）。
    override func draw(_ dirtyRect: NSRect) {
        guard let ov = overlay, let ctx = NSGraphicsContext.current?.cgContext else { return }
        let img = airsnip_overlay_display(ov)
        guard let data = img.data, img.width > 0 else { return }
        let (w, h, stride) = (Int(img.width), Int(img.height), Int(img.stride))
        guard
            let provider = CGDataProvider(
                dataInfo: nil, data: UnsafeRawPointer(data), size: stride * h, releaseData: { _, _, _ in }),
            let full = CGImage(
                width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: stride, space: sRGB,
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
        else { return }
        let x0 = max(0, Int((dirtyRect.minX * scale).rounded(.down)))
        let x1 = min(w, Int((dirtyRect.maxX * scale).rounded(.up)))
        let y0 = max(0, Int(((bounds.height - dirtyRect.maxY) * scale).rounded(.down)))
        let y1 = min(h, Int(((bounds.height - dirtyRect.minY) * scale).rounded(.up)))
        guard x1 > x0, y1 > y0,
            let part = full.cropping(to: CGRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0))
        else { return }
        ctx.interpolationQuality = .none
        ctx.draw(part, in: viewRect(x: x0, y: y0, w: x1 - x0, h: y1 - y0))
    }

    /// 核心要求重绘某像素矩形（宿主回调 present）。
    func present(x: Int32, y: Int32, w: Int32, h: Int32) {
        setNeedsDisplay(viewRect(x: Int(x), y: Int(y), w: Int(w), h: Int(h)).insetBy(dx: -1, dy: -1))
    }

    // MARK: 光标

    func setShape(_ shape: Int32) {
        guard shape != cursorShape else { return }
        cursorShape = shape
        cursor = OverlayView.nsCursor(shape)
        cursor.set()
        window?.invalidateCursorRects(for: self)
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: cursor)
    }

    /// 核心光标形状 → 系统光标。macOS 14 没有公开的斜向缩放光标，退回十字（15 起用 frameResize）。
    private static func nsCursor(_ s: Int32) -> NSCursor {
        switch Int(s) {
        case Int(AIRSNIP_CURSOR_ARROW): return .arrow
        case Int(AIRSNIP_CURSOR_HAND), Int(AIRSNIP_CURSOR_MOVE): return .openHand
        case Int(AIRSNIP_CURSOR_NS): return .resizeUpDown
        case Int(AIRSNIP_CURSOR_WE): return .resizeLeftRight
        case Int(AIRSNIP_CURSOR_NWSE):
            if #available(macOS 15.0, *) { return .frameResize(position: .topLeft, directions: .all) }
            return .crosshair
        case Int(AIRSNIP_CURSOR_NESW):
            if #available(macOS 15.0, *) { return .frameResize(position: .topRight, directions: .all) }
            return .crosshair
        case Int(AIRSNIP_CURSOR_RADIUS): return .pointingHand
        default: return .crosshair
        }
    }

    // MARK: 鼠标

    func mods(_ flags: NSEvent.ModifierFlags) -> UInt32 {
        var m: UInt32 = 0
        if flags.contains(.shift) { m |= AIRSNIP_MOD_SHIFT }
        if spaceDown { m |= AIRSNIP_MOD_SPACE }
        if flags.contains(.command) { m |= AIRSNIP_MOD_CMD }
        return m
    }

    /// 更新修饰键并返回事件的像素坐标（每个鼠标事件前同步，保证 Shift 约束读到的是当前状态）。
    private func sync(_ event: NSEvent) -> (x: Int32, y: Int32)? {
        guard let ov = overlay else { return nil }
        let p = pixel(of: event)
        airsnip_overlay_set_mods(ov, mods(event.modifierFlags), p.x, p.y)
        return p
    }

    override func mouseDown(with event: NSEvent) {
        guard controller?.mayInteract(self) ?? false, let p = sync(event), let ov = overlay else { return }
        window?.makeKey()
        window?.makeFirstResponder(self)
        // 与 Windows 一致：双击的第二下以「双击」代替「按下」，随后仍有弹起
        if event.clickCount >= 2 {
            airsnip_overlay_double_click(ov, p.x, p.y)
        } else {
            airsnip_overlay_mouse_down(ov, p.x, p.y)
        }
    }

    override func mouseDragged(with event: NSEvent) { move(event) }
    override func mouseMoved(with event: NSEvent) { move(event) }

    /// 别的屏已有选区时本屏不接受按下，也就不做窗口识别的悬停高亮。
    private func move(_ event: NSEvent) {
        guard controller?.mayInteract(self) ?? false, let p = sync(event), let ov = overlay else { return }
        airsnip_overlay_mouse_move(ov, p.x, p.y)
    }

    /// 鼠标离开本屏：撤掉窗口识别的悬停高亮（多屏时只有鼠标所在屏高亮）。
    override func mouseExited(with event: NSEvent) {
        guard let ov = overlay else { return }
        airsnip_overlay_leave(ov)
    }

    /// 覆盖层刚出现、鼠标还没动：按屏幕坐标 `point` 先做一次悬停识别。
    func hover(at point: NSPoint) {
        guard let ov = overlay, let window else { return }
        let p = convert(window.convertPoint(fromScreen: point), from: nil)
        airsnip_overlay_mouse_move(
            ov, Int32((p.x * scale).rounded(.down)), Int32(((bounds.height - p.y) * scale).rounded(.down)))
    }

    override func mouseUp(with event: NSEvent) {
        guard controller?.mayInteract(self) ?? false, sync(event) != nil, let ov = overlay else { return }
        airsnip_overlay_mouse_up(ov)
    }

    /// 右键：取消截图（OUT-3）。
    override func rightMouseUp(with event: NSEvent) {
        guard let ov = overlay else { return }
        airsnip_overlay_cancel(ov)
    }

    /// 滚轮（按格，见 WheelSteps）：设备向上 = 增大。
    override func scrollWheel(with event: NSEvent) {
        guard let p = sync(event), let ov = overlay else { return }
        let steps = wheel.take(event)
        if steps != 0 {
            airsnip_overlay_wheel(ov, p.x, p.y, Int32(steps), mods(event.modifierFlags))
        }
    }

    // MARK: 键盘

    override func keyDown(with event: NSEvent) {
        guard let ov = overlay else { return }
        // 文字输入中：按键先交输入法，字符经 NSTextInputClient、编辑键经 doCommand 回到核心。
        // 其余时候不经输入法，工具键、长截图键不会被中文输入法截走。
        if airsnip_overlay_editing_text(ov) != 0 {
            interpretKeyEvents([event])
            return
        }
        // 空格：按住拖动整个选区（CAP-2）
        if event.keyCode == 49 {
            if !event.isARepeat {
                spaceDown = true
                airsnip_overlay_set_mods(ov, mods(event.modifierFlags), lastPixel.x, lastPixel.y)
            }
            return
        }
        _ = airsnip_overlay_key(ov, UInt32(event.keyCode), mods(event.modifierFlags))
    }

    /// 输入法不处理的按键（insertNewline:、deleteBackward:、moveLeft:、cancelOperation: 等）：
    /// 把原始按键交给核心的按键分发（键码规则见 airsnip-macos keys.rs）。
    override func doCommand(by selector: Selector) {
        guard let ov = overlay, let event = NSApp.currentEvent, event.type == .keyDown else { return }
        _ = airsnip_overlay_key(ov, UInt32(event.keyCode), mods(event.modifierFlags))
    }

    /// ⌘ 组合先走这里（菜单快捷键通道）：交给核心，处理了就吞掉。
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard let ov = overlay, event.modifierFlags.contains(.command) else { return false }
        return airsnip_overlay_key(ov, UInt32(event.keyCode), mods(event.modifierFlags)) != 0
    }

    override func keyUp(with event: NSEvent) {
        guard let ov = overlay, event.keyCode == 49, spaceDown else { return }
        spaceDown = false
        airsnip_overlay_set_mods(ov, mods(event.modifierFlags), lastPixel.x, lastPixel.y)
    }

    override func flagsChanged(with event: NSEvent) {
        guard let ov = overlay else { return }
        airsnip_overlay_set_mods(ov, mods(event.modifierFlags), lastPixel.x, lastPixel.y)
    }

    /// 核心请求结束（宿主回调 close）：只能安排异步收尾，不能在回调里释放覆盖层。
    func requestClose() {
        controller?.requestEnd(from: self)
    }
}
