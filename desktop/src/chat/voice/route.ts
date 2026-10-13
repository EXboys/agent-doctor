import { teamupsAccountStatus, voiceCloudReady } from "../../ipc";

const KEY = "agent-doctor-voice-route";

export type VoiceRoute = "local" | "cloud";

/** Local is the default. Cloud is only kept when the person is signed in. */
export function readVoiceRoute(): VoiceRoute {
  try {
    return localStorage.getItem(KEY) === "cloud" ? "cloud" : "local";
  } catch {
    return "local";
  }
}

export function writeVoiceRoute(route: VoiceRoute): void {
  try {
    localStorage.setItem(KEY, route);
  } catch {
    /* ignore */
  }
}

/** The route for this listen or reply. A signed-out cloud choice falls back to this computer. */
export async function activeVoiceRoute(): Promise<VoiceRoute> {
  if (readVoiceRoute() !== "cloud") return "local";
  try {
    const [account, ready] = await Promise.all([
      teamupsAccountStatus(),
      voiceCloudReady(),
    ]);
    if (account?.signed_in && ready) return "cloud";
  } catch {
    /* stay local */
  }
  return "local";
}
