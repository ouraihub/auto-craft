// DSH native pre-dispatch policy: every non-read tool reaches the host approval seam.
export const name = "autocraft-builder-approval";
export const inject = ["tools", "approval"];
export function apply(ctx) {
    ctx.on("tools/pre-execute", async (exec, next) => {
        if (["read", "read_image", "grep", "glob"].includes(exec.name)
            || exec.name.startsWith("mcp__autocraft__")) return next();
        const previous = await next();
        if (previous.kind !== "allow") return previous;
        return { kind: "ask", reason: "autocraft project action policy" };
    });
}
