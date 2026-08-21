# Consumer Agent Vision — frontier design

**Honest product:** LIGH is not pixel CV. It is a **settled accessibility perception + human motor** stack so agents navigate iOS **without PNGs**, faster and more precisely than screenshot-to-LLM loops on labeled UI.

## Frontier control law

```text
observe(settle) → eyes ready? → act(label-first) → sense/verify → done
                      ↓ no
                 wait / home recover
```

| Principle | Implementation |
|-----------|----------------|
| Never plan on a blink | `observe --settle-ms` polls until `ax_quality=ready` + actionable (or timeout). Mid-nav trees → `transition`. |
| Stable targets | Semantic `id` (identifier or role+label+coarse center). **Tap prefers `--label`.** |
| Chrome out of policy | Spotlight `spotlight-pill` / status bar filtered from `actionable_topk`. |
| Surface, not vibes | `scene.surface`: `springboard` \| `settings` \| `messages_composer` \| `app` \| `transition` |
| Feel after act | `typed` sensation when HID accepts text (Messages often omits AX `value`). |
| Screenshots | Debug only. Forbidden on agent happy path. |

## What ships

- Observe v2 + settle + surface + chrome filter
- Motor: tap/label/id, long-press, scroll-until, clear, key, sense
- Agent loop: `scripts/agent-llm-loop.py` (host policy + LLM when ambiguous)

## Competitiveness (falsifiable)

1. ~~LLM goals without images ≥20/20 Messages + Settings~~ → **passed 40/40** (`gpt-5-mini`, 2026-08-21)
2. Vs vision-only: ≤ fail rate and ≤ tokens/step — **not run yet**
3. Micro: long-press / scroll-until — **passed** in substrate gate
4. No type loops when `typed` confirms — **host `typed` event**

Gate: `./scripts/gate-consumer-vision.sh`  
LLM: `LIGH_LLM_GATE=1 OPENAI_API_KEY=… ./scripts/gate-consumer-vision.sh`

## Not claimed

- Generic “computer use” on unlabeled/custom canvas apps  
- OCR/change-map to the model (IOSurface stays in-process / future)  
- That the LLM alone navigates without host settle/surface policy — the loop is **co-designed**

Speed vs WDA is real. **Agent navigation on labeled iOS UI without PNGs is now gated 40/40 on Messages+Settings.**
