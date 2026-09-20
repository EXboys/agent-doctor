/** Build edition: personal (TeamUps) vs team (enterprise). Locked at package time. */
export type ProductEdition = "personal" | "team";

function parseEdition(raw: string | undefined): ProductEdition {
  const value = (raw ?? "").trim().toLowerCase();
  if (value === "team" || value === "enterprise" || value === "evotown") {
    return "team";
  }
  return "personal";
}

/** Frontend edition from Vite (`VITE_AGENT_DOCTOR_EDITION` / `AGENT_DOCTOR_EDITION`). */
export function productEdition(): ProductEdition {
  return parseEdition(import.meta.env.VITE_AGENT_DOCTOR_EDITION);
}

export function isPersonalEdition(): boolean {
  return productEdition() === "personal";
}

export function isTeamEdition(): boolean {
  return productEdition() === "team";
}
