import { promptRequestsBrowserMcp, type MentionRef } from "../ask-resources";

export type HandsBlock = "browser" | "computer" | "both";

/** The sentence is asking to click other apps on this computer. */
export function promptRequestsComputerUse(text: string): boolean {
  const lower = text.toLowerCase();
  if (lower.includes("computer use")) return true;
  return (
    text.includes("操作这台电脑") ||
    text.includes("操作电脑") ||
    text.includes("控制这台电脑") ||
    text.includes("控制电脑") ||
    text.includes("千牛")
  );
}

/**
 * Which closed switch this sentence needs.
 * `browserOn` / `computerOn` are the settings switches.
 */
function mentionsBrowser(mentions: MentionRef[]): boolean {
  return mentions.some((item) => item.kind === "mcp" && item.id.toLowerCase().includes("browser"));
}

export function handsBlockForText(
  text: string,
  browserOn: boolean,
  computerOn: boolean,
  mentions: MentionRef[] = [],
): HandsBlock | null {
  const browser = !browserOn && (promptRequestsBrowserMcp(text) || mentionsBrowser(mentions));
  const computer = promptRequestsComputerUse(text) && !computerOn;
  if (browser && computer) return "both";
  if (browser) return "browser";
  if (computer) return "computer";
  return null;
}
