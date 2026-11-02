<script lang="ts">
  import Icon from "$lib/components/icons/Icon.svelte";
  import { cn } from "$lib/utils/cn";

  interface Props {
    // Local midnight of the picked day, or null for no day filter.
    value: Date | null;
    onSelect: (day: Date | null) => void;
  }

  let { value, onSelect }: Props = $props();

  const WEEKDAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

  let isOpen = $state(false);
  // First day of the month on screen.
  let shownMonth = $state(startOfMonth(value ?? new Date()));

  const monthLabel = $derived(
    shownMonth.toLocaleDateString(undefined, { month: "long", year: "numeric" }),
  );

  // Whole weeks, Sunday first, padded with the neighbouring months' days.
  const days = $derived.by(() => {
    const first = new Date(shownMonth);
    first.setDate(1 - first.getDay());
    const last = new Date(shownMonth.getFullYear(), shownMonth.getMonth() + 1, 0);
    last.setDate(last.getDate() + (6 - last.getDay()));

    const result: Date[] = [];
    for (const day = first; day <= last; day.setDate(day.getDate() + 1)) {
      result.push(new Date(day));
    }
    return result;
  });

  const triggerLabel = $derived(
    value
      ? value.toLocaleDateString(undefined, { month: "short", day: "numeric" })
      : null,
  );

  function startOfMonth(date: Date) {
    return new Date(date.getFullYear(), date.getMonth(), 1);
  }

  function isSameDay(a: Date | null, b: Date) {
    return (
      a !== null &&
      a.getFullYear() === b.getFullYear() &&
      a.getMonth() === b.getMonth() &&
      a.getDate() === b.getDate()
    );
  }

  function shiftMonth(delta: number) {
    shownMonth = new Date(
      shownMonth.getFullYear(),
      shownMonth.getMonth() + delta,
      1,
    );
  }

  function toggle() {
    if (!isOpen) shownMonth = startOfMonth(value ?? new Date());
    isOpen = !isOpen;
  }

  // Picking the selected day again clears the filter.
  function handlePick(day: Date) {
    onSelect(isSameDay(value, day) ? null : day);
    isOpen = false;
  }

  $effect(() => {
    if (!isOpen) return;

    const handleClickOutside = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest("[data-date-picker]")) {
        isOpen = false;
      }
    };

    document.addEventListener("click", handleClickOutside);
    return () => document.removeEventListener("click", handleClickOutside);
  });
</script>

<div class="relative bg-surface rounded-md" data-date-picker>
  <button
    onclick={toggle}
    aria-label={triggerLabel ? `Showing ${triggerLabel}; choose a day` : "Choose a day"}
    aria-expanded={isOpen}
    class={cn(
      "flex items-center gap-2 h-7 px-2 rounded-md border border-border hover:bg-surface-hover transition-colors text-sm",
      value && "border-primary/60 text-primary",
    )}
  >
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      aria-hidden="true"
      ><g
        fill="none"
        stroke="currentColor"
        stroke-linecap="round"
        stroke-linejoin="round"
        stroke-width="2"
        ><rect width="20" height="18" x="2" y="4" rx="4" /><path
          d="M8 2v4m8-4v4M2 10h20"
        /></g
      ></svg
    >
    {#if triggerLabel}
      <span class="whitespace-nowrap">{triggerLabel}</span>
    {/if}
  </button>

  {#if isOpen}
    <div
      class="absolute top-full mt-2 right-0 z-50 w-max p-2.5 rounded-xl border border-border bg-surface backdrop-blur-sm shadow-lg"
    >
      <div class="flex items-center justify-between mb-1">
        <button
          onclick={() => shiftMonth(-1)}
          aria-label="Previous month"
          class="size-7 flex items-center justify-center rounded-md hover:bg-white/10 transition-colors"
        >
          <Icon name="chevronRight" size={16} class="rotate-180" />
        </button>
        <span class="text-sm font-medium">{monthLabel}</span>
        <button
          onclick={() => shiftMonth(1)}
          aria-label="Next month"
          class="size-7 flex items-center justify-center rounded-md hover:bg-white/10 transition-colors"
        >
          <Icon name="chevronRight" size={16} />
        </button>
      </div>

      <div class="grid grid-cols-7 gap-x-1 gap-y-0.5 text-center">
        {#each WEEKDAYS as weekday}
          <span class="size-9 flex items-center justify-center text-xs text-text-muted">
            {weekday}
          </span>
        {/each}
        {#each days as day (day.getTime())}
          {@const isSelected = isSameDay(value, day)}
          <button
            onclick={() => handlePick(day)}
            aria-label={day.toLocaleDateString(undefined, { dateStyle: "full" })}
            aria-pressed={isSelected}
            class={cn(
              "size-9 flex items-center justify-center rounded-md text-sm transition-colors hover:bg-white/10",
              day.getMonth() !== shownMonth.getMonth() && "text-text-muted/60",
              isSameDay(new Date(), day) && !isSelected && "bg-white/10",
              isSelected && "bg-text text-background hover:bg-text",
            )}
          >
            {day.getDate()}
          </button>
        {/each}
      </div>
    </div>
  {/if}
</div>
