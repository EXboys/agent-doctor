# Anonymous install ping (装机统计)

The desktop client counts installations without requiring an account. Each
installation sends one tiny POST per launch to agentdoctor.cn. Everything
client-side lives in `crates/agent-doctor-core/src/install_ping.rs`.

## What is sent

`POST https://agentdoctor.cn/api/v1/doctor/install/ping`, JSON body:

```json
{
  "install_id": "0f8dd5a6-…",   // random UUID v4, created on first launch
  "version": "0.1.98-beta.1",   // app version
  "os": "macos",                // macos | windows | linux
  "arch": "aarch64",            // aarch64 | x86_64 | …
  "edition": "personal",        // personal | team
  "client": "desktop",          // which shipped surface sent it
  "first_run": false            // true only on the launch that created the ID
}
```

No account link, no hardware fingerprint, no paths, no usernames. The ID is
random, so it identifies an installation, not a person. Reinstalling or
clearing the settings DB counts as a new machine (slight overcount, accepted).

## Server side (agentdoctor.cn)

Shipped and live: the receiver lives in the `teamups` repo (deployed to the
ECS behind agentdoctor.cn):

- `src/app/api/v1/doctor/install/ping/route.ts` — the POST endpoint
- `src/lib/install-ping.ts` — validation, upsert, and `installPingStats()`
  (total / active 7d / daily new), already surfaced on the `/admin` overview
- `src/lib/db.ts` — the `install_pings` table

Store one row per `install_id`: `first_seen_at` from the first insert is the
install time; later pings refresh `last_seen_at` and the version/os fields.
Any 2xx response is success; anything else (or a timeout) is silently ignored
by the client. Handy queries (timestamps are ISO text, matching the server):

```sql
-- total installs
SELECT COUNT(*) FROM install_pings;

-- active in the last 7 days
SELECT COUNT(*) FROM install_pings
WHERE last_seen_at >= strftime('%Y-%m-%dT%H:%M:%S', 'now', '-7 days');

-- daily new installs (last 30 days)
SELECT date(first_seen_at) AS day, COUNT(*) AS new_installs
FROM install_pings
WHERE first_seen_at >= strftime('%Y-%m-%dT%H:%M:%S', 'now', '-30 days')
GROUP BY day ORDER BY day;

-- by os / edition / version
SELECT os, edition, version, COUNT(*) FROM install_pings
GROUP BY os, edition, version ORDER BY COUNT(*) DESC;
```

Suggested local testing override: set `AGENT_DOCTOR_INSTALL_PING_URL` to a
local receiver before launching the app.

## Opt-out

Counting is on by default (opt-out model). The switch lives in the personal
panel under「隐私」(`install-ping-toggle`), backed by the
`install_ping_opt_out` meta flag in the settings DB. When off, the client
sends nothing at all — no ping, no ID creation. The desktop startup ping runs
on a background thread 10 s after launch (`spawn_install_ping` in
`desktop/src-tauri/src/lib.rs`) and never blocks or surfaces errors.

## Privacy note

Disclose the counting in the site privacy policy: "匿名装机统计 — 上报随机
ID、应用版本和系统类型，不含任何个人数据，可在应用内关闭。" Do not add
fields (IP-derived IDs, hostnames, paths) to the payload.
