# LIGH Agent Scorepack — product contract for the truth-machine bet

**Buyers:** agent platforms, eval labs, CI for agent-authored PRs.  
**Not buyers (primary):** solo Cursor users seeking daily tap MCP (Maestro + XcodeBuildMCP own that).

## Job

```text
frozen task → inject UI bug → agent or TRAIL → Simulator certify → ok:true only
→ scoreboard: verified / holy_shit / wall_ms / tokens / fault
```

Trust is the product. Vibes merge is not allowed.

## Compose, don’t clone

| Partner | Owns | LIGH owns |
|---------|------|-----------|
| **Maestro** | Durable E2E YAML + Cloud | Ephemeral prove/repair of **this** change |
| **XcodeBuildMCP** | Build / test / sim toolkit | Effect-class localize + certify |
| **Apple Xcode agents** | Official build/preview | Stranger/agent **repair scoreboards** |

## Pack

- Manifest: [`scorepack/v1/manifest.json`](../scorepack/v1/manifest.json) (`ligh.scorepack.v1`)
- Gate: `./scripts/gate-scorepack.sh` (or `--dry-run` for contract CI)
- Result: [`docs/assets/scorepack-latest.json`](assets/scorepack-latest.json) (`ligh.scorepack.result.v1`)

Core tasks (three effect shapes): login gate · tab chrome · overlay/onboarding.

## CI

| Workflow | Role |
|----------|------|
| `ligh-scorepack.yml` | Scorepack dry-run always; full Mac scorepack when secrets set |
| `ligh-certify.yml` | PR goal-certify on a Tier B `.app` (agent change surface) |

## Local dogfood (secondary)

`./scripts/ligh-paradise.sh` + MCP remain for developers who already own a Mac.  
**Do not lead marketing with paradise** — lead with scorepack + certify.

## Host requirements (honest)

Scorepack builds are **memory-heavy**. BuildGovernor fails with **`infra_oom`** (not silent kill theater) when free RAM is below `LIGH_BUILD_MIN_FREE_MB` (default **2048**) or xcodebuild is jetsam’d.

Fixture `build-*.sh` scripts must **not** pipe `xcodebuild | tail` — that hides SIGKILL from the governor.

Publish fail boards. A red `claim_pass: false` with structured faults is the product working.
