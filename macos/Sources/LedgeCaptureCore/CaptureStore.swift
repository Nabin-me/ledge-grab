import Foundation

public struct CaptureStore {
    public let directory: URL

    public init(directory: URL) {
        self.directory = directory
    }

    public func savePNG(_ png: Data, at date: Date = .now) throws -> URL {
        let calendar = Calendar.current
        let day = calendar.dateComponents([.year, .month, .day], from: date)
        let name = String(format: "%04d-%02d-%02d", day.year ?? 0, day.month ?? 0, day.day ?? 0)
        let folder = directory.appendingPathComponent(name, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let stamp = Int64((date.timeIntervalSince1970 * 1000).rounded(.down))
        let file = folder.appendingPathComponent("Ledge-\(stamp)-\(UUID().uuidString).png")
        try png.write(to: file, options: [.atomic])
        return file
    }

    public func recent(limit: Int = 8) -> [URL] {
        guard limit > 0,
              let enumerator = FileManager.default.enumerator(at: directory,
                  includingPropertiesForKeys: [.isRegularFileKey, .isSymbolicLinkKey], options: [.skipsHiddenFiles])
        else { return [] }
        return enumerator.compactMap { element -> (Int64, URL)? in
            guard let file = element as? URL, file.pathExtension == "png",
                  let values = try? file.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey]),
                  values.isRegularFile == true, values.isSymbolicLink != true else { return nil }
            let name = file.deletingPathExtension().lastPathComponent
            guard name.hasPrefix("Ledge-"), let split = name.dropFirst(6).firstIndex(of: "-"),
                  let stamp = Int64(name[name.index(name.startIndex, offsetBy: 6)..<split]),
                  UUID(uuidString: String(name[name.index(after: split)...])) != nil else { return nil }
            return (stamp, file)
        }.sorted {
            $0.0 == $1.0 ? $0.1.lastPathComponent > $1.1.lastPathComponent : $0.0 > $1.0
        }.prefix(limit).map { $0.1 }
    }
}
