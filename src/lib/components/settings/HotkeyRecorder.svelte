<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  interface Props {
    value?: string;
    id?: string;
    onChange?: (hotkey: string) => void;
    disabled?: boolean;
  }

  let { value = "", onChange, disabled = false }: Props = $props();
  let isRecording = $state(false);
  let recordedKeys = $state<string[]>([]);
  // Modifiers currently held, tracked from their own key events. Wayland/Hyprland
  // doesn't keep e.metaKey set on later keydowns (Super is the compositor's
  // modifier), so flags alone make Super vanish — we track the keys directly.
  let heldMods = $state<string[]>([]);
  let currentOS = $state<string>("macos");

  // Símbolos según el sistema operativo
  const SYMBOLS_BY_OS: Record<string, Record<string, string>> = {
    macos: {
      Command: "⌘",
      Control: "⌃",
      Option: "⌥",
      Shift: "⇧",
    },
    windows: {
      Command: "Win",
      Control: "Ctrl",
      Alt: "Alt",
      Shift: "⇧",
    },
    linux: {
      Command: "Super",
      Control: "Ctrl",
      Alt: "Alt",
      Shift: "⇧",
    },
  };

  const MODIFIER_ORDER = ["Command", "Control", "Alt", "Option", "Shift"];

  onMount(async () => {
    try {
      const platform = await invoke<string>("get_platform");
      currentOS = platform;
      console.log("Platform detected:", platform);
    } catch (err) {
      console.error("Failed to detect platform:", err);
      currentOS = "macos"; // fallback
    }
  });

  const getSymbols = () => {
    return SYMBOLS_BY_OS[currentOS] || SYMBOLS_BY_OS.macos;
  };

  const formatHotkey = (hotkey: string, keys: string[]) => {
    const parts = keys.length
      ? keys
      : hotkey
          .split("+")
          .map((k) => k.trim())
          .filter(Boolean);

    if (!parts.length) return "Click to record hotkey";

    const symbols = getSymbols();
    return parts.map((part) => symbols[part] ?? part.toUpperCase()).join(" ");
  };

  const isModifier = (key: string) =>
    key === "Command" ||
    key === "Control" ||
    key === "Alt" ||
    key === "Option" ||
    key === "Shift";

  // Map a modifier KEY (not flag) to our canonical name. We match on both e.key
  // and e.code because the spelling varies by platform/engine — WebKitGTK reports
  // Super as key="Super" code="OSLeft", others use "Meta"/"MetaLeft".
  const modName = (e: KeyboardEvent): string | null => {
    const k = e.key;
    const c = e.code;
    if (
      k === "Meta" || k === "Super" || k === "OS" ||
      c === "MetaLeft" || c === "MetaRight" ||
      c === "OSLeft" || c === "OSRight" ||
      c === "SuperLeft" || c === "SuperRight"
    )
      return "Command";
    if (k === "Control" || c === "ControlLeft" || c === "ControlRight")
      return "Control";
    if (k === "Alt" || c === "AltLeft" || c === "AltRight") return "Alt";
    if (k === "Shift" || c === "ShiftLeft" || c === "ShiftRight") return "Shift";
    return null;
  };

  const orderedMods = () => MODIFIER_ORDER.filter((m) => heldMods.includes(m));

  const displayText = $derived.by(() =>
    isRecording
      ? recordedKeys.length > 0
        ? formatHotkey("", recordedKeys)
        : "Press keys…"
      : formatHotkey(value, [])
  );

  const startRecording = () => {
    if (disabled) return;
    isRecording = true;
    recordedKeys = [];
    heldMods = [];
  };

  const cancelRecording = () => {
    isRecording = false;
    recordedKeys = [];
    heldMods = [];
  };

  const finishRecording = () => {
    isRecording = false;
    const hasMain = recordedKeys.some((k) => !isModifier(k));
    const hasModifierKey = recordedKeys.some(isModifier);

    if (recordedKeys.length > 0 && hasMain && hasModifierKey && onChange) {
      const hotkey = recordedKeys.join("+");
      onChange(hotkey);
    }
    heldMods = [];
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (!isRecording || disabled) return;

    e.preventDefault();
    e.stopPropagation();

    if (e.key === "Escape") {
      cancelRecording();
      return;
    }

    const mod = modName(e);
    if (mod) {
      // A modifier key: remember it's held. Don't trust the flags — track the key.
      if (!heldMods.includes(mod)) heldMods = [...heldMods, mod];
      recordedKeys = orderedMods();
      return;
    }

    // A non-modifier key completes the combo with whatever modifiers are held.
    recordedKeys = [...orderedMods(), e.key.toUpperCase()];
  };

  const handleKeyUp = (e: KeyboardEvent) => {
    if (!isRecording || disabled) return;

    // Lock in a complete combo before the released modifier is dropped.
    const hasMain = recordedKeys.some((k) => !isModifier(k));
    const hasModifierKey = recordedKeys.some(isModifier);
    if (recordedKeys.length > 0 && hasMain && hasModifierKey) {
      finishRecording();
      return;
    }

    const mod = modName(e);
    if (mod) heldMods = heldMods.filter((m) => m !== mod);
  };

  const handleBlur = () => {
    if (isRecording) {
      finishRecording();
    }
  };
</script>

<!-- Capture phase: while recording, keys (incl. Esc) must not reach the
     page-level shortcuts, which listen on window in the bubble phase. -->
<svelte:window onkeydowncapture={handleKeyDown} onkeyup={handleKeyUp} />

<div class="flex items-center gap-2">
  <span
    class="px-3 py-1 rounded-md border border-border/60 text-sm text-no-wrap min-w-[100px] text-center"
  >
    {displayText}
  </span>
  <button
    class={`px-3 py-1 rounded-md bg-white/10 text-sm ${
      isRecording
        ? "border-primary text-primary bg-primary/10 animate-pulse"
        : "border-border/60 text-text bg-surface-100/40 hover:bg-white/20"
    }`}
    onclick={startRecording}
    onblur={handleBlur}
    id="edit-hotkey-button"
    {disabled}
  >
    {#if isRecording}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="1.1em"
        height="1.1em"
        viewBox="0 0 24 24"
        role="img"
        aria-label="Recording"
      >
        <g fill="none">
          <circle
            cx="12"
            cy="12"
            r="9.25"
            stroke="currentColor"
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="1.5"
          />
          <circle cx="12" cy="12" r="5" fill="currentColor" />
        </g>
      </svg>
    {:else}
      Edit
    {/if}
  </button>
</div>

<style>
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.5;
    }
  }
  .animate-pulse {
    animation: pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
</style>
