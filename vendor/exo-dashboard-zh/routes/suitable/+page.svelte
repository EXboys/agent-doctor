<script lang="ts">
  /**
   * Agent Doctor overlay: models this cluster can run now (or with more free memory).
   */
  import { onMount } from "svelte";
  import HeaderNav from "$lib/components/HeaderNav.svelte";
  import { topologyData, refreshState } from "$lib/stores/app.svelte";

  type ModelInfo = {
    id: string;
    name?: string;
    storage_size_megabytes?: number;
    family?: string;
    quantization?: string;
  };

  type FitKind = "fits_now" | "fits_later";

  let models = $state<ModelInfo[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);

  const topology = $derived(topologyData());

  function availableMemoryGB(): number {
    const nodes = topology?.nodes;
    if (!nodes) return 0;
    return (
      Object.values(nodes).reduce((acc, n) => {
        const total =
          n.macmon_info?.memory?.ram_total ?? n.system_info?.memory ?? 0;
        const used = n.macmon_info?.memory?.ram_usage ?? 0;
        return acc + (total - used);
      }, 0) /
      (1024 * 1024 * 1024)
    );
  }

  function clusterTotalMemoryGB(): number {
    const nodes = topology?.nodes;
    if (!nodes) return 0;
    return (
      Object.values(nodes).reduce((acc, n) => {
        const total =
          n.macmon_info?.memory?.ram_total ?? n.system_info?.memory ?? 0;
        return acc + total;
      }, 0) /
      (1024 * 1024 * 1024)
    );
  }

  function nodeCount(): number {
    const nodes = topology?.nodes;
    return nodes ? Object.keys(nodes).length : 0;
  }

  function modelSizeGB(model: ModelInfo): number {
    return (model.storage_size_megabytes || 0) / 1024;
  }

  function fitKind(model: ModelInfo): FitKind | null {
    const size = modelSizeGB(model);
    if (size <= 0) return null;
    if (size <= availableMemoryGB()) return "fits_now";
    if (size <= clusterTotalMemoryGB()) return "fits_later";
    return null;
  }

  function sortBySizeDesc(a: ModelInfo, b: ModelInfo): number {
    return modelSizeGB(b) - modelSizeGB(a);
  }

  const fitsNow = $derived(
    models.filter((m) => fitKind(m) === "fits_now").sort(sortBySizeDesc),
  );
  const fitsLater = $derived(
    models.filter((m) => fitKind(m) === "fits_later").sort(sortBySizeDesc),
  );

  const memoryRatio = $derived(() => {
    const total = clusterTotalMemoryGB();
    if (total <= 0) return 0;
    const free = availableMemoryGB();
    return Math.max(0, Math.min(100, Math.round((free / total) * 100)));
  });

  const freeSummary = $derived(() => {
    const free = availableMemoryGB();
    const total = clusterTotalMemoryGB();
    const count = nodeCount();
    if (total <= 0) return "Reading memory…";
    const macLabel = count > 1 ? `${count} 台电脑合计` : "当前电脑";
    return `${macLabel} · 空闲 ${free.toFixed(1)} GB / 共 ${total.toFixed(1)} GB`;
  });

  async function loadModels(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      await refreshState();
      const response = await fetch("/models");
      if (!response.ok) {
        throw new Error(`models ${response.status}`);
      }
      const payload = await response.json();
      const list = Array.isArray(payload?.data) ? payload.data : [];
      models = list as ModelInfo[];
    } catch (err) {
      console.error(err);
      loadError = "Could not load the model list. Try again in a moment.";
    } finally {
      loading = false;
    }
  }

  function displayName(model: ModelInfo): string {
    return model.name?.trim() || model.id;
  }

  function sizeLabel(model: ModelInfo): string {
    const gb = modelSizeGB(model);
    if (gb <= 0) return "—";
    if (gb < 1) return `${Math.round(gb * 1024)} MB`;
    return `${gb.toFixed(1)} GB`;
  }

  onMount(() => {
    void loadModels();
  });
</script>

<div class="ad-page-shell">
  <HeaderNav showHome={true} />

  <main class="ad-page-body">
    <section class="ad-hero-card">
      <div class="ad-hero-top">
        <div>
          <h1 class="ad-hero-title">Suitable models</h1>
          <p class="ad-hero-sub">
            Only models this setup can run. Larger ones stay hidden here — open Downloads to manage files already on disk.
          </p>
        </div>
        <button type="button" class="ad-btn-pill" onclick={() => loadModels()}>
          Refresh
        </button>
      </div>

      <div class="ad-meter-box">
        <div class="ad-meter-head">
          <span>{freeSummary()}</span>
          <strong>{fitsNow.length} 个可直接跑</strong>
        </div>
        <div class="ad-meter-track" aria-hidden="true">
          <div class="ad-meter-fill" style="width: {memoryRatio()}%"></div>
        </div>
      </div>

      <p class="ad-callout">
        To turn a model on for chat, go back to Agent Doctor and pick a size under Shared Compute. Use this page to see what fits; use Downloads to watch progress.
      </p>
    </section>

    {#if loading}
      <div class="ad-empty-card">Loading…</div>
    {:else if loadError}
      <div class="ad-empty-card is-error">{loadError}</div>
    {:else}
      <section class="ad-section-card">
        <div class="ad-section-head">
          <h2>Fits now</h2>
          <span class="ad-badge is-ok">{fitsNow.length}</span>
        </div>

        {#if fitsNow.length === 0}
          <p class="ad-empty-inline">
            Nothing fits in free memory right now. Free some apps, or connect another Mac and refresh.
          </p>
        {:else}
          <ul class="ad-model-list">
            {#each fitsNow as model (model.id)}
              <li class="ad-model-row is-ready">
                <div class="ad-model-main">
                  <strong title={model.id}>{displayName(model)}</strong>
                  <span>{model.id}</span>
                </div>
                <span class="ad-size-pill is-ready">{sizeLabel(model)}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      {#if fitsLater.length > 0}
        <section class="ad-section-card">
          <div class="ad-section-head">
            <div>
              <h2>Needs more free memory</h2>
              <p class="ad-section-note">
                These fit the total memory of your Macs, but not the free memory right now. Quit heavy apps or add another Mac.
              </p>
            </div>
            <span class="ad-badge">{fitsLater.length}</span>
          </div>

          <ul class="ad-model-list">
            {#each fitsLater as model (model.id)}
              <li class="ad-model-row">
                <div class="ad-model-main">
                  <strong title={model.id}>{displayName(model)}</strong>
                  <span>{model.id}</span>
                </div>
                <span class="ad-size-pill">{sizeLabel(model)}</span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/if}

    <div class="ad-foot-link">
      <a href="/#/downloads">Open Downloads →</a>
    </div>
  </main>
</div>
