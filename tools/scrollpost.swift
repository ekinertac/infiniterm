// tools/scrollpost.swift: posts a synthetic two-finger trackpad flick to ONE
// process, for measuring the canvas pan (#451). Run: `swift tools/scrollpost.swift
// <pid> <x> <y> [ms=800] [dy=-6] [hz=120]`.
//
// Why by pid: `CGEvent.postToPid` delivers the event to that process only, so
// a scratch instance can be driven while someone else uses the Mac; a global
// post would land in whatever window is under the pointer. The event is sent
// the way a trackpad sends it: a Began phase, Changed events at `hz` for `ms`,
// an Ended phase (the fingers lift), then the OS's momentum as events with no
// finger phase and a momentum phase of their own (what gpui 0.2 reports as
// `Moved` after `Ended`, see `pan_mode::PanSession`).
//
// x and y are screen coordinates inside the target window (top-left origin),
// where the pointer would be; the event carries that location.
// Related: tools/paintlog.sh (the `[paint]` log), tools/drive/lib.sh (a
// scratch instance), infiniterm-core/src/pan_mode.rs.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count >= 4, let pid = Int32(args[1]), let x = Double(args[2]), let y = Double(args[3]) else {
    FileHandle.standardError.write("usage: scrollpost <pid> <x> <y> [ms] [dy] [hz]\n".data(using: .utf8)!)
    exit(2)
}
let ms = args.count > 4 ? Double(args[4]) ?? 800 : 800
let dy = args.count > 5 ? Int64(args[5]) ?? -6 : -6
let hz = args.count > 6 ? Double(args[6]) ?? 120 : 120

// CGScrollPhase / CGMomentumScrollPhase values (CGEventTypes.h).
let phaseBegan: Int64 = 1, phaseChanged: Int64 = 2, phaseEnded: Int64 = 4
let momentumBegin: Int64 = 1, momentumContinue: Int64 = 2, momentumEnd: Int64 = 3

func post(_ deltaY: Int64, scrollPhase: Int64, momentum: Int64) {
    guard let e = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2,
                          wheel1: Int32(deltaY), wheel2: 0, wheel3: 0) else { return }
    e.location = CGPoint(x: x, y: y)
    e.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
    e.setIntegerValueField(.scrollWheelEventScrollPhase, value: scrollPhase)
    e.setIntegerValueField(.scrollWheelEventMomentumPhase, value: momentum)
    e.postToPid(pid)
}

let step = 1.0 / hz
let moves = Int(ms / 1000.0 * hz)
post(dy, scrollPhase: phaseBegan, momentum: 0)
for _ in 0..<moves {
    Thread.sleep(forTimeInterval: step)
    post(dy, scrollPhase: phaseChanged, momentum: 0)
}
Thread.sleep(forTimeInterval: step)
post(0, scrollPhase: phaseEnded, momentum: 0)
// The OS's momentum: decaying deltas, one per refresh, for about 600 ms.
var d = Double(dy) * 3
for i in 0..<Int(0.6 * hz) {
    Thread.sleep(forTimeInterval: step)
    d *= 0.94
    let m = Int64(d.rounded())
    if m == 0 { break }
    post(m, scrollPhase: 0, momentum: i == 0 ? momentumBegin : momentumContinue)
}
post(0, scrollPhase: 0, momentum: momentumEnd)
