// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import { ApiError, createHttp, type Http } from "../api/http";
import { ServerApisProvider } from "../data/server-apis";
import { body, image, p } from "../editor/content";
import { renderEditor } from "../editor/test-editor";
import { attachmentUrl, mediaUrl } from "../editor/urls";

import {
  createHttpAttachmentsApi,
  uploadProblem,
  type Attachment,
  type AttachmentsApi,
} from "./api";
import { PageFilesProvider } from "./context";
import { FilesPanel, sizeText } from "./files-panel";

const PAGE = "01M64W0000000000000000PAGE";
const file = (name: string, size = 2048): Attachment => ({
  page_id: PAGE,
  name,
  size,
  ref: `vellum://attachment/${PAGE}/${encodeURIComponent(name)}`,
  url: `/api/v1/attachments/${PAGE}/${encodeURIComponent(name)}`,
});

function fakeApi(overrides: Partial<AttachmentsApi> = {}): AttachmentsApi {
  return {
    list: vi.fn(() =>
      Promise.resolve([file("brief.pdf", 3 * 1024 ** 2), file("chart.png")]),
    ),
    upload: vi.fn((_page: string, picked: File) =>
      Promise.resolve({ ...file(picked.name), media_type: "image/png" }),
    ),
    remove: vi.fn(() => Promise.resolve()),
    ...overrides,
  };
}

function providers(api: AttachmentsApi | undefined) {
  return function Providers({ children }: { readonly children: ReactNode }) {
    return (
      <ServerApisProvider apis={{ attachments: api }}>
        <PageFilesProvider pageId={PAGE}>{children}</PageFilesProvider>
      </ServerApisProvider>
    );
  };
}

const png = () =>
  new File([new Uint8Array([137, 80, 78, 71])], "Team photo.png", {
    type: "image/png",
  });
const picker = () => {
  const input = document.querySelector<HTMLInputElement>('input[type="file"]');
  if (input === null) throw new Error("no file input");
  return input;
};

describe("attachment links", () => {
  it("maps an attachment ref to its download path and nothing else", () => {
    expect(attachmentUrl(`vellum://attachment/${PAGE}/a%20b.png`)).toBe(
      `/api/v1/attachments/${PAGE}/a%20b.png`,
    );
    expect(attachmentUrl(`vellum://attachment/${PAGE}/../x`)).toBeUndefined();
    expect(attachmentUrl("vellum://attachment/short/x.png")).toBeUndefined();
    expect(attachmentUrl("https://example.com/x.png")).toBeUndefined();
    expect(mediaUrl("https://example.com/x.png")).toBe(
      "https://example.com/x.png",
    );
    expect(mediaUrl("javascript:alert(1)")).toBeUndefined();
  });

  it("puts uploads in the editor's image block and shows them", async () => {
    const api = fakeApi();
    const user = userEvent.setup();
    const { editor } = await renderEditor(body(p("intro"), image("", "")), {
      wrapper: providers(api),
    });
    expect(
      screen.getByRole("button", { name: "Upload an image" }),
    ).toBeDefined();
    expect(picker().accept).toBe("image/*");
    await user.upload(picker(), png());
    expect(api.upload).toHaveBeenCalledWith(PAGE, expect.any(File));
    const shown = await screen.findByRole("img", { name: "Team photo" });
    expect(shown.getAttribute("src")).toBe(
      `/api/v1/attachments/${PAGE}/Team%20photo.png`,
    );
    expect(editor.getJSON().content[1]?.attrs).toMatchObject({
      src: `vellum://attachment/${PAGE}/Team%20photo.png`,
      alt: "Team photo",
    });
  });

  it("says in plain words why an upload was refused", async () => {
    const api = fakeApi({
      upload: vi.fn(() =>
        Promise.reject(
          new ApiError("unavailable", "the virus scanner is unavailable", {
            status: 503,
          }),
        ),
      ),
    });
    const user = userEvent.setup();
    await renderEditor(body(p("intro"), image("", "")), {
      wrapper: providers(api),
    });
    await user.upload(picker(), png());
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("uploads are paused");
  });

  it("offers no upload without a server", async () => {
    await renderEditor(body(p("intro"), image("", "")), {
      wrapper: providers(undefined as AttachmentsApi | undefined),
    });
    expect(
      screen.queryByRole("button", { name: "Upload an image" }),
    ).toBeNull();
  });

  it("names each refusal", () => {
    const failed = (status: number) =>
      uploadProblem(new ApiError("invalid_input", "x", { status }));
    expect(failed(413).key).toBe("upload-too-large");
    expect(failed(415).key).toBe("upload-wrong-type");
    expect(failed(422).key).toBe("upload-refused");
    expect(failed(500).key).toBe("upload-failed");
    expect(uploadProblem(new Error("offline")).detail).toBe("offline");
  });
});

