/**
 * One copy button per assistant turn. Tool rows between sentences stay in the
 * same turn; a user message starts the next one. The copy, the time, and the
 * rating sit on the last assistant fragment — but only when that fragment
 * ends the turn. A process row after it means the sentence is still part of
 * the work, and the button row would cover that process line.
 */
export function assistantTurnCopyOwners(roles: readonly string[]): boolean[] {
  const owners = roles.map(() => false);
  let lastAssistant = -1;
  let processAfter = false;
  const close = () => {
    if (lastAssistant >= 0 && !processAfter) owners[lastAssistant] = true;
    lastAssistant = -1;
    processAfter = false;
  };
  for (let i = 0; i < roles.length; i++) {
    const role = roles[i];
    if (role === "user") {
      close();
      continue;
    }
    if (role === "assistant") {
      lastAssistant = i;
      processAfter = false;
      continue;
    }
    if (role === "process" && lastAssistant >= 0) processAfter = true;
  }
  close();
  return owners;
}

/** Clipboard text for one assistant turn: each fragment, in order, separated by a blank line. */
export function joinAssistantTurnCopy(parts: readonly string[]): string {
  return parts
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
    .join("\n\n");
}
