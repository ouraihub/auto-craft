---
name: rebuilding-from-evidence
description: Rebuild a target from autocraft captures, requirements and cited evidence, then verify with rebuild checks.
---

# Rebuilding from evidence

Read the brief, requirements and `capture/README.md`, `capture/capture.json` and supplied screenshots before changing the app. Treat captures as observations, not a complete specification. Cite evidence ids for factual claims and explicitly label uncertain behavior. Read the existing investigation and handoff before gathering more evidence.

Respect clean-room rules. Recreate observable behavior from evidence; do not read target source, decompile, inspect hidden implementation details or copy protected assets. Use approved public references and the supplied captures. Work only within the allowed app/check paths and submit actions to the host approval policy; never bypass a refusal.

Keep the checklist with `core_plan_update` using todo/doing/done/blocked, and update it as work proceeds. Use `core_unknown_add` for missing facts instead of inventing behavior. Investigate unknowns with the autocraft MCP tools when permitted.

Make a coherent change, run `rebuild_check`, read its actual results and fix failures before claiming completion. Never substitute a successful build for behavioral or visual evidence. Explain any remaining unknowns honestly. Before ending each turn, write `core_handoff_write` with what changed, evidence, actual checks and the next step.
