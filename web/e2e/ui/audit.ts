// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The layout audit (ADR 0031, D123). `layoutAudit` runs inside the page
 * through `page.evaluate`, so it must stay self-contained: no imports, no
 * closures over module scope.
 *
 * Opt-outs for intended designs live in the markup, next to the design:
 * `data-allow-overlap` (a badge on an avatar, a floating label) and
 * `data-allow-clip` (a deliberately clipped preview).
 */

/* eslint-disable unicorn/consistent-function-scoping -- helpers must live inside the function that runs in the page */

type Rule =
  | "horizontal-overflow"
  | "outside-viewport"
  | "overlap"
  | "clipped"
  | "text-overflows-box"
  | "image-not-loaded"
  | "icon-missing"
  | "font-failed";

export interface Finding {
  rule: Rule;
  /** A short CSS path to the element. */
  target: string;
  message: string;
}

// One pass over the DOM, serialised into the page by page.evaluate, so it
// cannot call helpers from this module: everything it uses is inside it.
// eslint-disable-next-line complexity, sonarjs/cognitive-complexity
export function layoutAudit(): Finding[] {
  const findings: Finding[] = [];
  const root = document.documentElement;
  const viewportWidth = root.clientWidth;
  const tolerance = 1;

  const path = (element: Element): string => {
    const parts: string[] = [];
    let node: Element | null = element;
    while (node !== null && node !== root && parts.length < 4) {
      let part = node.tagName.toLowerCase();
      if (node.id === "") {
        const classes = [...node.classList].slice(0, 2);
        if (classes.length > 0) part += `.${classes.join(".")}`;
      } else {
        part += `#${node.id}`;
        parts.unshift(part);
        break;
      }
      parts.unshift(part);
      node = node.parentElement;
    }
    return parts.join(" > ");
  };
  const add = (rule: Rule, element: Element, message: string): void => {
    findings.push({ rule, target: path(element), message });
  };

  const visible = (element: Element): boolean => {
    if (!element.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  // The usual "visually hidden" utility: present for screen readers only.
  const screenReaderOnly = (element: Element): boolean => {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return (
      (rect.width <= 1 && rect.height <= 1) ||
      style.getPropertyValue("clip").startsWith("rect(0") ||
      style.clipPath === "inset(50%)"
    );
  };
  const clips = (value: string): boolean => value === "hidden" || value === "clip";
  const scrolls = (value: string): boolean => value === "auto" || value === "scroll";
  const optedOut = (element: Element, attribute: string): boolean =>
    element.closest(`[${attribute}]`) !== null;
  /** True when an ancestor below <body> clips or scrolls horizontally. */
  const insideHorizontalContainer = (element: Element): boolean => {
    for (
      let node = element.parentElement;
      node !== null && node !== document.body;
      node = node.parentElement
    ) {
      const overflowX = getComputedStyle(node).overflowX;
      if (clips(overflowX) || scrolls(overflowX)) return true;
    }
    return false;
  };
  /** The part of a box its scrolling or clipping ancestors leave visible. */
  const visiblePart = (element: Element, box: DOMRect): DOMRect => {
    let { left, top, right, bottom } = box;
    for (
      let node = element.parentElement;
      node !== null && node !== document.body;
      node = node.parentElement
    ) {
      const style = getComputedStyle(node);
      if (style.overflowX === "visible" && style.overflowY === "visible") continue;
      const frame = node.getBoundingClientRect();
      if (style.overflowX !== "visible") {
        left = Math.max(left, frame.left);
        right = Math.min(right, frame.right);
      }
      if (style.overflowY !== "visible") {
        top = Math.max(top, frame.top);
        bottom = Math.min(bottom, frame.bottom);
      }
    }
    return new DOMRect(left, top, Math.max(0, right - left), Math.max(0, bottom - top));
  };
  const ownText = (element: Element): boolean =>
    [...element.childNodes].some(
      (child) => child.nodeType === Node.TEXT_NODE && (child.textContent ?? "").trim() !== "",
    );
  const replaced = new Set([
    "IMG",
    "SVG",
    "VIDEO",
    "CANVAS",
    "INPUT",
    "SELECT",
    "TEXTAREA",
    "BUTTON",
    "IFRAME",
  ]);

  const elements = [...document.body.querySelectorAll("*")].filter(
    (element) =>
      !["SCRIPT", "STYLE", "TEMPLATE", "NOSCRIPT"].includes(element.tagName) && visible(element),
  );

  // Horizontal overflow of the page: the most common responsive bug.
  if (root.scrollWidth > viewportWidth + tolerance) {
    findings.push({
      rule: "horizontal-overflow",
      target: "html",
      message: `the page scrolls sideways: ${String(root.scrollWidth)}px of content in a ${String(viewportWidth)}px viewport`,
    });
  }

  const outside = new Set<Element>();
  for (const element of elements) {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);

    // Outside the viewport horizontally, reported once for the outermost element.
    // An absolutely positioned element parked entirely off screen (a skip
    // link) is a hiding technique; if it makes the page scroll, the
    // horizontal-overflow rule reports that.
    const parked =
      (style.position === "absolute" || style.position === "fixed") &&
      (rect.right <= 0 || rect.left >= viewportWidth);
    if (
      !parked &&
      (rect.right > viewportWidth + tolerance || rect.left < -tolerance) &&
      !screenReaderOnly(element) &&
      !insideHorizontalContainer(element) &&
      !(element.parentElement !== null && outside.has(element.parentElement))
    ) {
      outside.add(element);
      add(
        "outside-viewport",
        element,
        `spans x=${rect.left.toFixed(0)}..${rect.right.toFixed(0)}, viewport is 0..${String(viewportWidth)}`,
      );
    }

    // Content cut off by its own box.
    const truncates =
      style.textOverflow === "ellipsis" || style.getPropertyValue("-webkit-line-clamp") !== "none";
    if (
      !truncates &&
      !optedOut(element, "data-allow-clip") &&
      !screenReaderOnly(element) &&
      element.textContent.trim() !== ""
    ) {
      const cutX = clips(style.overflowX) && element.scrollWidth > element.clientWidth + tolerance;
      const cutY =
        clips(style.overflowY) && element.scrollHeight > element.clientHeight + tolerance;
      if (cutX || cutY) {
        add(
          "clipped",
          element,
          `content ${String(element.scrollWidth)}×${String(element.scrollHeight)} is cut to ${String(element.clientWidth)}×${String(element.clientHeight)}`,
        );
      }
    }

    // Text spilling out of a visible box, e.g. a long word or URL without wrapping.
    if (
      ownText(element) &&
      style.display !== "inline" &&
      style.overflowX === "visible" &&
      element.clientWidth > 0 &&
      element.scrollWidth > element.clientWidth + tolerance &&
      !insideHorizontalContainer(element)
    ) {
      add(
        "text-overflows-box",
        element,
        `text is ${String(element.scrollWidth)}px wide in a ${String(element.clientWidth)}px box`,
      );
    }
  }

  // Overlap between leaves: elements that carry text or are replaced content.
  const leaves = elements
    .filter(
      (element) =>
        (ownText(element) || replaced.has(element.tagName.toUpperCase())) &&
        !screenReaderOnly(element),
    )
    // Shapes inside one drawing overlap by design; the drawing is the leaf.
    .filter(
      (element) =>
        !optedOut(element, "data-allow-overlap") && !element.parentElement?.closest("svg"),
    )
    .slice(0, 4000)
    // Per line box: an inline element that wraps has one rectangle per line,
    // and its bounding box would cover text it does not touch.
    // An inline element's padding spills over neighbouring lines by design
    // (a code chip's background), so only its content box counts.
    .flatMap((element) => {
      const style = getComputedStyle(element);
      const inset =
        style.display === "inline"
          ? {
              top: Number.parseFloat(style.paddingTop) + Number.parseFloat(style.borderTopWidth),
              bottom:
                Number.parseFloat(style.paddingBottom) + Number.parseFloat(style.borderBottomWidth),
            }
          : { top: 0, bottom: 0 };
      // Only the part a scrolling or clipping ancestor leaves visible counts:
      // a long command inside a scrolling code block does not cover its
      // neighbour.
      return [...element.getClientRects()].map((box) => ({
        element,
        rect: visiblePart(
          element,
          new DOMRect(
            box.x,
            box.y + inset.top,
            box.width,
            Math.max(0, box.height - inset.top - inset.bottom),
          ),
        ),
      }));
    })
    .toSorted((a, b) => a.rect.top - b.rect.top);
  // A fixed dialog or side sheet sits above the page and covers it by
  // design, so a leaf inside one is only checked against the sheet's own
  // leaves. A fixed bar that is not a dialog still counts: content hidden
  // behind it is a bug.
  const sheetOf = (element: Element): Element | undefined => {
    const sheet = element.closest("dialog, aside, [role='dialog'], [role='alertdialog']");
    if (sheet === null) return undefined;
    return getComputedStyle(sheet).position === "fixed" || sheet.matches("dialog[open]")
      ? sheet
      : sheetOf(sheet.parentElement ?? document.documentElement);
  };
  const minimum = 2;
  const reported = new Set<string>();
  for (let index = 0; index < leaves.length; index += 1) {
    const a = leaves[index];
    if (a === undefined) continue;
    for (let next = index + 1; next < leaves.length; next += 1) {
      const b = leaves[next];
      if (b === undefined || b.rect.top >= a.rect.bottom - minimum) break;
      if (a.element.contains(b.element) || b.element.contains(a.element)) continue;
      if (sheetOf(a.element) !== sheetOf(b.element)) continue;
      const width = Math.min(a.rect.right, b.rect.right) - Math.max(a.rect.left, b.rect.left);
      const height = Math.min(a.rect.bottom, b.rect.bottom) - Math.max(a.rect.top, b.rect.top);
      const key = `${path(a.element)}|${path(b.element)}`;
      if (width > minimum && height > minimum && !reported.has(key)) {
        reported.add(key);
        add(
          "overlap",
          a.element,
          `overlaps ${path(b.element)} by ${width.toFixed(0)}×${height.toFixed(0)}px`,
        );
      }
    }
  }

  // Resources that did not arrive.
  for (const image of document.images) {
    const lazyAndAway =
      image.loading === "lazy" && image.getBoundingClientRect().top > window.innerHeight;
    if (!lazyAndAway && visible(image) && (!image.complete || image.naturalWidth === 0)) {
      add("image-not-loaded", image, `image ${image.currentSrc || image.src} did not load`);
    }
  }
  for (const use of document.querySelectorAll("use")) {
    const reference = use.getAttribute("href") ?? use.getAttribute("xlink:href") ?? "";
    if (reference.startsWith("#") && document.querySelector(reference) === null) {
      add("icon-missing", use, `icon sprite ${reference} is not in the page`);
    }
  }
  for (const face of document.fonts) {
    // WebKit marks a font-display: optional face as "error" when it misses
    // the short block period, and the page keeps its fallback font, which is
    // what optional asks for. A missing or blocked font file is still caught
    // by the harness as a failed request or an HTTP error.
    if (face.status === "error" && face.display !== "optional") {
      findings.push({
        rule: "font-failed",
        target: "document.fonts",
        message: `font ${face.family} (${face.weight}) failed to load`,
      });
    }
  }
  return findings;
}
