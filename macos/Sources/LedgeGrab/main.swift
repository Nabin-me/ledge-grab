import AppKit

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let controller = LedgeApp()
app.delegate = controller
app.run()
