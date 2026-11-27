import AppKit
import CoreGraphics
import ImageIO
import LedgeCaptureCore
import ScreenCaptureKit
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class LedgeApp: NSObject, NSApplicationDelegate {
    private let popover = NSPopover()
    private var statusItem: NSStatusItem?
    private var selection: RegionWindow?
    private var busy = false
    private var previousApp: NSRunningApplication?
    private var note = "Screenshots are saved on this Mac."
    private var shortcut: RegionShortcut?
    private let store = CaptureStore(directory: FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Pictures", isDirectory: true)
        .appendingPathComponent("Ledge Grab", isDirectory: true))

    func applicationDidFinishLaunching(_ notification: Notification) {
        popover.behavior = .transient
        popover.animates = true
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        let glyph = Bundle.main.url(forResource: "MenuMark", withExtension: "png")
            .flatMap { NSImage(contentsOf: $0) } ?? NSImage(systemSymbolName: "viewfinder", accessibilityDescription: nil)
        glyph?.isTemplate = true
        glyph?.size = NSSize(width: 19, height: 19)
        item.button?.image = glyph
        item.button?.toolTip = "Ledge Grab"
        item.button?.setAccessibilityLabel("Ledge Grab screenshot shelf")
        item.button?.target = self
        item.button?.action = #selector(toggleShelf)
        statusItem = item
        shortcut = RegionShortcut { [weak self] in self?.beginCapture(region: true) }
    }

    @objc private func toggleShelf() {
        guard !busy else { return }
        if popover.isShown { popover.performClose(nil) }
        else { showShelf() }
    }

    private func showShelf() {
        guard let button = statusItem?.button else { return }
        refreshShelf()
        if !popover.isShown { popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY) }
    }

    private func refreshShelf() {
        popover.contentViewController = NSHostingController(rootView: Shelf(
            items: store.recent(limit: 6), note: note, shortcutAvailable: shortcut != nil,
            captureRegion: { [weak self] in self?.beginCapture(region: true) },
            captureDisplay: { [weak self] in self?.beginCapture(region: false) },
            copy: { [weak self] in self?.copyFile($0) ?? false },
            openFolder: { [weak self] in self?.openFolder() },
            quit: { NSApplication.shared.terminate(nil) }))
    }

    private func beginCapture(region: Bool) {
        guard !busy else { return }
        let pointer = NSEvent.mouseLocation
        guard let screen = NSScreen.screens.first(where: { $0.frame.contains(pointer) }) ?? NSScreen.main,
              let displayID = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
        else { return fail("No active display was found.") }
        guard CGPreflightScreenCaptureAccess() else {
            popover.performClose(nil)
            _ = CGRequestScreenCaptureAccess()
            let alert = NSAlert()
            alert.messageText = "Allow Ledge Grab to capture your screen"
            alert.informativeText = "Enable Ledge Grab in System Settings → Privacy & Security → Screen & System Audio Recording, then quit and reopen the app."
            alert.addButton(withTitle: "Open System Settings")
            alert.addButton(withTitle: "Not now")
            NSApp.activate(ignoringOtherApps: true)
            if alert.runModal() == .alertFirstButton,
               let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture") {
                NSWorkspace.shared.open(url)
            }
            return
        }

        busy = true
        let frontmost = NSWorkspace.shared.frontmostApplication
        previousApp = frontmost?.processIdentifier == ProcessInfo.processInfo.processIdentifier ? nil : frontmost
        popover.performClose(nil)
        statusItem?.button?.toolTip = "Ledge Grab · Capturing"
        let frame = screen.frame
        let pixelWidth = Int((frame.width * screen.backingScaleFactor).rounded())
        let pixelHeight = Int((frame.height * screen.backingScaleFactor).rounded())
        Task { @MainActor in
            do {
                let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
                guard let display = content.displays.first(where: { $0.displayID == displayID.uint32Value })
                else { return fail("That display is no longer available.") }
                let ownApp = content.applications.filter { $0.processID == ProcessInfo.processInfo.processIdentifier }
                let filter = SCContentFilter(display: display, excludingApplications: ownApp, exceptingWindows: [])
                let config = SCStreamConfiguration()
                config.width = pixelWidth
                config.height = pixelHeight
                config.showsCursor = false
                let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
                if region { chooseRegion(in: frame, from: image) }
                else { await save(image) }
            } catch { fail("Capture failed. Check screen permission, then try again.") }
        }
    }

    private func chooseRegion(in frame: CGRect, from image: CGImage) {
        let panel = RegionWindow(contentRect: frame, styleMask: .borderless, backing: .buffered, defer: false)
        panel.setFrame(frame, display: false)
        panel.isReleasedWhenClosed = false
        panel.level = .screenSaver
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.hasShadow = false
        panel.hidesOnDeactivate = false
        let view = RegionView(frame: CGRect(origin: .zero, size: frame.size), frozen: image) { [weak self] cropped in
            guard let self else { return }
            self.selection?.orderOut(nil)
            self.selection = nil
            if let cropped { Task { @MainActor in await self.save(cropped) } }
            else { self.finishCapture() }
        }
        panel.contentView = view
        panel.initialFirstResponder = view
        selection = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(view)
    }

    private func save(_ image: CGImage) async {
        let destination = store
        let result = await Task.detached(priority: .userInitiated) {
            () -> (URL, Data)? in
            let output = NSMutableData()
            guard let encoder = CGImageDestinationCreateWithData(output, UTType.png.identifier as CFString, 1, nil)
            else { return nil }
            CGImageDestinationAddImage(encoder, image, nil)
            guard CGImageDestinationFinalize(encoder) else { return nil }
            let png = output as Data
            guard let url = try? destination.savePNG(png) else { return nil }
            return (url, png)
        }.value
        guard let (_, data) = result else { return fail("Could not save in Pictures → Ledge Grab.") }
        note = writeClipboard(data) ? "Saved and copied. Paste with ⌘V." : "Saved. Clipboard copy didn’t work."
        finishCapture()
        statusItem?.button?.toolTip = "Ledge Grab · " + note
    }

    private func writeClipboard(_ data: Data) -> Bool {
        let clipboard = NSPasteboard.general
        clipboard.clearContents()
        return clipboard.setData(data, forType: .png)
    }

    private func copyFile(_ file: URL) -> Bool {
        guard let data = try? Data(contentsOf: file, options: .mappedIfSafe) else { return false }
        return writeClipboard(data)
    }

    private func openFolder() {
        do {
            try FileManager.default.createDirectory(at: store.directory, withIntermediateDirectories: true)
            NSWorkspace.shared.open(store.directory)
        } catch { fail("The capture folder could not be opened.") }
    }

    private func finishCapture() {
        busy = false
        if NSApp.isActive { previousApp?.activate() }
        previousApp = nil
        statusItem?.button?.toolTip = "Ledge Grab"
    }

    private func fail(_ text: String) {
        finishCapture()
        note = text
        showShelf()
    }
}

