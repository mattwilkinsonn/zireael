import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

// Agent-invocable context compaction. Gives the model a `compact_self` tool so
// it can compact at a point IT recognizes as good (task finished + waiting on a
// gate/review, or between tasks) instead of only when the automatic threshold
// fires. Pairs with the idle-compaction backstop in config.yml (compaction.
// idleEnabled) — the tool is the proactive path, idle is the fallback.
//
// The mid-turn-abort nuance: AgentSession.compact() "aborts current agent
// operation first", so calling it synchronously from the tool's execute() would
// abort the very turn that called the tool. Instead the tool only SCHEDULES a
// compaction (sets a flag + returns normally), and a session_stop handler runs
// the compaction once the turn has settled — the boundary where the abort is a
// no-op because the turn is already done. session_stop also fires only after
// the built-in auto-maintenance check has run and released its controller, so
// ctx.compact() here never races "compaction already in progress".

export default function selfCompact(pi: ExtensionAPI): void {
	pi.setLabel("self-compact");
	const { z } = pi.zod;

	// Shared across the tool and the session_stop handler in one closure. A turn
	// either requests compaction (tool sets this) or it doesn't; the boundary
	// handler consumes it. No cross-session state — the extension is per session.
	let pendingCompaction = false;
	// Remembers the model's stated reason so the settle-time log has context.
	let requestedReason: string | undefined;

	pi.registerTool({
		name: "compact_self",
		label: "Compact Context",
		description:
			"Compact your own conversation context to free up tokens. Call this as your FINAL action when you have finished a task and are about to wait on a gate, review, or the next instruction — NEVER mid-task, because it discards older turns (summarizing them) and you would lose working context you still need. The compaction runs after this turn settles, not immediately, so this call returns right away and does not interrupt you. No-op if the session is already small or already compacted.",
		// Read-tier: scheduling a post-turn maintenance pass mutates nothing the
		// model can observe this turn; it must never require approval.
		approval: "read",
		parameters: z.object({
			reason: z
				.string()
				.optional()
				.describe(
					"Optional short note on why now is a good compaction point (e.g. 'finished PR, waiting on review').",
				),
		}),
		async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
			pendingCompaction = true;
			requestedReason = params.reason;
			const usage = ctx.getContextUsage();
			const now = usage ? `${Math.round(usage.tokens / 1000)}K` : "unknown";
			pi.logger.debug("compact_self scheduled", {
				tokens: usage?.tokens,
				reason: params.reason,
			});
			return {
				content: [
					{
						type: "text",
						text: `Compaction scheduled — it will run when this turn settles (context is ~${now} tokens now). This does not interrupt the current turn.`,
					},
				],
				details: {
					scheduled: true,
					tokens: usage?.tokens,
					reason: params.reason,
				},
			};
		},
	});

	// Turn is settling — the safe point to compact. Fires after the built-in
	// threshold/idle maintenance has already had its chance this turn.
	pi.on("session_stop", async (event, ctx: ExtensionContext) => {
		if (!pendingCompaction) return;
		// Clear before awaiting so a thrown compact() can't wedge the flag on and
		// re-fire every settle thereafter.
		pendingCompaction = false;
		const reason = requestedReason;
		requestedReason = undefined;

		// A continuation is already in flight (this stop is itself driving one, or
		// a queued message will own the next turn). Compacting now would race that
		// turn's context rebuild — defer by re-arming; the next clean settle runs it.
		if (event.stop_hook_active || ctx.hasPendingMessages()) {
			pendingCompaction = true;
			requestedReason = reason;
			return;
		}

		try {
			await ctx.compact();
			pi.logger.debug("compact_self ran at turn settle", { reason });
		} catch (err) {
			// "Nothing to compact (session too small)" / "Already compacted" are
			// benign — the goal state (small context) already holds. Anything else
			// is worth a warning but must not crash the settle path.
			const message = err instanceof Error ? err.message : String(err);
			if (/nothing to compact|already compacted/i.test(message)) {
				pi.logger.debug("compact_self no-op", { message });
			} else {
				pi.logger.warn("compact_self failed", { message });
			}
		}
	});
}
