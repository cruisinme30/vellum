// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Puts server-rendered HTML into the page under the app's Content Security
 * Policy, which allows no inline styles (`style-src 'self'`). The renderer
 * colours SVG with `style` attributes and Mermaid adds a `<style>`
 * element; parsed into the page as markup, the browser would drop both. So
 * the HTML is parsed in a detached document, each `style` attribute is
 * re-applied through the CSS object model, and `<style>` text goes into a
 * constructed stylesheet; both are allowed by the policy (ADR 0068, D287).
 */

/** Adds a constructed stylesheet to the document; returns its removal. */
export function adoptCss(css: string, document_: Document = document): () => void {
  const Sheet = document_.defaultView?.CSSStyleSheet;
  if (
    Sheet !== undefined &&
    "replaceSync" in Sheet.prototype &&
    "adoptedStyleSheets" in document_
  ) {
    const sheet = new Sheet();
    sheet.replaceSync(css);
    document_.adoptedStyleSheets = [...document_.adoptedStyleSheets, sheet];
    return () => {
      document_.adoptedStyleSheets = document_.adoptedStyleSheets.filter((s) => s !== sheet);
    };
  }
  // No constructed stylesheets (old engines, the unit-test DOM).
  const style = document_.createElement("style");
  style.textContent = css;
  document_.head.append(style);
  return () => {
    style.remove();
  };
}

/** Replaces `root`'s children with `html`; returns a cleanup for its styles. */
export function mountRendered(root: HTMLElement, html: string): () => void {
  const document_ = root.ownerDocument;
  const parsed = new DOMParser().parseFromString(
    `<!doctype html><body>${html}</body>`,
    "text/html",
  );
  const inline: [Element, string][] = [];
  for (const element of parsed.body.querySelectorAll("[style]")) {
    inline.push([element, element.getAttribute("style") ?? ""]);
    element.removeAttribute("style");
  }
  const sheets: string[] = [];
  for (const style of parsed.body.querySelectorAll("style")) {
    sheets.push(style.textContent);
    style.remove();
  }
  const nodes = [...parsed.body.childNodes].map((node) => document_.importNode(node, true));
  root.replaceChildren(...nodes);
  // The imported copies are new elements: re-apply styles by position.
  const imported = root.querySelectorAll("*");
  const originals = [...parsed.body.querySelectorAll("*")];
  const styles = new Map(inline);
  for (const [index, original] of originals.entries()) {
    const value = styles.get(original);
    const copy = imported[index];
    if (value !== undefined && copy !== undefined && "style" in copy) {
      (copy as HTMLElement).style.cssText = value;
    }
  }
  const removals = sheets.filter((css) => css.trim() !== "").map((css) => adoptCss(css, document_));
  return () => {
    for (const remove of removals) remove();
  };
}
