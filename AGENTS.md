# Repository Guidelines & Agent Rules

## Model & Tooling Notes (Qwen 3.8 27B)
- Keep reasoning structured and concise; avoid sweeping edits across files that were not requested.
- Prefer reading exact files/symbols before proposing edits.
- Never fabricate command outputs, test results, or file contents—only report what was directly observed.

## Execution Protocol
1. **Explore & Plan:** Read relevant files/context first. Outline a concise step-by-step plan before making changes.
2. **Surgical Execution:** Make minimal, targeted changes. Never alter unrelated logic or delete tests to bypass failures.
3. **Non-Interactive Commands:** Always use non-interactive CLI flags (e.g., `-y`, `--yes`) to prevent terminal hangs.
4. **Verify with Evidence:** Run tests/commands after changes and inspect the raw output. Never assume success without proof.
5. **Retry Cap:** If an approach fails 3 times, stop, document the root cause, and ask the user.
6. **Final FMEA Check:** Before saying DONE, check top 2-3 edge cases or regressions, fix any issues, and report remaining caveats.

## Output Format for Updates
When reporting progress or completion, structure clearly:
- **Plan / Changes Made:** Bullet points of exact files and edits.
- **Verification Evidence:** Commands run and key raw output/status.
- **FMEA & Caveats:** Edge cases checked, fixes applied, and any remaining risks.