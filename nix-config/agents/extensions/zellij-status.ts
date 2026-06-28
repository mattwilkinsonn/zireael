import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { basename } from "node:path";

// Reflect agent state in the terminal title, and under Zellij ring a standalone
// BEL on attention moments so the pane gets Zellij's [!] flag. Two gaps this
// fills:
//   - omp's built-in title is a static "pi - session - cwd" (no run state).
//   - omp only appends a BEL under tmux (terminal-capabilities.ts
//     sendNotification); under Zellij it writes the bare OSC with no BEL, so a
//     backgrounded pane never flags that the agent finished or needs you.
// The title is OSC-0, which Zellij captures as the pane title; the BEL is a
// *standalone* \x07 (not the OSC terminator), which is what Zellij turns into
// the [!] flag.

type State = "working" | "ready" | "waiting";

const GLYPH: Record<State, string> = {
	working: "\u27f3", // ⟳
	ready: "\u2713", // ✓
	waiting: "\u26a0", // ⚠
};

const inZellij = Boolean(process.env.ZELLIJ);

function show(pi: ExtensionAPI, ctx: ExtensionContext, state: State, ring: boolean): void {
	// Prefer the session name (auto-titled / user-set) over the cwd — several
	// agents can share a directory, so the dir is a poor per-pane label.
	const label = pi.getSessionName() || basename(ctx.cwd) || ctx.cwd;
	try {
		ctx.ui.setTitle(`${GLYPH[state]} ${state} \u00b7 ${label}`);
	} catch {
		// setTitle no-ops when headless / in RPC mode without PI_RPC_EMIT_TITLE.
	}
	// Only ring on genuine attention transitions, under Zellij, and only when
	// stdout is a real TTY — in RPC/headless mode stdout is the protocol stream,
	// so a stray BEL would corrupt it (the notify extension + omp's own OSC
	// handle desktop alerts elsewhere).
	if (ring && inZellij && process.stdout.isTTY) process.stdout.write("\x07");
}

export default function zellijStatus(pi: ExtensionAPI): void {
	pi.setLabel("zellij-status");
	pi.on("turn_start", (_event, ctx) => show(pi, ctx, "working", false));
	pi.on("input", (_event, ctx) => show(pi, ctx, "working", false));
	pi.on("tool_approval_resolved", (_event, ctx) => show(pi, ctx, "working", false));
	pi.on("session_stop", (_event, ctx) => show(pi, ctx, "ready", true));
	pi.on("tool_approval_requested", (_event, ctx) => show(pi, ctx, "waiting", true));
	pi.on("tool_call", (event, ctx) => {
		if (event.toolName === "ask") show(pi, ctx, "waiting", true);
	});
}
