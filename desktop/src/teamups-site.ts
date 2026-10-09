/** Default TeamUps origin (no trailing slash). Keep in sync with `default_teamups_base_url` in core. */
export const DEFAULT_TEAMUPS_BASE_URL = "https://teamups.vip";

let activeBaseUrl = DEFAULT_TEAMUPS_BASE_URL;

/** Use API / saved wiring base when it is a full http(s) origin. */
export function setTeamupsBaseUrl(url: string | null | undefined): void {
  const base = url?.trim().replace(/\/+$/, "");
  if (base && (base.startsWith("https://") || base.startsWith("http://"))) {
    activeBaseUrl = base;
    return;
  }
  activeBaseUrl = DEFAULT_TEAMUPS_BASE_URL;
}

export function teamupsBaseUrl(): string {
  return activeBaseUrl;
}

export function teamupsUrl(path: string): string {
  const segment = path.startsWith("/") ? path : `/${path}`;
  return `${teamupsBaseUrl()}${segment}`;
}

/** Ask sidebar account control — personal center on TeamUps. */
export function teamupsAccountPageUrl(): string {
  return teamupsUrl("/account");
}
