<script lang="ts">
  import { browser } from "$app/environment";

  interface Props {
    showHome?: boolean;
    onHome?: (() => void) | null;
    showSidebarToggle?: boolean;
    sidebarVisible?: boolean;
    onToggleSidebar?: (() => void) | null;
    showMobileMenuToggle?: boolean;
    mobileMenuOpen?: boolean;
    onToggleMobileMenu?: (() => void) | null;
    showMobileRightToggle?: boolean;
    mobileRightOpen?: boolean;
    onToggleMobileRight?: (() => void) | null;
    downloadProgress?: {
      count: number;
      percentage: number;
    } | null;
  }

  let {
    showHome = true,
    onHome = null,
    downloadProgress = null,
  }: Props = $props();

  let currentHash = $state(browser ? window.location.hash || "#/" : "#/");

  $effect(() => {
    if (!browser) return;
    const sync = () => {
      currentHash = window.location.hash || "#/";
    };
    sync();
    window.addEventListener("hashchange", sync);
    return () => window.removeEventListener("hashchange", sync);
  });

  function isActive(target: "suitable" | "downloads" | "home"): boolean {
    const h = currentHash.replace(/^#/, "") || "/";
    if (target === "suitable") return h.startsWith("/suitable");
    if (target === "downloads") return h.startsWith("/downloads");
    return h === "/" || h === "";
  }

  function handleHome(): void {
    if (onHome) {
      onHome();
      return;
    }
    if (browser) {
      window.location.hash = "/";
    }
  }
</script>

<header class="ad-board-header" data-tauri-drag-region>
  <button
    type="button"
    onclick={handleHome}
    class="ad-board-brand"
    disabled={!showHome}
    title={showHome ? "Go to home" : ""}
  >
    <span class="ad-board-brand-badge" aria-hidden="true">
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <rect x="3" y="4" width="7" height="7" rx="1.5" />
        <rect x="14" y="4" width="7" height="7" rx="1.5" />
        <rect x="8.5" y="13.5" width="7" height="7" rx="1.5" />
        <path d="M6.5 11v2.5h11V11" />
      </svg>
    </span>
    <span class="ad-board-brand-title">EXO算力管理</span>
  </button>

  <nav class="ad-board-tabs" aria-label="Main navigation">
    <a
      href="/#/suitable"
      class="ad-board-tab {isActive('suitable') ? 'is-active' : ''}"
      title="Models this setup can run"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M9 11l3 3L22 4" />
        <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
      </svg>
      <span>Suitable</span>
    </a>

    <a
      href="/#/downloads"
      class="ad-board-tab {isActive('downloads') ? 'is-active' : ''}"
      title="View downloads overview"
    >
      {#if downloadProgress}
        <span class="ad-board-dl-pill">
          {Math.round(downloadProgress.percentage)}%
        </span>
      {:else}
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 3v12" />
          <path d="M7 12l5 5 5-5" />
          <path d="M5 21h14" />
        </svg>
      {/if}
      <span>Downloads</span>
    </a>

    {#if showHome}
      <button
        type="button"
        onclick={handleHome}
        class="ad-board-tab {isActive('home') ? 'is-active' : ''}"
        title="Back to topology view"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3" y="4" width="18" height="12" rx="2" />
          <path d="M8 20h8M12 16v4" />
        </svg>
        <span>Devices</span>
      </button>
    {/if}
  </nav>
</header>
