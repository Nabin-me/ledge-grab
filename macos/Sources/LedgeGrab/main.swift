import AppKit

MainActor.assumeIsolated {
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    let controller = LedgeApp()
    app.delegate = controller
    withExtendedLifetime(controller) { app.run() }
}
