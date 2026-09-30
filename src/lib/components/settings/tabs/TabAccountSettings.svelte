<script>
  import Icon from "$lib/components/icons/Icon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Input from "$lib/components/ui/Input.svelte";
  import DevicesSection from "../DevicesSection.svelte";
</script>

<section class="bg-surface rounded-2xl border border-border/60 p-3 relative">
  <h2 class="text-lg font-semibold mb-4 flex items-center gap-2">
    <Icon name="user" size={20} />
    Account
  </h2>
  <div class="space-y-4 z-10">
    <div>
      <label
        for="email-input"
        class="block text-sm font-medium text-text-muted mb-2"
      >
        Email
      </label>
      <Input type="email" placeholder="Not signed in" value="" class="" />
      <p class="text-xs text-text-muted mt-1">
        Sign in to sync your clipboard across devices
      </p>
    </div>

    <div class="flex items-center justify-between">
      <div>
        <p class="block text-sm font-medium text-text-muted mb-1">
          Subscription
        </p>
        <p class="text-xs text-text-muted">Free Tier</p>
      </div>
      <Button variant="default" size="other">Upgrade to Pro</Button>
    </div>
  </div>
</section>

<DevicesSection />
