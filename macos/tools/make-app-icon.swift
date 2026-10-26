#!/usr/bin/env swift
// 設定アプリ「ikari 設定」のアプリのアイコン（錨）を .icns で生成する（ADR 0005 §3.2、§11.3）。
// 使い方: swift macos/tools/make-app-icon.swift [出力ディレクトリ]
// 既定の出力先は macos/Resources/settings（AppIcon.icns。生成物はコミットする。build.sh は生成しない）。
// 錨の形は make-icons.swift（入力メソッドのアイコン）と同じ。ここは大きいサイズなので、赤い角丸の地に白で描く。
// モードのアイコン（make-icons.swift、macos/Resources/icons）はレーン S3 の持ち分で、このスクリプトは触らない。
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// 錨を描く座標系（make-icons.swift と同じ 16 x 16）
let anchorSide: CGFloat = 16
let anchorStroke: CGFloat = 1.5
/// macOS のアイコンの格子: 1024 の中で、地の角丸の四角は 824（外側 100 は影のための余白）
let canvas: CGFloat = 1024
let tileSide: CGFloat = 824
let tileCornerRadius: CGFloat = 185
/// 錨が地に占める割合
let anchorScale: CGFloat = 0.62

/// iconutil が読む名前とピクセル数
let iconsetEntries: [(name: String, pixels: Int)] = [
    ("icon_16x16.png", 16), ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32), ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128), ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256), ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512), ("icon_512x512@2x.png", 1024),
]

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

/// 錨の線画（make-icons.swift の drawAnchor と同じ点）。`c` は 16 x 16 の座標系に変換済み
func drawAnchor(_ c: CGContext) {
    let side = anchorSide
    let mid = side / 2
    c.strokeEllipse(in: CGRect(x: mid - 1.6, y: 12.4, width: 3.2, height: 3.2))
    c.move(to: CGPoint(x: mid, y: 12.4))
    c.addLine(to: CGPoint(x: mid, y: 2.4))
    c.move(to: CGPoint(x: mid - 3.2, y: 10.2))
    c.addLine(to: CGPoint(x: mid + 3.2, y: 10.2))
    c.strokePath()
    c.move(to: CGPoint(x: 1.6, y: 6.6))
    c.addQuadCurve(to: CGPoint(x: mid, y: 1.6), control: CGPoint(x: 2.4, y: 1.6))
    c.addQuadCurve(to: CGPoint(x: side - 1.6, y: 6.6), control: CGPoint(x: side - 2.4, y: 1.6))
    c.strokePath()
    c.move(to: CGPoint(x: mid - 2, y: 2.0))
    c.addLine(to: CGPoint(x: mid + 2, y: 2.0))
    c.strokePath()
}

/// 1024 x 1024 の絵を `pixels` 四方に描く
func render(pixels: Int) -> CGImage {
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    guard
        let c = CGContext(
            data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: 0, space: space,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { fail("cannot create a bitmap context") }
    c.interpolationQuality = .high
    c.scaleBy(x: CGFloat(pixels) / canvas, y: CGFloat(pixels) / canvas)

    // 地: 角丸の四角（影つき）に縦のグラデーション
    let inset = (canvas - tileSide) / 2
    let tile = CGRect(x: inset, y: inset, width: tileSide, height: tileSide)
    let tilePath = CGPath(roundedRect: tile, cornerWidth: tileCornerRadius, cornerHeight: tileCornerRadius, transform: nil)
    c.saveGState()
    c.setShadow(offset: CGSize(width: 0, height: -12), blur: 24, color: CGColor(gray: 0, alpha: 0.35))
    c.addPath(tilePath)
    c.setFillColor(CGColor(srgbRed: 0.80, green: 0.20, blue: 0.16, alpha: 1))
    c.fillPath()
    c.restoreGState()

    c.saveGState()
    c.addPath(tilePath)
    c.clip()
    let colors = [
        CGColor(srgbRed: 0.96, green: 0.36, blue: 0.24, alpha: 1),
        CGColor(srgbRed: 0.66, green: 0.11, blue: 0.14, alpha: 1),
    ] as CFArray
    let gradient = CGGradient(colorsSpace: space, colors: colors, locations: [0, 1])!
    c.drawLinearGradient(
        gradient, start: CGPoint(x: canvas / 2, y: tile.maxY), end: CGPoint(x: canvas / 2, y: tile.minY), options: [])
    c.restoreGState()

    // 錨: 地の中央に白で
    let anchorPoints = tileSide * anchorScale
    let scale = anchorPoints / anchorSide
    c.saveGState()
    c.translateBy(x: (canvas - anchorPoints) / 2, y: (canvas - anchorPoints) / 2)
    c.scaleBy(x: scale, y: scale)
    c.setStrokeColor(CGColor(gray: 1, alpha: 1))
    c.setLineWidth(anchorStroke)
    c.setLineCap(.round)
    c.setLineJoin(.round)
    drawAnchor(c)
    c.restoreGState()

    guard let image = c.makeImage() else { fail("cannot make an image") }
    return image
}

func writePNG(_ image: CGImage, to url: URL) {
    guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { fail("cannot write \(url.path)") }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { fail("cannot finalize \(url.path)") }
}

let defaultOutput = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
    .deletingLastPathComponent().appendingPathComponent("Resources/settings")
let output = CommandLine.arguments.count > 1 ? URL(fileURLWithPath: CommandLine.arguments[1]) : defaultOutput
let fileManager = FileManager.default
try fileManager.createDirectory(at: output, withIntermediateDirectories: true)

let iconset = fileManager.temporaryDirectory.appendingPathComponent("ikari-app-icon-\(UUID().uuidString).iconset")
try fileManager.createDirectory(at: iconset, withIntermediateDirectories: true)
defer { try? fileManager.removeItem(at: iconset) }

var cache: [Int: CGImage] = [:]
for entry in iconsetEntries {
    let image = cache[entry.pixels] ?? render(pixels: entry.pixels)
    cache[entry.pixels] = image
    writePNG(image, to: iconset.appendingPathComponent(entry.name))
}

let icns = output.appendingPathComponent("AppIcon.icns")
let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", "-o", icns.path, iconset.path]
try process.run()
process.waitUntilExit()
guard process.terminationStatus == 0 else { fail("iconutil failed (\(process.terminationStatus))") }
print("wrote \(icns.path)")
