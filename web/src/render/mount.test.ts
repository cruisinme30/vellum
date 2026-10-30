// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { adoptCss, mountRendered } from "./mount";

describe("mountRendered", () => {
  it("re-applies inline styles through the CSS object model and lifts out <style>", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const html = `<article class="vl-doc"><figure class="vl-figure"><svg viewBox="0 0 10 10"><style>#m .node { fill: red; }</style><rect id="r" style="fill:var(--vl-chart-1);stroke-width:2" width="5" height="5"></rect></svg><p id="p" style="color: blue">Hi</p></figure></article>`;
    const before = document.head.querySelectorAll("style").length;
    const unmount = mountRendered(root, html);

    expect(root.querySelector("style")).toBeNull();
    const rect = root.querySelector<SVGElement>("#r");
    expect(rect?.style.getPropertyValue("fill")).toBe("var(--vl-chart-1)");
    expect(rect?.style.getPropertyValue("stroke-width")).toBe("2");
    expect(root.querySelector<HTMLElement>("#p")?.style.color).toBe("blue");
    expect(root.querySelector("figure")?.className).toBe("vl-figure");
    // The unit-test DOM has no constructed stylesheets: the text goes to <head>.
    expect(document.head.querySelectorAll("style")).toHaveLength(before + 1);

    unmount();
    expect(document.head.querySelectorAll("style")).toHaveLength(before);
    root.remove();
  });

  it("replaces what was there", () => {
    const root = document.createElement("div");
    root.innerHTML = "<p>old</p>";
    mountRendered(root, "<p>new</p>")();
    expect(root.textContent).toBe("new");
  });
});

describe("adoptCss", () => {
  it("adopts a constructed stylesheet where the browser has them", () => {
    let adopted: unknown[] = [];
    class Sheet {
      css = "";
      replaceSync(css: string) {
        this.css = css;
      }
    }
    const fake = {
      defaultView: { CSSStyleSheet: Sheet },
      get adoptedStyleSheets() {
        return adopted;
      },
      set adoptedStyleSheets(value: unknown[]) {
        adopted = value;
      },
    } as unknown as Document;
    const remove = adoptCss(".a { color: red }", fake);
    expect(adopted).toHaveLength(1);
    expect((adopted[0] as Sheet).css).toBe(".a { color: red }");
    remove();
    expect(adopted).toEqual([]);
  });
});
