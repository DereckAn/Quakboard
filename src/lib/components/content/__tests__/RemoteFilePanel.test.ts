import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

// Event handlers the component registered, so tests can fire events at it.
const handlers = new Map<string, (event: { payload: unknown }) => void>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(name, handler);
    return () => handlers.delete(name);
  }),
}));

vi.mock("$lib/tauri/commands", () => ({
  tauriSyncFetchFile: vi.fn().mockResolvedValue(undefined),
  tauriSyncCancelFetch: vi.fn().mockResolvedValue(undefined),
}));

import { tauriSyncCancelFetch, tauriSyncFetchFile } from "$lib/tauri/commands";
import RemoteFilePanel from "../RemoteFilePanel.svelte";

const remote = {
  offer_id: "offer-1",
  origin_device_id: "laptop-id",
  origin_name: "Laptop",
  sha256: "a".repeat(64),
  size: 2048,
  mime: "application/pdf",
};

function renderPanel() {
  return render(RemoteFilePanel, { itemId: "item-1", fileName: "report.pdf", remote });
}

async function fire(name: string, payload: unknown) {
  await waitFor(() => expect(handlers.has(name)).toBe(true));
  handlers.get(name)!({ payload });
}

afterEach(() => {
  cleanup();
  handlers.clear();
  vi.clearAllMocks();
});

describe("RemoteFilePanel", () => {
  it("says which device has the file", () => {
    const { getByText } = renderPanel();
    expect(getByText(/on Laptop/)).toBeTruthy();
  });

  it("fetches this item when Fetch is clicked", async () => {
    const { getByRole } = renderPanel();
    await fireEvent.click(getByRole("button", { name: /Fetch from Laptop/ }));

    expect(tauriSyncFetchFile).toHaveBeenCalledWith("item-1");
  });

  it("shows how far the fetch got", async () => {
    const { findByRole } = renderPanel();
    await fire("sync-file-progress", { itemId: "item-1", phase: "fetching", done: 1024, total: 2048 });

    expect((await findByRole("progressbar")).getAttribute("aria-valuenow")).toBe("50");
  });

  it("ignores progress for other items", async () => {
    const { queryByRole } = renderPanel();
    await fire("sync-file-progress", { itemId: "other", phase: "fetching", done: 1024, total: 2048 });

    expect(queryByRole("progressbar")).toBeNull();
  });

  it("cancels this item's fetch", async () => {
    const { findByRole } = renderPanel();
    await fire("sync-file-progress", { itemId: "item-1", phase: "fetching", done: 10, total: 2048 });
    await fireEvent.click(await findByRole("button", { name: "Cancel" }));

    expect(tauriSyncCancelFetch).toHaveBeenCalledWith("item-1");
  });

  it("explains a failed fetch", async () => {
    const { findByRole } = renderPanel();
    await fire("sync-file-done", {
      itemId: "item-1",
      ok: false,
      cancelled: false,
      error: "Laptop is offline or unreachable",
    });

    expect((await findByRole("alert")).textContent).toContain("Laptop is offline");
  });

  it("offers to try again after a failure", async () => {
    const { findByRole } = renderPanel();
    await fire("sync-file-done", { itemId: "item-1", ok: false, cancelled: false, error: "x" });

    expect(await findByRole("button", { name: /Try again/ })).toBeTruthy();
  });

  it("shows no error after the user cancels", async () => {
    const { queryByRole } = renderPanel();
    await fire("sync-file-done", { itemId: "item-1", ok: false, cancelled: true, error: null });

    expect(queryByRole("alert")).toBeNull();
  });
});
