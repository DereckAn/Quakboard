import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/tauri/commands", () => ({}));
vi.mock("$lib/stores/clipboard.svelte", () => ({ clipboardStore: {} }));
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn() }));

import type { ClipboardItem } from "$lib/types";
import ContentViewer from "../ContentViewer.svelte";

function image(metadata: Record<string, unknown>): ClipboardItem {
  return {
    id: "item-1",
    contentType: "image",
    contentText: "photo.png",
    contentMetadata: metadata,
    sourceApp: null,
    codeLanguage: null,
    fileUrl: "/home/me/Pictures/photo.png",
    fileName: "photo.png",
    fileSizeBytes: 2048,
    fileMimeType: "image/png",
    isFavorite: false,
    isSnippet: false,
    snippetName: null,
    createdAt: new Date(),
    updatedAt: new Date(),
  } as ClipboardItem;
}

afterEach(cleanup);

describe("ContentViewer", () => {
  it("offers Send for an image file copied in a file manager", () => {
    const item = image({ source: "file", external_path: "/home/me/Pictures/photo.png" });
    const { queryByRole } = render(ContentViewer, { item });

    expect(queryByRole("button", { name: /Send/ })).not.toBeNull();
  });

  it("gives an image file one copy button, not image and file copies", () => {
    const item = image({ source: "file", external_path: "/home/me/Pictures/photo.png" });
    const { queryAllByRole } = render(ContentViewer, { item });

    expect(queryAllByRole("button", { name: /Copy/ })).toHaveLength(1);
  });

  it("doesn't offer Send for a copied image, which syncs on its own", () => {
    const { queryByRole } = render(ContentViewer, { item: image({ source: "clipboard" }) });

    expect(queryByRole("button", { name: /Send/ })).toBeNull();
  });
});
