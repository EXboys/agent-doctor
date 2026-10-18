/**
 * One copy button per assistant turn. Tool rows between sentences stay in the
 * same turn; a user message starts the next one. The copy and the time sit on
 * the last assistant fragment.
 */
export function assistantTurnCopyOwners(roles: readonly string[]): boolean[] {
  const owners = roles.map(() => false);
  let lastAssistant = -1;
  const close = () => {
    if (lastAssistant >= 0) owners[lastAssistant] = true;
    lastAssistant = -1;
  };
  for (let i = 0; i < roles.length; i++) {
    if (roles[i] === "user") {
      close();
      continue;
    }
    if (roles[i] === "assistant") lastAssistant = i;
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
