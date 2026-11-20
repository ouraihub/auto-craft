// Test-only official pi extension: a loopback provider and observable approval/tool-list hooks.
import fs from "node:fs";

export default function (pi) {
    pi.registerProvider("autocraft-p02-loopback", {
        name: "P02 loopback fixture",
        baseUrl: process.env.AUTOCRAFT_P02_BASE_URL,
        api: "openai-completions",
        apiKey: "$AUTOCRAFT_P02_KEY",
        models: [{
            id: "p02-scripted", name: "P02 scripted", reasoning: false, input: ["text"],
            cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
            contextWindow: 128000, maxTokens: 4096,
        }],
    });
    pi.on("session_start", (event) => {
        if (process.env.AUTOCRAFT_P02_TOOLS_REPORT) {
            fs.appendFileSync(process.env.AUTOCRAFT_P02_TOOLS_REPORT, JSON.stringify({
                reason: event.reason,
                active: pi.getActiveTools(),
                configured: pi.getAllTools().map((tool) => tool.name),
            }) + "\n");
        }
    });
    pi.on("tool_call", async (event, ctx) => {
        if (event.toolName !== "write" || !process.env.AUTOCRAFT_P02_APPROVAL_REPORT) return;
        const answer = await ctx.ui.input("autocraft:approve", JSON.stringify({
            tool: event.toolName, input: event.input,
        }));
        fs.writeFileSync(process.env.AUTOCRAFT_P02_APPROVAL_REPORT, JSON.stringify({
            returnedUndefined: answer === undefined,
        }));
        return { block: true, reason: "P02_APPROVAL_CANCELLED_REASON" };
    });
}
