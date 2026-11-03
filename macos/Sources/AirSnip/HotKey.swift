// SPDX-License-Identifier: GPL-3.0-only
// 全局热键（SYS-2）：Carbon RegisterEventHotKey。不需要辅助功能权限；
// 组合已被其他程序注册时返回错误码，由调用方提示冲突。

import Carbon.HIToolbox

final class HotKey {
    private var ref: EventHotKeyRef?
    private let id: UInt32

    /// 已注册热键的回调（按 id 分发）；Carbon 事件处理器是 C 函数指针，不能捕获上下文。
    private static var handlers: [UInt32: () -> Void] = [:]
    private static var installed = false
    /// 热键签名 'ASNP'。
    private static let signature: OSType = 0x4153_4E50

    /// 注册热键：`keyCode` 为 kVK_* 键码，`modifiers` 为 Carbon 修饰位。失败返回 OSStatus。
    init(keyCode: UInt32, modifiers: UInt32, id: UInt32, handler: @escaping () -> Void) throws {
        self.id = id
        HotKey.installHandler()
        let hkID = EventHotKeyID(signature: HotKey.signature, id: id)
        let status = RegisterEventHotKey(keyCode, modifiers, hkID, GetApplicationEventTarget(), 0, &ref)
        guard status == noErr else { throw HotKeyError(status: status) }
        HotKey.handlers[id] = handler
    }

    deinit {
        if let ref { UnregisterEventHotKey(ref) }
        HotKey.handlers[id] = nil
    }

    private static func installHandler() {
        guard !installed else { return }
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, _ -> OSStatus in
            var hk = EventHotKeyID()
            let status = GetEventParameter(
                event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                nil, MemoryLayout<EventHotKeyID>.size, nil, &hk)
            if status == noErr, hk.signature == HotKey.signature {
                HotKey.handlers[hk.id]?()
            }
            return noErr
        }, 1, &spec, nil, nil)
        installed = true
    }
}

struct HotKeyError: Error {
    let status: OSStatus
}

/// 热键的显示文本（如 ⌃⌥A）：修饰位 + 字母/数字键名；未知键码显示为 #键码。
func hotKeyLabel(keyCode: UInt32, modifiers: UInt32) -> String {
    var s = ""
    if modifiers & UInt32(controlKey) != 0 { s += "⌃" }
    if modifiers & UInt32(optionKey) != 0 { s += "⌥" }
    if modifiers & UInt32(shiftKey) != 0 { s += "⇧" }
    if modifiers & UInt32(cmdKey) != 0 { s += "⌘" }
    let names: [UInt32: String] = [
        0x00: "A", 0x0B: "B", 0x08: "C", 0x02: "D", 0x0E: "E", 0x03: "F", 0x05: "G", 0x04: "H",
        0x22: "I", 0x26: "J", 0x28: "K", 0x25: "L", 0x2E: "M", 0x2D: "N", 0x1F: "O", 0x23: "P",
        0x0C: "Q", 0x0F: "R", 0x01: "S", 0x11: "T", 0x20: "U", 0x09: "V", 0x0D: "W", 0x07: "X",
        0x10: "Y", 0x06: "Z", 0x12: "1", 0x13: "2", 0x14: "3", 0x15: "4", 0x17: "5", 0x16: "6",
        0x1A: "7", 0x1C: "8", 0x19: "9", 0x1D: "0", 0x7A: "F1", 0x78: "F2", 0x63: "F3", 0x76: "F4",
        0x60: "F5", 0x61: "F6", 0x62: "F7", 0x64: "F8", 0x65: "F9", 0x6D: "F10", 0x67: "F11", 0x6F: "F12",
        0x31: "空格", 0x24: "↩", 0x30: "⇥", 0x33: "⌫", 0x1B: "-", 0x18: "=", 0x21: "[", 0x1E: "]",
        0x29: ";", 0x27: "'", 0x2B: ",", 0x2F: ".", 0x2C: "/", 0x2A: "\\", 0x32: "`",
    ]
    return s + (names[keyCode] ?? "#\(keyCode)")
}
