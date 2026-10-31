// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Which build assets Vite may inline as `data:` URLs.
 *
 * The server's Content Security Policy allows fonts from the app's own
 * origin only (`font-src 'self'`), so an inlined font is blocked and its
 * characters fall back to another font. Fonts therefore always ship as
 * files; everything else keeps Vite's default size rule (`undefined`).
 */
export function inlineAsset(filePath: string): false | undefined {
  return /\.(woff2?|ttf|otf|eot)(\?.*)?$/i.test(filePath) ? false : undefined;
}
