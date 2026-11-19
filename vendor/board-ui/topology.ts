/** Shared device canvas for the EXO board and the llama board. */

export type BoardDeviceKind = "mac-mini" | "mac-studio" | "macbook";

export interface BoardDeviceAction {
  label: string;
  peer: string;
}

export interface BoardDevice {
  id: string;
  name: string;
  kind: BoardDeviceKind;
  memoryUsedBytes?: number;
  memoryTotalBytes?: number;
  gpuPercent?: number;
  detail?: string;
  dimmed?: boolean;
  action?: BoardDeviceAction;
}

export interface BoardLink {
  from: string;
  to: string;
}

export interface BoardTopologyOptions {
  width?: number;
  height?: number;
}

const APPLE_LOGO =
  "M788.1 340.9c-5.8 4.5-108.2 62.2-108.2 190.5 0 148.4 130.3 200.9 134.2 202.2-.6 3.2-20.7 71.9-68.7 141.9-42.8 61.6-87.5 123.1-155.5 123.1s-85.5-39.5-164-39.5c-76.5 0-103.7 40.8-165.9 40.8s-105.6-57-155.5-127C46.7 790.7 0 663 0 541.8c0-194.4 126.4-297.5 250.8-297.5 66.1 0 121.2 43.4 162.7 43.4 39.5 0 101.1-46 176.3-46 28.5 0 130.9 2.6 198.3 99.2zm-234-181.5c31.1-36.9 53.1-88.1 53.1-139.3 0-7.1-.6-14.3-1.9-20.1-50.6 1.9-110.8 33.7-147.1 75.8-28.5 32.4-55.1 83.6-55.1 135.5 0 7.8 1.3 15.6 1.9 18.1 3.2.6 8.4 1.3 13.6 1.3 45.4 0 102.5-30.4 135.5-71.3z";

export function deviceKindFromModel(model: string | undefined | null): BoardDeviceKind {
  const name = (model ?? "").toLowerCase();
  if (name.includes("studio")) return "mac-studio";
  if (name.includes("macbook") || name.includes("book")) return "macbook";
  return "mac-mini";
}

export function renderBoardTopology(
  devices: BoardDevice[],
  links: BoardLink[] = [],
  options: BoardTopologyOptions = {},
): string {
  const width = options.width ?? 780;
  const height = options.height ?? 460;
  if (devices.length === 0) {
    return `<svg class="ad-topo-svg" viewBox="0 0 ${width} ${height}"></svg>`;
  }
  const placed = placeDevices(devices, width, height);
  const byId = new Map(placed.map((item) => [item.device.id, item]));
  const drawn = links.length
    ? links
    : devices.slice(1).map((device) => ({ from: devices[0].id, to: device.id }));
  const edges = drawn
    .map((link) => {
      const from = byId.get(link.from);
      const to = byId.get(link.to);
      if (!from || !to) return "";
      return edge(from.x, from.y, to.x, to.y, from.radius * 0.85);
    })
    .join("");
  const nodes = placed.map((item) => deviceNode(item)).join("");
  return `<svg class="ad-topo-svg" viewBox="0 0 ${width} ${height}" role="img">${edges}${nodes}</svg>`;
}

interface Placed {
  device: BoardDevice;
  x: number;
  y: number;
  radius: number;
}

function placeDevices(devices: BoardDevice[], width: number, height: number): Placed[] {
  const count = devices.length;
  const minDim = Math.min(width, height);
  const radius =
    count <= 1
      ? Math.min(156, minDim * 0.34)
      : Math.min(108, minDim * 0.2) * Math.max(0.62, 1 - (count - 1) * 0.08);
  const cx = width / 2;
  const cy = height / 2 + 6;
  if (count === 1) return [{ device: devices[0], x: cx, y: cy, radius }];
  const orbit = Math.min(minDim * 0.3, Math.max(radius * 2.15, minDim * 0.22));
  return devices.map((device, index) => {
    const angle = -Math.PI / 2 + (index * 2 * Math.PI) / count;
    return {
      device,
      x: cx + orbit * Math.cos(angle),
      y: cy + orbit * Math.sin(angle),
      radius,
    };
  });
}

