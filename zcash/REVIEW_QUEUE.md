# Review queue (GPT-6 Sol via Codex)

Codex has been failing (401 and reconnect errors on 2026-09-25; OpenAI status incident), so
the reviews wait here instead of blocking work. Run them with
`codex exec -m gpt-6-sol -s read-only … < /dev/null`, as the memory note describes.
**Prune:** drop a review when newer work makes it redundant or irrelevant.

| # | Review | Target | Status |
|---|---|---|---|
| 1 | Adversarial re-check of the key service after the fixes | `zcash/extapp-keys` @ `91fc95c0ac` (vs `zcash/extapp`), plus `EXTAPP_P1_KEYS.md` "Review resolutions" | **Folded into #3.** The key service is merged into the demo branch unchanged. |
| 2 | Adversarial review of the SDK fixes and the Zcash app | `zcash/extapp-app` @ `9b10fd2bfa` (vs `zcash/extapp`); prompt in the author's `whole-diff-review/SOL-P2A-PROMPT.md` | **Folded into #3.** P2b removes the test-seed stub and adds signing to the same code. |
| 3 | Adversarial review of the integrated extapp: key service + SDK fixes + Zcash app with signing | `zcash/extapp-signstart` @ `07a27d1d76` (vs `zcash/extapp`) | **Done 2026-09-26.** [Sol](reviews/SOL-REVIEW-P3-INTEGRATED-07a27d1d76.md) and a genuine Fable 5.1 run ([Fable](reviews/FABLE-REVIEW-P3-INTEGRATED-07a27d1d76.md)). Both: no path to a signature over unreviewed data. Both must-fix the production gating of the test-only kernel ML-DSA commit `d485d458c5`. Resolutions are recorded with the series shaping pass. |

Opus clarity reviews of #1 and #2 are done and applied. A readability review of the whole
range, against Trezor's conventions, is running with #3.
