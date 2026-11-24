# Network proxy for agent CLIs

`agent-doctor-proxy` decides which HTTP proxy Claude Code, Codex, Antigravity, Hermes, and every other agent CLI child should use. It does **not** run a proxy and never forwards traffic.

A desktop app opened from the Dock inherits a bare environment: no `https_proxy` from `~/.zshrc`. The same CLI that works in Terminal then times out against overseas model hosts. This crate closes that gap.

## Priority (first hit wins)

1. `AGENT_DOCTOR_PROXY` — explicit URL, or `direct` / `off` / `none` to strip every proxy.
2. Proxy variables already on this process (app launched from a terminal).
3. Proxy variables from the user’s login shell, if that proxy port answers within ~300 ms.
4. The OS system proxy (macOS Network settings / Windows Internet Settings), if that port answers.
5. Nothing. TUN / VPN / OpenVPN users land here and need no variables.

Only proxy and certificate variables are taken from the shell. Keys such as `ANTHROPIC_API_KEY` must not change which service an agent talks to.

## Wiring

- Desktop startup calls `agent_doctor_core::proxy::warm()`. A background thread computes the plan immediately, then every 30 seconds re-reads the system proxy and checks that the local proxy port still answers. The login shell is read once.
- Spawning a child only reads that stored plan. It does not wait on the shell or open a socket. `AGENT_DOCTOR_PROXY` and proxy variables already on this process still apply immediately.
- `exec::command_for_path` applies the plan to every long-lived or short CLI child. Callers that must stay direct (China installer downloads) remove proxy variables after building the command — same pattern as today.
- `raw_spawn_guard` covers this crate’s sources.

## Limits

- SOCKS-only system proxies are not applied: Claude Code and Node do not speak SOCKS. Prefer the HTTP port (Clash mixed port, V2rayU HTTP).
- PAC auto-config has no single address to hand on; the plan stays direct and records a note.
- Install shell scripts that use China mirrors stay on their own path and are not forced through this plan.

## Failure copy

A transport-shaped chat failure (reconnect exhausted, channel closed, bare timeout without an HTTP status) maps to `network`, not `provider_url`. The UI says the model servers cannot be reached and offers “Ask again”, never “Check the address”.
