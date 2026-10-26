#!/usr/bin/env swift
// ikari のアイコンを PDF で生成する（ADR 0004 §6.6、ADR 0005 §3、§15.2）。使い方: swift macos/tools/make-icons.swift [出力ディレクトリ]
// 既定の出力先は macos/Resources/icons。16pt の単色テンプレート画像（黒。macOS が明暗に合わせて色を付ける）。
//   ikari.pdf          錨。軸を I ビーム（上下に横線のある縦棒）に見立てる。入力メソッドのアイコン（システム設定の入力ソース一覧）
//   ikari-japanese.pdf 塗りの円に「あ」を白抜き（入力モード .Japanese）。メニューバーに出る
// 英数モードはなくした（ADR 0005 §15.2-1）ので、英数のアイコンは作らない。
// 最後に、日本語のアイコンを 16px と 32px にラスタライズして、不透明な画素の割合が下限（0.45）以上かを確かめる。
// 下回ったら失敗（終了コード 1）にする。「塗りの円」であることを数で縛る（ADR 0005 §3.2）。
import CoreGraphics
import CoreText
import Foundation

let side: CGFloat = 16
let stroke: CGFloat = 1.5  // 16pt でも潰れない太さ
let circleDiameter: CGFloat = 15  // 日本語のアイコンの円（ADR 0005 §3.2）
let japaneseGlyph = "あ"
let japaneseFont = "HiraginoSans-W6"
let japaneseFontSize: CGFloat = 10.5
/// 日本語のアイコンの不透明な画素の割合の下限（ADR 0005 §3.2）
let minimumOpaqueRatio = 0.45
let checkedPixelSizes = [16, 32]

func render(_ name: String, into directory: URL, draw: (CGContext) -> Void) {
    var box = CGRect(x: 0, y: 0, width: side, height: side)
    let url = directory.appendingPathComponent(name) as CFURL
    guard let context = CGContext(url, mediaBox: &box, nil) else {
        FileHandle.standardError.write(Data("cannot create \(name)\n".utf8))
        exit(1)
    }
    context.beginPDFPage(nil)
    context.setStrokeColor(CGColor(gray: 0, alpha: 1))
    context.setFillColor(CGColor(gray: 0, alpha: 1))
    context.setLineWidth(stroke)
    context.setLineCap(.round)
    context.setLineJoin(.round)
    draw(context)
    context.endPDFPage()
    context.closePDF()
}

/// 錨: 上の小さな輪、I ビームの軸（上下に横線）、横木、下の弧と先端
func drawAnchor(_ c: CGContext) {
    let mid = side / 2
    // 輪
    c.strokeEllipse(in: CGRect(x: mid - 1.6, y: 12.4, width: 3.2, height: 3.2))
    // 軸（I ビーム）: 縦棒 + 下の横線。上は輪が受ける
    c.move(to: CGPoint(x: mid, y: 12.4))
    c.addLine(to: CGPoint(x: mid, y: 2.4))
    // 横木
    c.move(to: CGPoint(x: mid - 3.2, y: 10.2))
    c.addLine(to: CGPoint(x: mid + 3.2, y: 10.2))
    c.strokePath()
    // 下の弧（左右の腕）と先端
    c.move(to: CGPoint(x: 1.6, y: 6.6))
    c.addQuadCurve(to: CGPoint(x: mid, y: 1.6), control: CGPoint(x: 2.4, y: 1.6))
    c.addQuadCurve(to: CGPoint(x: side - 1.6, y: 6.6), control: CGPoint(x: side - 2.4, y: 1.6))
    c.strokePath()
    // I ビームの下端の横線
    c.move(to: CGPoint(x: mid - 2, y: 2.0))
    c.addLine(to: CGPoint(x: mid + 2, y: 2.0))
    c.strokePath()
}

/// 塗りの円に、文字を白抜きにする。くり抜きは path の段階で行う（ブレンドモードは PDF で再現されないので使わない）
func drawFilledCircle(_ c: CGContext, glyph: String, fontName: String, fontSize: CGFloat) {
    let inset = (side - circleDiameter) / 2
    let circle = CGPath(ellipseIn: CGRect(x: inset, y: inset, width: circleDiameter, height: circleDiameter), transform: nil)

    let font = CTFontCreateWithName(fontName as CFString, fontSize, nil)
    var characters = Array(glyph.utf16)
    var glyphs = [CGGlyph](repeating: 0, count: characters.count)
    guard CTFontGetGlyphsForCharacters(font, &characters, &glyphs, characters.count), let outline = CTFontCreatePathForGlyph(font, glyphs[0], nil)
    else {
        FileHandle.standardError.write(Data("font \(fontName) has no glyph for \(glyph)\n".utf8))
        exit(1)
    }
    // 字形の外接矩形の中心を、円の中心に合わせる
    let box = outline.boundingBoxOfPath
    var move = CGAffineTransform(translationX: side / 2 - box.midX, y: side / 2 - box.midY)
    guard let centered = outline.copy(using: &move) else { exit(1) }
    let punched = circle.subtracting(centered)
    c.addPath(punched)
    c.fillPath()
}

// MARK: 検査

/// PDF を `pixels` 四方にラスタライズして、不透明度（アルファ）の平均を返す
func opaqueRatio(of url: URL, pixels: Int) -> Double {
    guard let document = CGPDFDocument(url as CFURL), let page = document.page(at: 1),
        let context = CGContext(
            data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: pixels * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
        let data = context.data
    else {
        FileHandle.standardError.write(Data("cannot rasterize \(url.lastPathComponent)\n".utf8))
        exit(1)
    }
    let scale = CGFloat(pixels) / side
    context.scaleBy(x: scale, y: scale)
    context.drawPDFPage(page)
    let bytes = data.assumingMemoryBound(to: UInt8.self)
    var total = 0
    for index in 0..<(pixels * pixels) { total += Int(bytes[index * 4 + 3]) }
    return Double(total) / Double(pixels * pixels * 255)
}

let defaultOutput = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
    .deletingLastPathComponent().appendingPathComponent("Resources/icons")
let output = CommandLine.arguments.count > 1 ? URL(fileURLWithPath: CommandLine.arguments[1]) : defaultOutput
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)

render("ikari.pdf", into: output, draw: drawAnchor)
render("ikari-japanese.pdf", into: output) {
    drawFilledCircle($0, glyph: japaneseGlyph, fontName: japaneseFont, fontSize: japaneseFontSize)
}
print("wrote icons to \(output.path)")

var failed = false
for pixels in checkedPixelSizes {
    let ratio = opaqueRatio(of: output.appendingPathComponent("ikari-japanese.pdf"), pixels: pixels)
    let verdict = ratio >= minimumOpaqueRatio ? "ok" : "TOO LIGHT"
    print(String(format: "ikari-japanese.pdf @%dpx: opaque ratio %.3f (>= %.2f) %@", pixels, ratio, minimumOpaqueRatio, verdict))
    if ratio < minimumOpaqueRatio { failed = true }
}
if failed { exit(1) }
