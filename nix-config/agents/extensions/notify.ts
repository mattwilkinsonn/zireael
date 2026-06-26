import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { basename } from "node:path";

// Native macOS desktop notifications for omp. Under Zellij, omp's OSC 9/777
// notifications are consumed by the multiplexer and never reach Ghostty, so we
// post natively via terminal-notifier — installed native arm64 through Homebrew
// (the nixpkgs build is the upstream x86 .app and fails to post under Rosetta
// on Apple Silicon). Fires on turn completion and when omp is blocked on you.

const GHOSTTY_BUNDLE = "com.mitchellh.ghostty";

function notify(ctx: ExtensionContext, message: string): void {
	if (process.platform !== "darwin") return; // terminal-notifier is macOS-only
	const tn = Bun.which("terminal-notifier");
	if (!tn) return; // not installed yet (pre nix-switch) — skip silently
	const title = basename(ctx.cwd) || "Oh My Pi";
	try {
		Bun.spawn(
			[tn, "-title", title, "-message", message, "-sender", GHOSTTY_BUNDLE],
			{ stdout: "ignore", stderr: "ignore", stdin: "ignore" },
		).unref();
	} catch {
		// spawn failed — silently skip.
	}
}

export default function notifier(pi: ExtensionAPI): void {
	pi.setLabel("notify");

	// Turn settled — your turn. Skip aborts/errors (match omp's own gating).
	pi.on("session_stop", (event, ctx) => {
		const m = event.last_assistant_message;
		const stopReason = m && typeof m === "object" && "stopReason" in m ? String(m.stopReason) : "";
		if (stopReason === "aborted" || stopReason === "error") return;
		notify(ctx, "Done — your turn");
	});

	// omp is blocked on you: a tool needs approval, or it asked a question.
	pi.on("tool_approval_requested", (event, ctx) => {
		notify(ctx, `Needs approval: ${event.toolName}`);
	});
	pi.on("tool_call", (event, ctx) => {
		if (event.toolName === "ask") notify(ctx, "Waiting for your input");
	});
}
