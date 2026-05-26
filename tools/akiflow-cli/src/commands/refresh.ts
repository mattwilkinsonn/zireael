import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import { rebuild, refresh } from "../lib/cache";

export const refreshCommand = defineCommand({
	meta: {
		name: "refresh",
		description:
			"Sync local cache from Akiflow API (delta by default; --rebuild for full)",
	},
	args: {
		rebuild: {
			type: "boolean",
			description: "Delete cache and full-sync from scratch",
		},
		json: {
			type: "boolean",
			description: "Output sync summary as JSON",
		},
	},
	run: async ({ args }) => {
		const client = createClient();
		const start = Date.now();
		const summary = args.rebuild
			? await rebuild(client)
			: await refresh(client);
		const elapsed = Date.now() - start;
		if (args.json) {
			console.log(
				JSON.stringify(
					{
						mode: args.rebuild ? "rebuild" : "delta",
						elapsed_ms: elapsed,
						summary,
					},
					null,
					2,
				),
			);
			return;
		}
		console.log(
			`Cache ${args.rebuild ? "rebuilt" : "refreshed"} in ${elapsed}ms`,
		);
		for (const [res, info] of Object.entries(summary)) {
			const u = (info as { upserted: number }).upserted ?? 0;
			const t = (info as { tombstones?: number }).tombstones ?? 0;
			console.log(
				`  ${res.padEnd(15)} +${u} upserts${t > 0 ? `, -${t} tombstones` : ""}`,
			);
		}
	},
});
