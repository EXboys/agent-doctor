#!/usr/bin/env python3
"""Agent Doctor UX: hide exo chat only — keep onboarding and everything else.

- Default to topology-only mode (hides conversation chrome, keeps device graph).
- Hide the "exit topology-only / back to chat" control.
- Always mount HeaderNav (overlaid by vendor/exo-dashboard-zh/components/HeaderNav.svelte).
- Import Agent Doctor light theme CSS and set title to EXO算力管理.
- Do NOT rewrite onboarding steps, Continue, or redirects.
"""

from __future__ import annotations

import pathlib
import re
import sys


def patch_home_page(path: pathlib.Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    orig = text
    notes: list[str] = []

    # Always show HeaderNav in topology-only mode so Suitable / Downloads stay reachable.
    if "Agent Doctor UX: always show HeaderNav" not in text:
        old = """  {#if !topologyOnlyEnabled}
    <HeaderNav
      showHome={true}
      onHome={handleGoHome}
      showSidebarToggle={true}
      {sidebarVisible}
      onToggleSidebar={toggleChatSidebarVisible}
      showMobileMenuToggle={true}
      mobileMenuOpen={mobileChatOpen}
      onToggleMobileMenu={toggleMobileChatSidebar}
      showMobileRightToggle={!chatStarted && !topologyOnlyEnabled}
      {mobileRightOpen}
      onToggleMobileRight={toggleMobileRightSidebar}
      downloadProgress={activeDownloadSummary}
    />
  {/if}"""
        new = """  <!-- Agent Doctor UX: always show HeaderNav (Downloads must stay reachable). -->
  <HeaderNav
    showHome={true}
    onHome={handleGoHome}
    showSidebarToggle={false}
    showMobileMenuToggle={false}
    showMobileRightToggle={false}
    downloadProgress={activeDownloadSummary}
  />"""
        if old in text:
            text = text.replace(old, new, 1)
            notes.append("always-HeaderNav")

    # Hide "exit topology-only mode" (returns users to exo chat).
    if "Agent Doctor UX: hide exit-chat control" not in text:
        pattern = re.compile(
            r"(<!-- Exit topology-only mode button -->\s*)"
            r"(<button\b[\s\S]*?onclick=\{toggleTopologyOnlyMode\}[\s\S]*?</button>)",
            re.MULTILINE,
        )

        def wrap_exit(m: re.Match[str]) -> str:
            return (
                m.group(1)
                + "<!-- Agent Doctor UX: hide exit-chat control -->\n"
                + "          {#if false}\n"
                + m.group(2)
                + "\n          {/if}"
            )

        text2, n = pattern.subn(wrap_exit, text, count=1)
        if n == 1:
            text = text2
            notes.append("hide-exit-chat")

    if text != orig:
        path.write_text(text, encoding="utf-8")
    return notes


def patch_layout(layout: pathlib.Path) -> list[str]:
    notes: list[str] = []
    layout_text = layout.read_text(encoding="utf-8")
    layout_orig = layout_text
    if 'agent-doctor-theme.css' not in layout_text and 'import "../app.css";' in layout_text:
        layout_text = layout_text.replace(
            'import "../app.css";',
            'import "../app.css";\n  import "../agent-doctor-theme.css";',
            1,
        )
        notes.append("theme-import")
    if "<title>EXO</title>" in layout_text:
        layout_text = layout_text.replace("<title>EXO</title>", "<title>EXO算力管理</title>", 1)
        notes.append("document-title")
    if layout_text != layout_orig:
        layout.write_text(layout_text, encoding="utf-8")
    return notes


def patch_app_store(path: pathlib.Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    orig = text
    notes: list[str] = []
    # Default topology-only so the post-onboarding home shows devices, not chat.
    if "topologyOnlyMode = $state(false)" in text:
        text = text.replace(
            "topologyOnlyMode = $state(false)",
            "topologyOnlyMode = $state(true) // Agent Doctor: hide chat by default",
            1,
        )
        notes.append("default-topology-only")
    if 'localStorage.getItem("exo-topology-only-mode")' in text and "Agent Doctor UX: prefer topology-only" not in text:
        old = """      const stored = localStorage.getItem("exo-topology-only-mode");
      if (stored !== null) {
        this.topologyOnlyMode = stored === "true";
      }"""
        new = """      // Agent Doctor UX: prefer topology-only (ignore stale false from stock exo chat).
      const stored = localStorage.getItem("exo-topology-only-mode");
      if (stored !== null && stored === "true") {
        this.topologyOnlyMode = true;
      } else {
        this.topologyOnlyMode = true;
      }"""
        if old in text:
            text = text.replace(old, new, 1)
            notes.append("prefer-topology-only")
    if text != orig:
        path.write_text(text, encoding="utf-8")
    return notes


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <dashboard-work-dir>", file=sys.stderr)
        return 2
    work = pathlib.Path(sys.argv[1])
    page = work / "src" / "routes" / "+page.svelte"
    layout = work / "src" / "routes" / "+layout.svelte"
    theme = work / "src" / "agent-doctor-theme.css"
    store = work / "src" / "lib" / "stores" / "app.svelte.ts"
    if not page.is_file() or not layout.is_file() or not theme.is_file() or not store.is_file():
        print(f"missing sources under {work}", file=sys.stderr)
        return 1

    notes: list[str] = []
    notes.extend(patch_layout(layout))
    notes.extend(patch_home_page(page))
    notes.extend(patch_app_store(store))
    print("ux-patch: " + (", ".join(notes) if notes else "no-op"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