function edge(x1: number, y1: number, x2: number, y2: number, pad: number): string {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len = Math.hypot(dx, dy) || 1;
  const ux = dx / len;
  const uy = dy / len;
  return `<line x1="${n(x1 + ux * pad)}" y1="${n(y1 + uy * pad)}" x2="${n(x2 - ux * pad)}" y2="${n(y2 - uy * pad)}" stroke="rgba(37,99,235,0.55)" stroke-width="2" stroke-linecap="round"></line>`;
}

function deviceNode(item: Placed): string {
  const { device, x, y, radius } = item;
  const body =
    device.kind === "mac-studio"
      ? macStudio(device, radius)
      : device.kind === "macbook"
        ? macBook(device, radius)
        : macMini(device, radius);
  const label = memoryLabel(device);
  const nameSize = Math.max(13, radius * 0.15);
  const subSize = Math.max(11, radius * 0.12);
  const action = device.action
    ? `<foreignObject x="-46" y="${n(radius * 0.95)}" width="92" height="22"><button xmlns="http://www.w3.org/1999/xhtml" type="button" data-board-action="remove-peer" data-peer="${esc(device.action.peer)}" style="border:0;background:transparent;color:#64748b;font:650 12px -apple-system,sans-serif;cursor:pointer;width:100%;padding:0;">${esc(device.action.label)}</button></foreignObject>`
    : "";
  return `<g data-node="${esc(device.id)}" transform="translate(${n(x)} ${n(y)})" opacity="${device.dimmed ? 0.45 : 1}">
    ${body}
    <text y="${n(-radius * 0.72)}" text-anchor="middle" fill="#0f172a" font-size="${n(nameSize)}" font-weight="650">${esc(device.name)}</text>
    ${label ? `<text y="${n(radius * 0.78)}" text-anchor="middle" fill="#64748b" font-size="${n(subSize)}">${esc(label)}</text>` : ""}
    ${action}
  </g>`;
}

function macMini(device: BoardDevice, radius: number): string {
  const w = radius * 1.3;
  const h = radius * 0.7;
  return box(device, radius, w, h, h * 0.2, false);
}

function macStudio(device: BoardDevice, radius: number): string {
  const w = radius * 1.25;
  const h = radius * 0.85;
  return box(device, radius, w, h, h * 0.15, true);
}

function box(device: BoardDevice, radius: number, w: number, h: number, top: number, wideSlot: boolean): string {
  const x = -w / 2;
  const y = -h / 2;
  const clip = `mem-${slug(device.id)}`;
  const used = memoryRatio(device);
  const fillH = (h - top) * used;
  const fill = used
    ? `<rect x="${n(x)}" y="${n(y + top + (h - top - fillH))}" width="${n(w)}" height="${n(fillH)}" fill="rgba(255,215,0,0.75)" clip-path="url(#${clip})"></rect>`
    : "";
  const slotY = y + top + (h - top) * 0.55;
  const slotH = h * 0.16;
  const slots = [0.2, 0.32]
    .map(
      (at) =>
        `<rect x="${n(x + w * at)}" y="${n(slotY)}" width="${n(w * 0.045)}" height="${n(slotH)}" rx="1.2" fill="rgba(0,0,0,0.4)"></rect>`,
    )
    .join("");
  const sd = wideSlot
    ? `<rect x="${n(x + w * 0.48)}" y="${n(slotY)}" width="${n(w * 0.2)}" height="${n(slotH * 0.6)}" rx="1" fill="rgba(0,0,0,0.4)"></rect>`
    : "";
  return `<defs><clipPath id="${clip}"><rect x="${n(x)}" y="${n(y + top)}" width="${n(w)}" height="${n(h - top)}" rx="3"></rect></clipPath></defs>
    <rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" rx="4" fill="#1a1a1a" stroke="rgba(179,179,179,0.85)" stroke-width="1.5" ${device.dimmed ? 'stroke-dasharray="5 3"' : ""}></rect>
    ${fill}
    <line x1="${n(x)}" y1="${n(y + top)}" x2="${n(x + w)}" y2="${n(y + top)}" stroke="rgba(255,255,255,0.18)"></line>
    ${slots}${sd}
    ${gpuBar(device, w, h, radius)}`;
}

