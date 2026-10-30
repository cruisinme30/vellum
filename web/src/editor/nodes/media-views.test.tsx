// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { body, embed, image, linkCard, p } from "../content";
import { renderEditor } from "../test-editor";

/** A paragraph first, so the block under test is not selected on load. */
const page = (block: ReturnType<typeof embed>) => body(p("intro"), block);

describe("LinkCardView", () => {
  it("links a safe address in a new tab, titled and with its host", async () => {
    await renderEditor(
      page(linkCard("https://docs.example.com/guide", "The guide")),
    );
    const link = screen.getByRole("link", { name: /The guide/ });
    expect(link.getAttribute("href")).toBe("https://docs.example.com/guide");
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    expect(link.textContent).toContain("docs.example.com");
    expect(screen.queryByRole("textbox", { name: "Address" })).toBeNull();
  });

  it("falls back to the address when it has no title", async () => {
    await renderEditor(page(linkCard("/p/123", "")));
    expect(screen.getByRole("link").textContent).toContain("/p/123");
  });

  it("refuses an unsafe address and asks for one", async () => {
    await renderEditor(page(linkCard("javascript:alert(1)", "x")));
    expect(screen.queryByRole("link")).toBeNull();
    expect(
      screen.getByText("Add the address of the page to link."),
    ).toBeDefined();
  });

  it("shows its form while empty and saves both fields", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(page(linkCard("", "")));
    await user.type(screen.getByRole("textbox", { name: "Title" }), "Docs");
    await user.type(
      screen.getByRole("textbox", { name: "Address" }),
      "https://a.example",
    );
    expect(editor.getJSON().content[1]?.attrs).toMatchObject({
      url: "https://a.example",
      title: "Docs",
    });
  });

  it("shows its form while selected", async () => {
    const { editor } = await renderEditor(
      page(linkCard("https://a.example", "A")),
    );
    expect(screen.queryByRole("textbox", { name: "Address" })).toBeNull();
    act(() => {
      editor.commands.setNodeSelection(7);
    });
    const address = await screen.findByRole("textbox", { name: "Address" });
    expect((address as HTMLInputElement).value).toBe("https://a.example");
  });

  it("hides its form in a read-only editor", async () => {
    await renderEditor(page(linkCard("", "")), { editable: false });
    expect(screen.queryByRole("textbox", { name: "Address" })).toBeNull();
  });
});

describe("EmbedView", () => {
  it("frames a supported provider in a sandbox", async () => {
    await renderEditor(
      page(embed("https://www.youtube.com/watch?v=dQw4w9WgXcQ")),
    );
    const frame = screen.getByTitle("YouTube embed");
    expect(frame.tagName).toBe("IFRAME");
    expect(frame.getAttribute("src")).toBe(
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
    );
    expect(frame.getAttribute("sandbox")).toBe(
      "allow-scripts allow-same-origin allow-popups",
    );
    expect(frame.getAttribute("referrerpolicy")).toBe(
      "strict-origin-when-cross-origin",
    );
  });

  it("links an unsupported site instead of framing it", async () => {
    await renderEditor(page(embed("https://example.com/video")));
    expect(document.querySelector("iframe")).toBeNull();
    expect(screen.getByRole("link").getAttribute("href")).toBe(
      "https://example.com/video",
    );
    expect(screen.getByRole("link").textContent).toBe(
      "Open https://example.com/video (this site cannot be embedded)",
    );
  });

  it("refuses an unsafe address", async () => {
    await renderEditor(page(embed("javascript:alert(1)")));
    expect(document.querySelector("iframe")).toBeNull();
    expect(screen.queryByRole("link")).toBeNull();
    expect(
      screen.getByText("Add the address of a video, design or map."),
    ).toBeDefined();
  });

  it("saves the address typed in its form", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(page(embed("")));
    await user.type(
      screen.getByRole("textbox", { name: "Address" }),
      "https://youtu.be/abcdefg",
    );
    expect(editor.getJSON().content[1]?.attrs?.["url"]).toBe(
      "https://youtu.be/abcdefg",
    );
    expect(screen.getByTitle("YouTube embed")).toBeDefined();
    act(() => {
      screen.getByRole("textbox", { name: "Address" }).blur();
    });
    expect(screen.queryByRole("textbox", { name: "Address" })).toBeNull();
  });
});

describe("ImageView", () => {
  const sized = {
    type: "image",
    attrs: {
      src: "/assets/chart.png",
      alt: "A chart",
      title: "Weekly",
      width: "50%",
      align: "left",
    },
  };

  it("shows a safe image with its alt text, width, caption and alignment", async () => {
    await renderEditor(page(sized));
    const picture = screen.getByRole("img", { name: "A chart" });
    expect(picture.getAttribute("src")).toBe("/assets/chart.png");
    expect(picture.style.width).toBe("50%");
    expect(screen.getByText("Weekly").tagName).toBe("FIGCAPTION");
    expect(picture.closest<HTMLElement>("[data-align]")?.dataset["align"]).toBe(
      "left",
    );
  });

  it("leaves the width to CSS and drops the caption when they are empty", async () => {
    await renderEditor(page(image("https://example.com/a.png", "A")));
    const picture = screen.getByRole("img", { name: "A" });
    expect(picture.getAttribute("style")).toBeNull();
    expect(document.querySelector("figcaption")).toBeNull();
  });

  it("refuses an unsafe source", async () => {
    await renderEditor(page(image("data:image/png;base64,AAAA", "x")));
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.getByText("Add an image address.")).toBeDefined();
  });

  it("saves the source and alt text typed in its form", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(page(image("", "")));
    await user.type(
      screen.getByRole("textbox", { name: "Alternative text" }),
      "Logo",
    );
    await user.type(
      screen.getByRole("textbox", { name: "Image address" }),
      "/logo.svg",
    );
    expect(editor.getJSON().content[1]?.attrs).toMatchObject({
      src: "/logo.svg",
      alt: "Logo",
    });
  });
});
