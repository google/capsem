// Bundled terminal fonts are declared with `font-display: swap`, so the
// browser does not fetch one until something asks for it. xterm.js measures
// its cell size once, when the font is set; if the font arrives after that,
// every glyph is drawn in a cell sized for the fallback.

/** The part of `document.fonts` this needs. */
export interface FontLoader {
  load(font: string): Promise<unknown>;
}

/**
 * Fetch `fontFamily` so the terminal measures the real font, not a fallback.
 * A font that cannot be loaded is not an error: the family list ends in a
 * generic and the terminal must still open.
 */
export async function loadTerminalFont(fonts: FontLoader, fontFamily: string, fontSize: number): Promise<void> {
  try {
    await fonts.load(`${fontSize}px ${fontFamily}`);
  } catch {
    // Fall back to the next family in the list.
  }
}
