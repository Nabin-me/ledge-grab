import CoreGraphics
import Foundation
import XCTest
@testable import LedgeCaptureCore

final class CaptureCoreTests: XCTestCase {
    func testRetinaCropRespectsTopLeftOriginAndRoundsOut() {
        let canvas = CGRect(x: 0, y: 0, width: 100, height: 80)
        let pixelSize = CGSize(width: 200, height: 160)
        XCTAssertEqual(
            CaptureGeometry.pixelRect(CGRect(x: 10.25, y: 7.25, width: 20, height: 12.5),
                                      in: canvas, imageSize: pixelSize),
            CGRect(x: 20, y: 14, width: 41, height: 26))
    }

    func testCropClipsToCanvasWithNonZeroOrigin() {
        let canvas = CGRect(x: 100, y: -50, width: 100, height: 80)
        let pixelSize = CGSize(width: 200, height: 160)
        XCTAssertEqual(
            CaptureGeometry.pixelRect(CGRect(x: 170, y: 10, width: 50, height: 55),
                                      in: canvas, imageSize: pixelSize),
            CGRect(x: 140, y: 120, width: 60, height: 40))
    }

    func testSmallEmptyAndOffCanvasCropDoesNotProduceAnImage() {
        let canvas = CGRect(x: 0, y: 0, width: 100, height: 100)
        for selection in [CGRect.zero, CGRect(x: 3, y: 4, width: 1, height: 8),
                          CGRect(x: 200, y: 10, width: 20, height: 30)] {
            XCTAssertNil(CaptureGeometry.pixelRect(selection, in: canvas, imageSize: canvas.size))
        }
    }

    func testRecentCapturesPersistSortIgnoreUnrelatedFilesAndLimit() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = CaptureStore(directory: directory)
        XCTAssertTrue(store.recent().isEmpty)
        let older = try store.savePNG(Data([1]), at: Date(timeIntervalSince1970: 1_700_000_000))
        let newer = try store.savePNG(Data([2]), at: Date(timeIntervalSince1970: 1_700_090_000))
        try Data([3]).write(to: directory.appendingPathComponent("Other.png"))
        try FileManager.default.createSymbolicLink(at: directory.appendingPathComponent(newer.lastPathComponent),
                                                   withDestinationURL: newer)
        XCTAssertEqual(try Data(contentsOf: newer), Data([2]))
        XCTAssertEqual(CaptureStore(directory: directory).recent(), [newer, older])
        XCTAssertEqual(store.recent(limit: 1), [newer])
        XCTAssertTrue(store.recent(limit: 0).isEmpty)
    }

    func testRapidCapturesNeverOverwrite() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        let store = CaptureStore(directory: folder)
        let instant = Date()
        let one = try store.savePNG(Data([1]), at: instant)
        let two = try store.savePNG(Data([2]), at: instant)
        XCTAssertNotEqual(one, two)
        XCTAssertEqual(Set(store.recent()), Set([one, two]))
    }
}
