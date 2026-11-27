import AppKit
import LedgeCaptureCore

final class RegionWindow: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

final class RegionView: NSView {
    private let frozen: CGImage
    private let preview: NSImage
    private let finish: (CGImage?) -> Void
    private var start: CGPoint?
    private var region = CGRect.zero
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    init(frame: CGRect, frozen: CGImage, finish: @escaping (CGImage?) -> Void) {
        self.frozen = frozen
        self.preview = NSImage(cgImage: frozen, size: frame.size)
        self.finish = finish
        super.init(frame: frame)
        setAccessibilityLabel("Drag to capture a region. Escape cancels.")
    }

    required init?(coder: NSCoder) { fatalError("Programmatic view") }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .crosshair)
    }

    override func draw(_ dirtyRect: NSRect) {
        preview.draw(in: bounds, from: .zero, operation: .copy, fraction: 1, respectFlipped: true, hints: nil)
        let shade = NSBezierPath(rect: bounds)
        shade.windingRule = .evenOdd
        if !region.isEmpty { shade.appendRect(region) }
        NSColor.black.withAlphaComponent(0.42).setFill()
        shade.fill()
        if !region.isEmpty {
            NSColor.white.setStroke()
            let edge = NSBezierPath(rect: region)
            edge.lineWidth = 1.5
            edge.stroke()
        } else if start == nil {
            let label: NSString = "Drag to capture   ·   Esc to cancel"
            let attributes: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 14, weight: .semibold),
                .foregroundColor: NSColor.white
            ]
            let size = label.size(withAttributes: attributes)
            let box = CGRect(x: bounds.midX - size.width / 2 - 14, y: bounds.midY - 18,
                             width: size.width + 28, height: 36)
            NSColor.black.withAlphaComponent(0.8).setFill()
            NSBezierPath(roundedRect: box, xRadius: 12, yRadius: 12).fill()
            label.draw(at: CGPoint(x: box.midX - size.width / 2, y: box.midY - size.height / 2),
                       withAttributes: attributes)
        }
    }

    override func mouseDown(with event: NSEvent) {
        start = convert(event.locationInWindow, from: nil)
        region = .zero
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) { updateRegion(event) }

    override func mouseUp(with event: NSEvent) {
        updateRegion(event)
        start = nil
        let size = CGSize(width: CGFloat(frozen.width), height: CGFloat(frozen.height))
        guard let rect = CaptureGeometry.pixelRect(region, in: bounds, imageSize: size),
              let crop = frozen.cropping(to: rect) else {
            region = .zero
            needsDisplay = true
            return
        }
        finish(crop)
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { finish(nil) }
        else { super.keyDown(with: event) }
    }

    override func rightMouseDown(with event: NSEvent) { finish(nil) }

    private func updateRegion(_ event: NSEvent) {
        guard let start else { return }
        let end = convert(event.locationInWindow, from: nil)
        let rect = CGRect(x: min(start.x, end.x), y: min(start.y, end.y),
                          width: abs(end.x - start.x), height: abs(end.y - start.y))
        let clipped = rect.intersection(bounds)
        region = clipped.isNull ? .zero : clipped
        needsDisplay = true
    }
}
