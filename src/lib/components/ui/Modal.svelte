<script lang="ts">
  import { cn } from "$lib/utils/cn";

  interface Props {
    isOpen: boolean;
    onClose: () => void;
    title?: string;
    class?: string;
    children?: import("svelte").Snippet;
  }

  let { isOpen, onClose, title, class: className, children }: Props = $props();

  // Handle ESC key
  $effect(() => {
    if (!isOpen) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Capture phase + stop: Esc closes only the modal, not the page
        // shortcuts listening on window (e.g. settings' Esc-to-go-back).
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };

    window.addEventListener("keydown", handleKeyDown, { capture: true });
    return () =>
      window.removeEventListener("keydown", handleKeyDown, { capture: true });
  });

  const handleBackdropClick = (e: MouseEvent) => {
    if (e.target === e.currentTarget) {
      onClose();
    }
  };

  const handleBackdropKeyDown = (e: KeyboardEvent) => {
    // Keys typed in the modal's own inputs bubble up here; only the
    // backdrop itself should close on Enter/Space.
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onClose();
    }
  };
</script>

{#if isOpen}
  <!-- Backdrop -->
  <div
    class="fixed inset-0 z-50 bg-black/80 flex items-center justify-center p-4"
    onclick={handleBackdropClick}
    onkeydown={handleBackdropKeyDown}
    role="button"
    tabindex="-1"
    aria-label="Close modal"
  >
    <!-- Modal content -->
    <div
      class={cn(
        "relative bg-surface rounded-lg shadow-lg max-w-lg w-full max-h-[85vh] overflow-y-auto",
        className
      )}
    >
      {#if title}
        <div
          class="flex items-center justify-between p-6 border-b border-border"
        >
          <h2 class="text-lg font-semibold">{title}</h2>
          <button
            onclick={onClose}
            aria-label="Close modal"
            class="rounded-sm opacity-70 hover:opacity-100 transition-opacity"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="24"
              height="24"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M18 6 6 18" />
              <path d="m6 6 12 12" />
            </svg>
          </button>
        </div>
      {/if}

      <div class="p-6">
        {@render children?.()}
      </div>
    </div>
  </div>
{/if}
