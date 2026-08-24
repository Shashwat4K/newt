import AppKit
import CoreText
import NewtKit

/// The font and the cell geometry derived from it.
///
/// A terminal is a fixed grid, so every glyph is positioned by cell rather than
/// by the shaper's advances. The metrics here define that grid.
struct TerminalFont {
    let regular: CTFont
    let bold: CTFont
    let italic: CTFont
    let boldItalic: CTFont

    let cellWidth: CGFloat
    let cellHeight: CGFloat
    /// Distance from the bottom of a cell up to the text baseline.
    let baseline: CGFloat

    init(size: CGFloat) {
        // Prefer a Nerd Font when one is installed: prompt themes and TUIs
        // lean heavily on Private Use Area icons that SF Mono and Menlo have no
        // glyphs for, and CoreText cannot substitute what nothing provides.
        // Still one hard-coded font, just a better-informed choice.
        let candidates = [
            "MesloLGS NF",
            "JetBrainsMono Nerd Font",
            "FiraCode Nerd Font",
            "SFMono-Regular",
            "Menlo",
        ]
        let base: NSFont =
            candidates.lazy.compactMap { NSFont(name: $0, size: size) }.first
            ?? NSFont.monospacedSystemFont(ofSize: size, weight: .regular)

        regular = base as CTFont
        bold = Self.variant(of: regular, size: size, traits: .traitBold)
        italic = Self.variant(of: regular, size: size, traits: .traitItalic)
        boldItalic = Self.variant(of: regular, size: size, traits: [.traitBold, .traitItalic])

        // Advance of a representative glyph rather than the font's maximum:
        // the maximum is skewed by rare wide glyphs in some monospaced fonts.
        var glyph = CTFontGetGlyphWithName(regular, "M" as CFString)
        var advance = CGSize.zero
        CTFontGetAdvancesForGlyphs(regular, .horizontal, &glyph, &advance, 1)

        let ascent = CTFontGetAscent(regular)
        let descent = CTFontGetDescent(regular)
        let leading = CTFontGetLeading(regular)

        // Rounded to whole pixels: fractional cell sizes accumulate across a
        // row and leave the grid visibly misaligned at the right edge.
        cellWidth = advance.width.rounded(.up)
        cellHeight = (ascent + descent + leading).rounded(.up)
        baseline = descent.rounded(.up)
    }

    /// Pixel-to-cell conversions for this font.
    var geometry: GridGeometry {
        GridGeometry(cellWidth: cellWidth, cellHeight: cellHeight)
    }

    /// Font to use for a cell's attribute flags.
    func font(bold isBold: Bool, italic isItalic: Bool) -> CTFont {
        switch (isBold, isItalic) {
        case (true, true): return boldItalic
        case (true, false): return bold
        case (false, true): return italic
        case (false, false): return regular
        }
    }

    private static func variant(
        of font: CTFont,
        size: CGFloat,
        traits: CTFontSymbolicTraits
    ) -> CTFont {
        // Returns nil when the family has no such face; the regular font is a
        // better answer than a synthesised one.
        CTFontCreateCopyWithSymbolicTraits(font, size, nil, traits, traits) ?? font
    }
}
