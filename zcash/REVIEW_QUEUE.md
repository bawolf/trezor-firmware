# Review queue (GPT-6 Sol via Codex)

Codex has been failing (401 and reconnect errors on 2026-09-25; OpenAI status incident), so
the reviews wait here instead of blocking work. Run them with
`codex exec -m gpt-6-sol -s read-only … < /dev/null`, as the memory note describes.
**Prune:** drop a review when newer work makes it redundant or irrelevant.

| # | Review | Target | Status |
|---|---|---|---|
| 1 | Adversarial re-check of the key service after the fixes | `zcash/extapp-keys` @ `91fc95c0ac` (vs `zcash/extapp`), plus `EXTAPP_P1_KEYS.md` "Review resolutions" | **Folded into #3.** The key service is merged into the demo branch unchanged. |
| 2 | Adversarial review of the SDK fixes and the Zcash app | `zcash/extapp-app` @ `9b10fd2bfa` (vs `zcash/extapp`); prompt in the author's `whole-diff-review/SOL-P2A-PROMPT.md` | **Folded into #3.** P2b removes the test-seed stub and adds signing to the same code. |
| 3 | Adversarial review of the integrated extapp: key service + SDK fixes + Zcash app with signing | `zcash/extapp-demo` once P2b lands (vs `zcash/extapp`) | **Queued.** Run when P2b is done and Codex is healthy. |

Opus clarity reviews of #1 and #2 are done and applied. A clarity review of the P2b delta
goes with #3.
