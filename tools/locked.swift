// Prints 1 when the screen is locked, else 0. Built by tools/drive/lib.sh users with
//   swiftc -O -o tools/locked tools/locked.swift
// The driver refuses to run behind the lock screen, where gpui does not draw.
import Foundation
import CoreGraphics
let d = CGSessionCopyCurrentDictionary() as? [String: Any] ?? [:]
print(d["CGSSessionScreenIsLocked"] as? Int ?? 0)
