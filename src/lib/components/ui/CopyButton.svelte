<script lang="ts">
  import Icon from "$lib/components/icons/Icon.svelte";
  import { cn } from "$lib/utils/cn";

  interface Props {
    onclick: () => void;
    copied: boolean;
    label: string;
    class?: string;
    disabled?: boolean;
  }

  let { onclick, copied, label, class: className, disabled = false }: Props =
    $props();
</script>

<button
  {onclick}
  {disabled}
  class={cn(
    "px-4 py-1 bg-gray-700 text-white rounded-lg text-sm hover:bg-gray-800 transition-colors flex items-center justify-center gap-2 disabled:opacity-60 disabled:cursor-default",
    className,
  )}
>
  <Icon name={copied ? "check" : "copy"} size={18} class="text-white" />
  <span>{copied ? "Copied!" : label}</span>
</button>
