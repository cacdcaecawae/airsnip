// SPDX-License-Identifier: GPL-3.0-only
// 贴图（PIN-1~6）：截图选区或剪贴板图片钉在屏幕上（浮动面板，不抢焦点，出现在所有桌面空间）。
// 拖动移动、滚轮或双指捏合缩放（0.1–8 倍）、⌘ + 滚轮调透明度、方向键微调（Shift 5pt）、双击 / Esc 关闭、
// 右键菜单（复制 / 保存 / 关闭 / 关闭全部）、边框开关键（默认 ⌃F3，切换全部贴图并记住）；最多 20 张。
// 缩放由 Core Animation 合成（图层内容随窗口拉伸），不自己重采样，窗口静止时没有任何重绘。

import AppKit
import CAirsnip

enum Pins {
    static let limit = 20
    private static var all: [PinPanel] = []

    static var config: AsPinConfig {
        var c = AsPinConfig()
        airsnip_config_pin(&c)
        return c
    }

    /// 贴图的窗口号（抓屏时保留贴图，其余自身窗口排除）。
    static var windowIDs: Set<CGWindowID> { Set(all.map { CGWindowID($0.windowNumber) }) }

    /// 新建贴图：`size` 为显示大小（points），`origin` 为左下角屏幕坐标；nil 时在鼠标所在屏居中并错开。
    @discardableResult
    static func show(_ image: CGImage, size: NSSize, at origin: NSPoint?) -> Bool {
        guard all.count < limit else {
            Alerts.show("贴图数量已达上限（\(limit) 张）。\n请关闭部分贴图后再试。")
            return false
        }
        let screen = NSScreen.screens.first { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) } ?? NSScreen.main
        let area = screen?.visibleFrame ?? NSRect(origin: .zero, size: size)
        let step = CGFloat(all.count * 20)
        var p = origin ?? NSPoint(x: area.midX - size.width / 2 + step, y: area.midY - size.height / 2 - step)
        if origin == nil {
            p.x = min(max(p.x, area.minX + 4), max(area.maxX - size.width - 4, area.minX + 4))
            p.y = min(max(p.y, area.minY + 4), max(area.maxY - size.height - 4, area.minY + 4))
        }
        let panel = PinPanel(image: image, size: size, border: config.border != 0)
        panel.setFrameOrigin(p)
        all.append(panel)
        panel.orderFrontRegardless()
        return true
    }

    /// 剪贴板贴图（PIN-2）：按图片自带的显示大小（Retina 截图带分辨率信息时与屏幕上一样大）。
    static func fromClipboard() {
        guard let img = NSImage(pasteboard: .general),
            let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil)
        else {
            Alerts.show("剪贴板中没有图片")
            return
        }
        show(cg, size: img.size, at: nil)
    }

    static func closed(_ panel: PinPanel) { all.removeAll { $0 === panel } }

    static func closeAll() { all.forEach { $0.close() } }

    /// 切换全部贴图的边框（以第一张为准取反）并记住。
    static func toggleBorder() {
        let on = !(all.first?.border ?? true)
        all.forEach { $0.border = on }
        airsnip_config_set_pin_border(on ? 1 : 0)
    }
}

/// 一张贴图：无边框浮动面板，内容是图层（随窗口拉伸）。
final class PinPanel: NSPanel {
    let image: CGImage
    private let base: NSSize
    private var zoom: CGFloat = 1
    private var wheel = WheelSteps()

    var border: Bool {
        didSet { applyBorder() }
    }

    init(image: CGImage, size: NSSize, border: Bool) {
        self.image = image
        self.base = size
        self.border = border
        super.init(
            contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered, defer: false)
        level = .floating
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        hasShadow = true
        isReleasedWhenClosed = false
        // 截图结束时应用会隐藏自己（把焦点还给原应用），贴图不能跟着隐藏
        hidesOnDeactivate = false
        canHide = false
        let view = PinView(frame: NSRect(origin: .zero, size: size))
        view.wantsLayer = true
        view.layer?.contents = image
        view.layer?.contentsGravity = .resize
        view.layer?.magnificationFilter = .linear
        view.layer?.minificationFilter = .trilinear
        view.layer?.borderColor = NSColor(srgbRed: 0x1E / 255, green: 0x90 / 255, blue: 1, alpha: 1).cgColor
        contentView = view
        applyBorder()
    }

    override var canBecomeKey: Bool { true }

    override func close() {
        super.close()
        // 推迟移出列表：此刻可能还在本窗口的事件处理里，列表是它最后的强引用
        DispatchQueue.main.async { Pins.closed(self) }
    }

    /// 1px（物理像素）蓝框，画在内容边缘。
    private func applyBorder() {
        contentView?.layer?.borderWidth = border ? 1 / backingScaleFactor : 0
    }

