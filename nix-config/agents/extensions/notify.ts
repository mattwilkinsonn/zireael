import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { basename } from "node:path";

// Native macOS desktop notifications for omp. Under Zellij, omp's OSC 9/777
// notifications are consumed by the multiplexer and never reach Ghostty (zellij
// only forwards a BEL), so we bypass the terminal-escape path entirely and fire
// terminal-notifier (falling back to osascript when it's not installed yet).
// Fires on turn completion and when omp is blocked waiting on you.

const GHOSTTY_BUNDLE = "com.mitchellh.ghostty"; // toast wears Ghostty's icon; click focuses it

function notify(ctx: ExtensionContext, message: string): void {
	if (process.platform !== "darwin") return; // terminal-notifier/osascript are macOS-only
	const title = basename(ctx.cwd) || "Oh My Pi";
	const opts = { stdout: "ignore", stderr: "ignore", stdin: "ignore" } as const;
	try {
		const tn = Bun.which("terminal-notifier");
		if (tn) {
			Bun.spawn(
				[tn, "-title", title, "-message", message, "-sender", GHOSTTY_BUNDLE, "-group", `omp-${title}`],
				opts,
			).unref();
			return;
		}
		// Fallback: osascript is always present. No -sender/click-to-focus; strip
		// quotes/backslashes so the inlined AppleScript string can't break.
		const safe = (s: string): string => s.replace(/["\\]/g, "");
		Bun.spawn(
			["osascript", "-e", `display notification "${safe(message)}" with title "${safe(title)}"`],
			opts,
		).unref();
	} catch {
		// notifier missing or spawn failed — silently skip.
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
