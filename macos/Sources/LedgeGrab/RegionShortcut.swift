import Carbon
import Foundation

/// A registered shortcut, not a global input monitor: the app receives only this action.
final class RegionShortcut {
    private var handler: EventHandlerRef?
    private var key: EventHotKeyRef?
    private let action: @MainActor () -> Void

    init?(action: @escaping @MainActor () -> Void) {
        self.action = action
        var type = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let target = GetApplicationEventTarget()
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard InstallEventHandler(target, { _, _, context in
            guard let context else { return OSStatus(eventNotHandledErr) }
            let action = Unmanaged<RegionShortcut>.fromOpaque(context).takeUnretainedValue().action
            DispatchQueue.main.async { action() }
            return noErr
        }, 1, &type, context, &handler) == noErr else { return nil }
        let identity = EventHotKeyID(signature: 0x4C444745, id: 1) // LDGE
        guard RegisterEventHotKey(UInt32(kVK_ANSI_4), UInt32(optionKey | shiftKey), identity, target, 0, &key) == noErr
        else {
            if let handler { RemoveEventHandler(handler) }
            handler = nil
            return nil
        }
    }

    deinit {
        if let key { UnregisterEventHotKey(key) }
        if let handler { RemoveEventHandler(handler) }
    }
}