function macBook(device: BoardDevice, radius: number): string {
  const w = radius * 1.35;
  const h = radius * 0.95;
  const y = -h / 2;
  const screenH = h * 0.7;
  const screenW = w * 0.85;
  const screenX = -screenW / 2;
  const bezel = 3;
  const clip = `screen-${slug(device.id)}`;
  const used = memoryRatio(device);
  const innerH = screenH - bezel * 2;
  const fillH = innerH * used;
  const fill = used
    ? `<rect x="${n(screenX + bezel)}" y="${n(y + bezel + (innerH - fillH))}" width="${n(screenW - bezel * 2)}" height="${n(fillH)}" fill="rgba(255,215,0,0.85)" clip-path="url(#${clip})"></rect>`
    : "";
  const logoH = screenH * 0.22;
  const logoScale = logoH / 1000;
  const logoX = -(814 * logoScale) / 2;
  const logoY = y + screenH / 2 - (1000 * logoScale) / 2;
  const baseY = y + screenH;
  const baseH = h * 0.3;
  const baseTop = screenW;
  const baseBottom = w;
  return `<defs><clipPath id="${clip}"><rect x="${n(screenX + bezel)}" y="${n(y + bezel)}" width="${n(screenW - bezel * 2)}" height="${n(innerH)}" rx="2"></rect></clipPath></defs>
    <rect x="${n(screenX)}" y="${n(y)}" width="${n(screenW)}" height="${n(screenH)}" rx="3" fill="#1a1a1a" stroke="rgba(179,179,179,0.85)" stroke-width="1.5"></rect>
    <rect x="${n(screenX + bezel)}" y="${n(y + bezel)}" width="${n(screenW - bezel * 2)}" height="${n(innerH)}" rx="2" fill="#0a0a12"></rect>
    ${fill}
    <path d="${APPLE_LOGO}" transform="translate(${n(logoX)} ${n(logoY)}) scale(${logoScale})" fill="#fff" opacity="0.9"></path>
    <path d="M ${n(-baseTop / 2)} ${n(baseY)} L ${n(baseTop / 2)} ${n(baseY)} L ${n(baseBottom / 2)} ${n(baseY + baseH)} L ${n(-baseBottom / 2)} ${n(baseY + baseH)} Z" fill="#2c2c2c" stroke="rgba(179,179,179,0.8)"></path>
    ${gpuBar(device, w, h, radius)}`;
}

function gpuBar(device: BoardDevice, bodyW: number, bodyH: number, radius: number): string {
  const barW = Math.max(18, radius * 0.26);
  const barH = bodyH * 0.92;
  const x = bodyW / 2 + 12;
  const y = -barH / 2;
  const pct = clamp(device.gpuPercent ?? 0, 0, 100);
  const fillH = (pct / 100) * barH;
  const fill = pct
    ? `<rect x="${n(x)}" y="${n(y + barH - fillH)}" width="${n(barW)}" height="${n(fillH)}" rx="2" fill="rgba(37,99,235,0.85)"></rect>`
    : "";
  return `<rect x="${n(x)}" y="${n(y)}" width="${n(barW)}" height="${n(barH)}" rx="3" fill="rgba(80,80,90,0.72)"></rect>${fill}`;
}

function memoryRatio(device: BoardDevice): number {
  const total = device.memoryTotalBytes ?? 0;
  const used = device.memoryUsedBytes ?? 0;
  if (total <= 0 || used <= 0) return 0;
  return clamp(used / total, 0, 1);
}

function memoryLabel(device: BoardDevice): string {
  const total = device.memoryTotalBytes ?? 0;
  const used = device.memoryUsedBytes ?? 0;
  if (total > 0 && used > 0) {
    return `${formatBytes(used)}/${formatBytes(total)} (${Math.round((used / total) * 100)}%)`;
  }
  if (total > 0) return formatBytes(total);
  return device.detail ?? "";
}

function formatBytes(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  if (gb < 10) return `${gb.toFixed(1)} GB`;
  return `${Math.round(gb)} GB`;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function n(value: number): string {
  return value.toFixed(1);
}

function slug(id: string): string {
  return id.replace(/[^a-zA-Z0-9_-]/g, "") || "node";
}

function esc(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
