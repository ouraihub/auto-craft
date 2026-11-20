---
name: investigating-a-target
description: Investigate a program, app, file, website or device with autocraft analysis tools and evidence-backed findings.
---

# Investigating a target

Start with `core_session_show`, `core_targets_list` and `core_investigation_show`. Resume the existing handoff instead of restarting the investigation. For a new question, write the brief with `core_brief_write` and a plan with `core_plan_update` before analysis.

Observe, then derive, then infer. Cite the returned `ev_` evidence ids for every factual claim. Separate direct observations, deductions, guesses and unknowns. Static analysis cannot prove code ran. A failed search establishes only the scope searched, not absence.

Read summaries and overviews first, then page into relevant detail with offset/limit. Predict what an expensive call should show and compare its result with that prediction. Do not repeat an identical call that already failed or answered the question. If a tool is missing, use `list_toolsets` and `enable_toolset`.

After each batch, update plan statuses, record/test hypotheses, and record settled findings with evidence ids. Correct findings using `core_finding_edit` and the revision from `core_investigation_show`; do not silently replace conclusions. Record uncertainty with `core_unknown_add`.

Before ending each turn, call `core_handoff_write` with the current state, next step and traps. Reports must come from findings and evidence, not chat memory alone.
