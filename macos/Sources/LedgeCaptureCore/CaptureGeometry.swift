import Foundation

public enum CaptureGeometry {
    /// The selection view has a top-left origin, as do CGImage pixel rectangles.
    public static func pixelRect(_ selection: CGRect, in canvas: CGRect, imageSize: CGSize) -> CGRect? {
        let selected = selection.standardized.intersection(canvas)
        guard !selected.isNull, selected.width >= 2, selected.height >= 2,
              canvas.width > 0, canvas.height > 0, imageSize.width > 0, imageSize.height > 0 else { return nil }
        let xScale = imageSize.width / canvas.width
        let yScale = imageSize.height / canvas.height
        let left = floor((selected.minX - canvas.minX) * xScale)
        let top = floor((selected.minY - canvas.minY) * yScale)
        let right = min(imageSize.width, ceil((selected.maxX - canvas.minX) * xScale))
        let bottom = min(imageSize.height, ceil((selected.maxY - canvas.minY) * yScale))
        return CGRect(x: left, y: top, width: right - left, height: bottom - top)
    }
}