private struct Shelf: View {
    let items: [URL]
    let note: String
    let shortcutAvailable: Bool
    let captureRegion: () -> Void
    let captureDisplay: () -> Void
    let copy: (URL) -> Bool
    let openFolder: () -> Void
    let quit: () -> Void
    @State private var feedback: String?
    private let columns = Array(repeating: GridItem(.flexible(), spacing: 8), count: 3)

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("Ledge Grab", systemImage: "viewfinder")
                    .font(.system(size: 17, weight: .semibold))
                Spacer()
                Button(action: openFolder) { Image(systemName: "folder") }
                    .help("Open capture folder").accessibilityLabel("Open capture folder")
                Button(action: quit) { Image(systemName: "power") }
                    .help("Quit Ledge Grab").accessibilityLabel("Quit Ledge Grab")
            }
            HStack(spacing: 8) {
                Button(action: captureRegion) {
                    Label(shortcutAvailable ? "Region · ⌥⇧4" : "Region", systemImage: "crop")
                }.buttonStyle(.borderedProminent)
                Button(action: captureDisplay) { Label("Display", systemImage: "display") }
                    .buttonStyle(.bordered)
            }
            if items.isEmpty {
                Text("Your recent captures will live here.")
                    .font(.callout).foregroundStyle(.secondary).padding(.vertical, 8)
            } else {
                Text("RECENT CAPTURES")
                    .font(.system(size: 10, weight: .semibold, design: .monospaced))
                    .foregroundStyle(.secondary)
                LazyVGrid(columns: columns, spacing: 8) {
                    ForEach(items, id: \.self) { url in
                        Button {
                            feedback = copy(url) ? "Copied. Paste with ⌘V." : "Could not read that capture."
                        } label: {
                            Thumbnail(url: url)
                        }
                        .buttonStyle(.plain)
                        .help("Click to copy or drag the PNG to another app")
                        .accessibilityLabel("Copy capture \((items.firstIndex(of: url) ?? 0) + 1)")
                        .onDrag { NSItemProvider(contentsOf: url) ?? NSItemProvider(object: url as NSURL) }
                    }
                }
            }
            Text(feedback ?? note)
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(14)
        .frame(width: 330)
    }
}

private struct Thumbnail: View {
    let url: URL

    private let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceThumbnailMaxPixelSize: 260
    ]

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 7).fill(.quaternary)
            if let source = CGImageSourceCreateWithURL(url as CFURL, nil),
               let image = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions as CFDictionary) {
                Image(nsImage: NSImage(cgImage: image, size: .zero))
                    .resizable().scaledToFit()
            } else {
                Image(systemName: "photo").foregroundStyle(.secondary)
            }
        }
        .frame(height: 62)
        .clipShape(RoundedRectangle(cornerRadius: 7))
    }
}
