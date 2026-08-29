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

## Distance (honest)

| Layer | Status |
|-------|--------|
| Fail-closed certify + TRAIL lab | Strong |
| External scorepack SKU | **This pack** — start of product |
| Hosted Mac multi-tenant | Missing (commodity without unique job) |
| Production breadth | Narrow effect classes — expand pack versions, not tap MCP |