describe("the files panel", () => {
  function renderPanel(api: AttachmentsApi, canWrite = true) {
    const Providers = providers(api);
    const onClose = vi.fn();
    render(
      <Providers>
        <FilesPanel canWrite={canWrite} onClose={onClose} />
      </Providers>,
    );
    return { user: userEvent.setup(), onClose };
  }

  it("lists files with downloads and sizes", async () => {
    renderPanel(fakeApi(), false);
    const pdf = await screen.findByRole("link", { name: "brief.pdf" });
    expect(pdf.getAttribute("href")).toBe(
      `/api/v1/attachments/${PAGE}/brief.pdf`,
    );
    expect(pdf.hasAttribute("download")).toBe(true);
    expect(screen.getByText("3 MB")).toBeDefined();
    expect(screen.queryByRole("button", { name: /Delete/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Upload a file" })).toBeNull();
  });

  it("deletes a file only after a second click, then reloads", async () => {
    const api = fakeApi();
    const { user } = renderPanel(api);
    await user.click(
      await screen.findByRole("button", { name: "Delete chart.png" }),
    );
    expect(api.remove).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Delete chart.png for good" }),
    );
    expect(api.remove).toHaveBeenCalledWith(
      `/api/v1/attachments/${PAGE}/chart.png`,
    );
    expect(await screen.findByText("Deleted chart.png.")).toBeDefined();
    expect(api.list).toHaveBeenCalledTimes(2);
  });

  it("uploads any file and lists it", async () => {
    const api = fakeApi();
    const { user } = renderPanel(api);
    await user.upload(picker(), png());
    expect(await screen.findByText("Uploaded Team photo.png.")).toBeDefined();
    expect(api.list).toHaveBeenCalledTimes(2);
  });

  it("closes", async () => {
    const { user, onClose } = renderPanel(fakeApi());
    const panel = screen.getByRole("complementary", {
      name: "Files on this page",
    });
    await user.click(
      within(panel).getByRole("button", { name: "Close files" }),
    );
    expect(onClose).toHaveBeenCalled();
  });

  it("formats sizes", () => {
    expect(sizeText(512)).toBe("512 B");
    expect(sizeText(1536)).toBe("1.5 KB");
  });
});

describe("the attachments API", () => {
  it(
    "uploads the raw file under its name and deletes only attachment " +
      "paths",
    async () => {
      const upload = vi.fn(() => Promise.resolve(file("a b.png")));
      const send = vi.fn(() => Promise.resolve());
      const get = vi.fn(() =>
        Promise.resolve({ items: [file("z.txt"), file("a.txt")] }),
      );
      const api = createHttpAttachmentsApi({
        get,
        send,
        upload,
        forgetSession: vi.fn(),
      } as unknown as Http);
      const picked = new File(["x"], "a b.png");
      await api.upload(PAGE, picked);
      expect(upload).toHaveBeenCalledWith(
        `/api/v1/attachments/${PAGE}`,
        { name: "a b.png" },
        picked,
        "application/octet-stream",
      );
      const listed = await api.list(PAGE);
      expect(listed.map((found) => found.name)).toEqual(["a.txt", "z.txt"]);
      await api.remove(`/api/v1/attachments/${PAGE}/a.txt`);
      expect(send).toHaveBeenCalledWith(
        "DELETE",
        `/api/v1/attachments/${PAGE}/a.txt`,
      );
      await expect(api.remove("/api/v1/drive/items/x")).rejects.toThrow(
        "not an attachment",
      );
    },
  );

  it(
    "sends an upload's bytes with the CSRF token and its media " + "type",
    async () => {
      const calls: { url: string; init: RequestInit }[] = [];
      const fetch = vi.fn(
        (input: RequestInfo | URL, init: RequestInit = {}) => {
          const url = input instanceof Request ? input.url : input.toString();
          calls.push({ url, init });
          return Promise.resolve(
            url === "/api/csrf"
              ? Response.json({ token: "t0k" })
              : Response.json(file("x.png"), { status: 201 }),
          );
        },
      );
      const http = createHttp({
        fetch: fetch as unknown as typeof globalThis.fetch,
        idempotencyKey: () => "k",
      });
      const body = png();
      await http.upload(
        "/api/v1/attachments/P",
        { name: "x y.png" },
        body,
        "image/png",
      );
      const sent = calls.find((call) => call.init.method === "POST");
      const headers = sent?.init.headers as Record<string, string>;
      expect(sent?.url).toBe("/api/v1/attachments/P?name=x+y.png");
      expect(sent?.init.body).toBe(body);
      expect(headers["x-csrf-token"]).toBe("t0k");
      expect(headers["content-type"]).toBe("image/png");
      expect(headers["idempotency-key"]).toBe("k");
    },
  );
});
