<script lang="ts">
  /**
   * EXO 组网 uses the same device canvas as the llama board.
   * Source of the drawing: vendor/board-ui/topology.ts
   * (copied to src/lib/board-ui/ during the dashboard build).
   */
  import { topologyData } from "$lib/stores/app.svelte";
  import {
    deviceKindFromModel,
    renderBoardTopology,
    type BoardDevice,
    type BoardLink,
  } from "../board-ui/topology";

  interface Props {
    class?: string;
    highlightedNodes?: Set<string>;
    filteredNodes?: Set<string>;
    onNodeClick?: (nodeId: string) => void;
  }

  let {
    class: className = "",
    filteredNodes = new Set<string>(),
    onNodeClick,
  }: Props = $props();

  const data = $derived(topologyData());
  const markup = $derived.by(() => {
    const nodes = data?.nodes ?? {};
    const hidden = filteredNodes ?? new Set<string>();
    const devices: BoardDevice[] = Object.entries(nodes).map(([id, node]) => ({
      id,
      name: node.friendly_name || node.system_info?.model_id || id.slice(0, 8),
      kind: deviceKindFromModel(node.system_info?.model_id),
      memoryUsedBytes: node.macmon_info?.memory?.ram_usage,
      memoryTotalBytes: node.macmon_info?.memory?.ram_total ?? node.system_info?.memory,
      gpuPercent:
        typeof node.macmon_info?.gpu_usage?.[1] === "number"
          ? node.macmon_info.gpu_usage[1] * 100
          : undefined,
      dimmed: hidden.size > 0 && !hidden.has(id),
    }));
    const links: BoardLink[] = (data?.edges ?? []).map((edge) => ({
      from: edge.source,
      to: edge.target,
    }));
    return renderBoardTopology(devices, links, { width: 1100, height: 680 });
  });

  function clickNode(event: MouseEvent) {
    const node = (event.target as Element | null)?.closest?.("[data-node]");
    const id = node?.getAttribute("data-node");
    if (id) onNodeClick?.(id);
  }
</script>

<div class="ad-topo-stage ad-topo-stage-fill {className}" onclick={clickNode} role="presentation">
  {@html markup}
</div>
