// Lists the on-screen windows of one app: window id, title, bounds, one
// per line. The app is named by owner name, or by pid with `--pid N`, which
// is what the driver uses: two apps here are both called infiniterm (the
// Tauri one and the port), and the driver must never find the other one's
// window. Build: swiftc -O tools/winid.swift -o tools/winid
import CoreGraphics
import Foundation
let args = CommandLine.arguments
let byPid = args.count > 2 && args[1] == "--pid"
let name = byPid ? "" : args[1]
let pid = byPid ? Int(args[2]) ?? -1 : -1
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]]
for w in list {
    let owner = w[kCGWindowOwnerName as String] as? String ?? ""
    let ownerPid = w[kCGWindowOwnerPID as String] as? Int ?? -1
    let matches = byPid ? ownerPid == pid : owner == name
    if matches, let id = w[kCGWindowNumber as String] as? Int {
        let title = w[kCGWindowName as String] as? String ?? ""
        let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
        print("\(id)\t\(title)\t\(b)")
    }
}
