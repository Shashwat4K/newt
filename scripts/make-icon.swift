#!/usr/bin/env swift
//
// Draws newt's icon and writes an .icns.
//
// Generated rather than checked in as a binary: the shape is a few lines of
// drawing code, and a script can be read and changed, which a blob cannot.

import AppKit
import Foundation

let arguments = CommandLine.arguments
guard arguments.count > 1 else {
    FileHandle.standardError.write(Data("usage: make-icon.swift <output.icns>\n".utf8))
    exit(2)
}
let outputPath = arguments[1]

/// Draw the icon at a given pixel size.
func drawIcon(size: Int) -> NSBitmapImageRep {
    let dimension = CGFloat(size)
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: size,
        pixelsHigh: size,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    )!

    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

    // Rounded dark square, matching the terminal's own background.
    let inset = dimension * 0.06
    let body = NSRect(x: inset, y: inset, width: dimension - inset * 2, height: dimension - inset * 2)
    let radius = dimension * 0.22
    let path = NSBezierPath(roundedRect: body, xRadius: radius, yRadius: radius)
    NSColor(srgbRed: 0.08, green: 0.08, blue: 0.10, alpha: 1).setFill()
    path.fill()

    NSColor(srgbRed: 0.30, green: 0.32, blue: 0.38, alpha: 1).setStroke()
    path.lineWidth = max(1, dimension * 0.008)
    path.stroke()

    // A prompt: chevron plus cursor block. Drawn as geometry rather than text so
    // it does not depend on a font being installed.
    let stroke = dimension * 0.055
    let chevron = NSBezierPath()
    chevron.move(to: NSPoint(x: dimension * 0.30, y: dimension * 0.62))
    chevron.line(to: NSPoint(x: dimension * 0.45, y: dimension * 0.50))
    chevron.line(to: NSPoint(x: dimension * 0.30, y: dimension * 0.38))
    chevron.lineWidth = stroke
    chevron.lineCapStyle = .round
    chevron.lineJoinStyle = .round
    NSColor(srgbRed: 0.53, green: 0.82, blue: 0.47, alpha: 1).setStroke()
    chevron.stroke()

    NSColor(srgbRed: 0.85, green: 0.86, blue: 0.88, alpha: 1).setFill()
    NSBezierPath(
        rect: NSRect(
            x: dimension * 0.52,
            y: dimension * 0.36,
            width: dimension * 0.20,
            height: stroke
        )
    ).fill()

    NSGraphicsContext.restoreGraphicsState()
    return rep
}

// The sizes iconutil expects, each in 1x and 2x.
let iconSet = URL(fileURLWithPath: NSTemporaryDirectory())
    .appendingPathComponent("newt-\(UUID().uuidString).iconset")
try FileManager.default.createDirectory(at: iconSet, withIntermediateDirectories: true)

for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = base * scale
        let rep = drawIcon(size: pixels)
        guard let png = rep.representation(using: .png, properties: [:]) else {
            FileHandle.standardError.write(Data("could not encode \(pixels)px\n".utf8))
            exit(1)
        }
        let suffix = scale == 1 ? "" : "@2x"
        let name = "icon_\(base)x\(base)\(suffix).png"
        try png.write(to: iconSet.appendingPathComponent(name))
    }
}

let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconSet.path, "-o", outputPath]
try iconutil.run()
iconutil.waitUntilExit()

try? FileManager.default.removeItem(at: iconSet)

guard iconutil.terminationStatus == 0 else {
    FileHandle.standardError.write(Data("iconutil failed\n".utf8))
    exit(1)
}
print("wrote \(outputPath)")