    /// 缩放到 `zoom` 倍（0.1–8），保持窗口中心不动。
    func setZoom(_ z: CGFloat) {
        zoom = min(max(z, 0.1), 8)
        let size = NSSize(width: (base.width * zoom).rounded(), height: (base.height * zoom).rounded())
        let f = frame
        setFrame(NSRect(x: f.midX - size.width / 2, y: f.midY - size.height / 2, width: size.width, height: size.height),
            display: true)
    }

    override func scrollWheel(with event: NSEvent) {
        let steps = wheel.take(event)
        guard steps != 0 else { return }
        if event.modifierFlags.contains(.command) {
            alphaValue = min(max(alphaValue + CGFloat(steps) * 24 / 255, 12 / 255), 1)
        } else {
            setZoom(zoom * pow(1.1, CGFloat(steps)))
        }
    }

    override func magnify(with event: NSEvent) { setZoom(zoom * (1 + event.magnification)) }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount >= 2 {
            close()
        } else {
            performDrag(with: event)
        }
    }

    override func rightMouseDown(with event: NSEvent) {
        let menu = NSMenu()
        let items: [(String, Selector)] = [
            ("复制", #selector(copyImage)), ("保存", #selector(saveImage)),
            ("关闭", #selector(close)), ("关闭全部", #selector(closeAll)),
        ]
        for (i, (title, action)) in items.enumerated() {
            if i == 2 { menu.addItem(.separator()) }
            menu.addItem(withTitle: title, action: action, keyEquivalent: "").target = self
        }
        if let view = contentView { NSMenu.popUpContextMenu(menu, with: event, for: view) }
    }

    override func keyDown(with event: NSEvent) {
        let key = Pins.config
        let step: CGFloat = event.modifierFlags.contains(.shift) ? 5 : 1
        switch Int(event.keyCode) {
        case 53: close()  // Esc
        case 123: setFrameOrigin(NSPoint(x: frame.minX - step, y: frame.minY))
        case 124: setFrameOrigin(NSPoint(x: frame.minX + step, y: frame.minY))
        case 125: setFrameOrigin(NSPoint(x: frame.minX, y: frame.minY - step))
        case 126: setFrameOrigin(NSPoint(x: frame.minX, y: frame.minY + step))
        case Int(key.border_key) where carbonMods(event.modifierFlags) == key.border_mods: Pins.toggleBorder()
        default: super.keyDown(with: event)
        }
    }

    @objc private func copyImage() { Clipboard.write(image, size: base) }

    @objc private func saveImage() {
        var buf = [CChar](repeating: 0, count: 4096)
        guard airsnip_default_save_path(&buf, buf.count) == 0 else {
            Alerts.show("保存失败：\(String(cString: airsnip_last_error()))")
            return
        }
        let url = URL(fileURLWithPath: String(cString: buf))
        guard let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else { return }
        do { try png.write(to: url) } catch { Alerts.show("保存失败：\(error.localizedDescription)") }
    }

    @objc private func closeAll() { Pins.closeAll() }
}

/// 贴图内容视图：只承载图层；第一下点击就能拖动（面板不激活应用）。
private final class PinView: NSView {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

/// NSEvent 修饰键 → Carbon 修饰位（与配置里的热键格式一致）。
func carbonMods(_ flags: NSEvent.ModifierFlags) -> UInt32 {
    var m: UInt32 = 0
    if flags.contains(.command) { m |= 0x100 }
    if flags.contains(.shift) { m |= 0x200 }
    if flags.contains(.option) { m |= 0x800 }
    if flags.contains(.control) { m |= 0x1000 }
    return m
}

/// 滚轮归并为格：普通鼠标每个事件一格；触控板连续滚动按 12pt 一格。方向统一为「设备向上 = 正」。
struct WheelSteps {
    private var accum: CGFloat = 0

    mutating func take(_ event: NSEvent) -> Int {
        var dy = event.scrollingDeltaY
        if event.isDirectionInvertedFromDevice { dy = -dy }
        guard event.hasPreciseScrollingDeltas else { return dy > 0 ? 1 : (dy < 0 ? -1 : 0) }
        accum += dy
        let steps = Int((accum / 12).rounded(.towardZero))
        accum -= CGFloat(steps) * 12
        return steps
    }
}

extension CGImage {
    /// 复制 RGBA8 预乘 sRGB 像素为 CGImage（源缓冲可在返回后释放）。
    static func copyRGBA(_ img: AsImage) -> CGImage? {
        guard let data = img.data, img.width > 0, img.height > 0 else { return nil }
        let bytes = Data(bytes: data, count: Int(img.stride) * Int(img.height))
        guard let provider = CGDataProvider(data: bytes as CFData) else { return nil }
        return CGImage(
            width: Int(img.width), height: Int(img.height), bitsPerComponent: 8, bitsPerPixel: 32,
            bytesPerRow: Int(img.stride), space: sRGB,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }
}
