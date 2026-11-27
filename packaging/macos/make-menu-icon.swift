import AppKit
import Foundation

let image = NSImage(size: NSSize(width: 40, height: 40), flipped: true) { rect in
    NSColor.black.setFill()
    let notch = NSBezierPath()
    notch.move(to: CGPoint(x: 4, y: 9))
    notch.line(to: CGPoint(x: 36, y: 9))
    notch.curve(to: CGPoint(x: 31, y: 16), controlPoint1: CGPoint(x: 33, y: 9), controlPoint2: CGPoint(x: 33, y: 12))
    notch.curve(to: CGPoint(x: 26, y: 20), controlPoint1: CGPoint(x: 30, y: 19), controlPoint2: CGPoint(x: 28, y: 20))
    notch.line(to: CGPoint(x: 14, y: 20))
    notch.curve(to: CGPoint(x: 9, y: 16), controlPoint1: CGPoint(x: 12, y: 20), controlPoint2: CGPoint(x: 10, y: 19))
    notch.curve(to: CGPoint(x: 4, y: 9), controlPoint1: CGPoint(x: 7, y: 12), controlPoint2: CGPoint(x: 7, y: 9))
    notch.close()
    notch.fill()
    NSBezierPath(roundedRect: CGRect(x: 13, y: 24, width: 14, height: 7), xRadius: 3.5, yRadius: 3.5).fill()
    return true
}
guard CommandLine.arguments.count == 2, let tiff = image.tiffRepresentation,
      let bitmap = NSBitmapImageRep(data: tiff),
      let png = bitmap.representation(using: .png, properties: [:]) else {
    fatalError("Could not produce menu mark")
}
try png.write(to: URL(fileURLWithPath: CommandLine.arguments[1]), options: .atomic)
