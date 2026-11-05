// Render the 1200x630 share card for agentdoctor.io and agentdoctor.cn.
// CoreGraphics + CoreText only: no extra dependencies on this machine.
import Foundation
import CoreGraphics
import CoreText
import ImageIO
import UniformTypeIdentifiers

let W: CGFloat = 1200, H: CGFloat = 630

func rgb(_ r: Int, _ g: Int, _ b: Int, _ a: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat(r) / 255, green: CGFloat(g) / 255, blue: CGFloat(b) / 255, alpha: a)
}

func makeContext() -> CGContext {
    let cs = CGColorSpaceCreateDeviceRGB()
    let ctx = CGContext(data: nil, width: Int(W), height: Int(H), bitsPerComponent: 8, bytesPerRow: 0,
                        space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.setFillColor(rgb(7, 11, 16))
    ctx.fill(CGRect(x: 0, y: 0, width: W, height: H))
    return ctx
}

/// Glow behind the card, drawn in design coordinates (origin top-left).
func drawGlow(_ ctx: CGContext, at x: CGFloat, y: CGFloat, radius: CGFloat, color: CGColor) {
    let cs = CGColorSpaceCreateDeviceRGB()
    let grad = CGGradient(colorsSpace: cs, colors: [color, color.copy(alpha: 0)!] as CFArray, locations: [0, 1])!
    ctx.saveGState()
    ctx.drawRadialGradient(grad,
                           startCenter: CGPoint(x: x, y: H - y), startRadius: 0,
                           endCenter: CGPoint(x: x, y: H - y), endRadius: radius,
                           options: [])
    ctx.restoreGState()
}

func font(_ name: String, _ size: CGFloat) -> CTFont {
    CTFontCreateWithName(name as CFString, size, nil)
}

func lineWidth(_ line: CTLine) -> CGFloat {
    let w = CTLineGetTypographicBounds(line, nil, nil, nil)
    return CGFloat(w)
}

/// Plain text, drawn at (x, baseline) in design coordinates (origin top-left).
func drawText(_ ctx: CGContext, _ text: String, _ f: CTFont, color: CGColor, x: CGFloat, baseline: CGFloat) {
    let attr = NSAttributedString(string: text, attributes: [
        NSAttributedString.Key(kCTFontAttributeName as String): f,
        NSAttributedString.Key(kCTForegroundColorAttributeName as String): color,
    ])
    let line = CTLineCreateWithAttributedString(attr)
    ctx.textPosition = CGPoint(x: x, y: H - baseline)
    CTLineDraw(line, ctx)
}

/// Text filled with a gradient, left edge at x, baseline in design coordinates.
func drawGradientText(_ ctx: CGContext, _ text: String, _ f: CTFont, x: CGFloat, baseline: CGFloat, colors: [CGColor]) {
    let attr = NSAttributedString(string: text, attributes: [
        NSAttributedString.Key(kCTFontAttributeName as String): f,
    ])
    let line = CTLineCreateWithAttributedString(attr)
    let width = lineWidth(line)
    let ascent = CTFontGetAscent(f)
    let descent = CTFontGetDescent(f)
    let mh = CGFloat(ceil(ascent + descent))
    let mw = CGFloat(ceil(width))
    let cs = CGColorSpaceCreateDeviceRGB()

    // 1. glyphs into their own bitmap: only the alpha channel matters
    guard let mctx = CGContext(data: nil, width: Int(mw), height: Int(mh), bitsPerComponent: 8,
                               bytesPerRow: 0, space: cs,
                               bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
    mctx.textPosition = .zero
    mctx.translateBy(x: 0, y: mh - ascent)
    CTLineDraw(line, mctx)
    guard let mask = mctx.makeImage() else { return }

    // 2. gradient, then keep it only where the glyphs are
    guard let tctx = CGContext(data: nil, width: Int(mw), height: Int(mh), bitsPerComponent: 8,
                               bytesPerRow: 0, space: cs,
                               bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
    let locations: [CGFloat] = colors.count == 2 ? [0, 1] : [0, 0.5, 1]
    let grad = CGGradient(colorsSpace: cs, colors: colors as CFArray, locations: locations)!
    tctx.drawLinearGradient(grad,
                            start: CGPoint(x: 0, y: 0),
                            end: CGPoint(x: mw, y: mh),
                            options: [])
    tctx.setBlendMode(.destinationIn)
    tctx.draw(mask, in: CGRect(x: 0, y: 0, width: mw, height: mh))
    guard let painted = tctx.makeImage() else { return }

    ctx.draw(painted, in: CGRect(x: x, y: H - baseline + ascent - mh, width: mw, height: mh))
}

func drawLogo(_ ctx: CGContext, path: String, x: CGFloat, y: CGFloat, size: CGFloat) {
    let url = URL(fileURLWithPath: path)
    guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
          let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
        FileHandle.standardError.write("cannot load \(path)\n".data(using: .utf8)!)
        return
    }
    ctx.draw(img, in: CGRect(x: x, y: H - y - size, width: size, height: size))
}

func writePNG(_ ctx: CGContext, to path: String) {
    guard let img = ctx.makeImage() else { return }
    let out = URL(fileURLWithPath: path)
    guard let dest = CGImageDestinationCreateWithURL(out as CFURL, UTType.png.identifier as CFString, 1, nil) else { return }
    CGImageDestinationAddImage(dest, img, nil)
    CGImageDestinationFinalize(dest)
    print("wrote \(path)")
}

// ---------------------------------------------------------------- english card
func englishCard() {
    let ctx = makeContext()
    drawGlow(ctx, at: 1140, y: 0, radius: 560, color: rgb(46, 200, 214, 0.22))
    drawGlow(ctx, at: 0, y: 40, radius: 520, color: rgb(228, 178, 58, 0.13))

    drawLogo(ctx, path: "/Users/airlu/Documents/agent-doctor/site/assets/icon-128.png", x: 84, y: 84, size: 56)
    drawText(ctx, "Agent Doctor", font("AvenirNext-DemiBold", 30), color: rgb(238, 243, 248), x: 158, baseline: 121)

    let h1 = font("AvenirNext-Heavy", 82)
    drawText(ctx, "Agents break.", h1, color: rgb(238, 243, 248), x: 84, baseline: 290)
    drawGradientText(ctx, "Now you can fix them.", h1, x: 84, baseline: 380,
                     colors: [rgb(216, 255, 251), rgb(46, 200, 214), rgb(228, 178, 58)])

    drawText(ctx, "Scan, diagnose, repair, roll back — one desktop app for the agents",
             font("HelveticaNeue", 27), color: rgb(169, 182, 198), x: 84, baseline: 460)
    drawText(ctx, "already on your computer.",
             font("HelveticaNeue", 27), color: rgb(169, 182, 198), x: 84, baseline: 496)

    drawText(ctx, "agentdoctor.io", font("Menlo-Bold", 21), color: rgb(46, 200, 214), x: 84, baseline: 570)

    writePNG(ctx, to: "/Users/airlu/Documents/agent-doctor/site/assets/og.png")
}

// ---------------------------------------------------------------- chinese card
func chineseCard() {
    let ctx = makeContext()
    drawGlow(ctx, at: 1140, y: 0, radius: 560, color: rgb(46, 200, 214, 0.22))
    drawGlow(ctx, at: 0, y: 40, radius: 520, color: rgb(228, 178, 58, 0.13))

    drawLogo(ctx, path: "/Users/airlu/Documents/teamups/public/logos/agentdoctor.png", x: 84, y: 84, size: 56)
    drawText(ctx, "AgentDoctor", font("PingFangSC-Semibold", 30), color: rgb(238, 243, 248), x: 158, baseline: 121)

    let h1 = font("PingFangSC-Semibold", 78)
    drawText(ctx, "装得上，", h1, color: rgb(238, 243, 248), x: 84, baseline: 290)
    drawGradientText(ctx, "也修得好。", h1, x: 84, baseline: 382,
                     colors: [rgb(216, 255, 251), rgb(46, 200, 214), rgb(228, 178, 58)])

    drawText(ctx, "国内外多智能体一键安装，接入国内主流模型；",
             font("PingFangSC-Regular", 27), color: rgb(169, 182, 198), x: 84, baseline: 460)
    drawText(ctx, "装完还能查、能修、能回滚。",
             font("PingFangSC-Regular", 27), color: rgb(169, 182, 198), x: 84, baseline: 496)

    drawText(ctx, "agentdoctor.cn", font("Menlo-Bold", 21), color: rgb(46, 200, 214), x: 84, baseline: 570)

    writePNG(ctx, to: "/Users/airlu/Documents/teamups/public/logos/og.png")
}

englishCard()
chineseCard()
