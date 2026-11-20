//! Bundled analysis methods and the per-launch system instruction; no session state.

use autocraft_core::{Result, SessionRole, StartSession};
use std::path::PathBuf;

pub(crate) const ANALYSIS_RULE: &str = "autocraft analysis tools are on the autocraft MCP server (autocraft-analysis in OMP); cite their ev_ evidence ids for factual claims and distinguish observations from inferences.";
pub(crate) const QUESTION: &str = "You're answering a question about the project built inside autocraft. This isn't a task: answer it and stop. Don't create, change or delete any file, don't run anything, and don't carry on the build; if something should change, explain what and let the person request a change. Answer briefly in plain words for someone who doesn't code, with full paths when asked where something is. You're a short-lived copy with fewer tools than the builder, and the build may still be running beside you. A tool you can't see here is not a tool the build has lost, and a call with no result here may still be running: never report tools as disconnected or work as cut off because of what you see here.";

pub(crate) fn instruction(role: SessionRole) -> &'static str {
    match role {
        SessionRole::Worker => ANALYSIS_RULE,
        SessionRole::ReadOnly => QUESTION,
    }
}

/// Stage only the method relevant to this launch; read-only sessions load no skills.
pub(crate) fn stage(request: &StartSession) -> Result<Option<PathBuf>> {
    if request.role == SessionRole::ReadOnly {
        return Ok(None);
    }
    let (name, body) = if request.builder.is_some() {
        (
            "rebuilding-from-evidence",
            include_str!("../skills/rebuilding-from-evidence/SKILL.md"),
        )
    } else {
        (
            "investigating-a-target",
            include_str!("../skills/investigating-a-target/SKILL.md"),
        )
    };
    let root = request.support_dir.join("skills");
    crate::host::write_support(&root.join(name), "SKILL.md", body)?;
    Ok(Some(crate::process::plain_path(&root)))
}

/// Pi's structured sections preserve the engine's own prompt cache ownership.
pub(crate) fn pi_extension(request: &StartSession) -> String {
    let mut body =
        crate::projection_engine::pi_extension(request.analysis.is_some(), &request.routing);
    body.pop();
    body.pop();
    body.push_str(&format!("\tpi.on(\"before_agent_start\", (event) => {{ event.systemPromptOptions.sections[\"autocraft-analysis\"] = {}; }});\n", serde_json::json!(instruction(request.role))));
    if request.builder.is_some() {
        body.push_str(PI_APPROVAL);
    }
    body.push_str("}\n");
    body
}

const PI_APPROVAL: &str = r#"
    pi.on("tool_call", async (event, ctx) => {
        if (["read", "grep", "find", "ls"].includes(event.toolName)) return;
        const tool = pi.getAllTools().find((candidate) => candidate.name === event.toolName);
        if (tool?.namespace?.name === "mcp__autocraft" || event.toolName.startsWith("mcp__autocraft__")) return;
        const raw = await ctx.ui.input("autocraft:approve", JSON.stringify({ toolName: event.toolName, input: event.input }));
        let verdict;
        try { verdict = JSON.parse(raw ?? ""); } catch { verdict = { allow: false, reason: "The approval check gave no answer." }; }
        if (verdict?.allow !== true) return { block: true, reason: verdict?.reason ?? "The action was refused." };
    });
"#;

pub(crate) fn omp_extension(request: &StartSession) -> String {
    let mut body = crate::projection_engine::omp_extension(&request.routing);
    body.pop();
    body.pop();
    body.push_str(&format!("\tpi.on(\"before_agent_start\", (event) => ({{ systemPrompt: [...event.systemPrompt, {}] }}));\n", serde_json::json!(instruction(request.role))));
    if request.builder.is_some() {
        body.push_str(OMP_APPROVAL);
    }
    body.push_str("}\n");
    body
}

pub(crate) fn omp_overlay(request: &StartSession, skills: Option<&std::path::Path>) -> String {
    let mut body = crate::projection_engine::omp_overlay(&request.routing);
    // OMP 18.18.x otherwise mounts MCP tools behind xd:// instead of advertising schemas.
    body.push_str("tools:\n  xdev: false\n");
    if request.analysis.is_some()
        && request.routing.mcp.is_empty()
        && request.routing.default_model.is_none()
    {
        body.push_str("mcp:\n  enableProjectConfig: false\n");
    }
    if let Some(skills) = skills {
        body.push_str(&format!("skills:\n  enableCodexUser: false\n  enableClaudeUser: false\n  enableClaudeProject: false\n  enablePiUser: false\n  enablePiProject: false\n  enableAgentsUser: false\n  enableAgentsProject: false\n  customDirectories: [{}]\n", serde_json::json!(skills.display().to_string())));
    }
    if request.builder.is_some() {
        body.push_str("todo:\n  enabled: false\n");
    }
    body
}

pub(crate) fn dsh_patch(
    request: &StartSession,
    skills: Option<&std::path::Path>,
) -> Result<String> {
    let mut body = crate::projection_engine::dsh_patch(&request.routing)?;
    body.push_str(&format!(
        "- id: system-prompt\n  config:\n    personaPrefix: {}\n",
        serde_json::json!(instruction(request.role))
    ));
    if let Some(skills) = skills {
        body.push_str(&format!(
            "- id: skill-filesystem\n  config:\n    customSkillDirs: [{}]\n",
            serde_json::json!(skills.display().to_string())
        ));
    }
    if request.role == SessionRole::ReadOnly {
        body.push_str("- id: tool-fs\n  config:\n    write: false\n    edit: false\n");
        for row in [
            "tool-todo",
            "tool-goal",
            "tool-skill",
            "tool-workflow",
            "tool-subagent",
            "tool-subagent-fork",
            "tool-subagent-control",
            "tool-subagent-list-agents",
            "tool-web",
            "plan-mode",
            "tool-bash",
            "tool-pwsh",
        ] {
            body.push_str(&format!("- id: {row}\n  disabled: true\n"));
        }
    } else {
        body.push_str("- id: tool-todo\n  disabled: true\n");
    }
    Ok(body)
}

const OMP_APPROVAL: &str = r#"
    pi.on("tool_call", async (event, ctx) => {
        if (["read", "grep", "find", "ls", "glob"].includes(event.toolName)) return;
        if (event.toolName.startsWith("mcp__autocraft_analysis_")) return;
        const raw = await ctx.ui.input("autocraft:approve", JSON.stringify({ toolName: event.toolName, input: event.input }));
        let verdict;
        try { verdict = JSON.parse(raw ?? ""); } catch { verdict = { allow: false, reason: "The approval check gave no answer." }; }
        if (verdict?.allow !== true) return { block: true, reason: verdict?.reason ?? "The action was refused." };
    });
"#;
