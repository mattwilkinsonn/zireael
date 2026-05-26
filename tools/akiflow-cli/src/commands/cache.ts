import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import { syncTasksCache } from "../lib/tasks-local-cache";

export const cacheRefreshCommand = defineCommand({
	meta: {
		name: "refresh",
		description: "Refresh local tasks cache (full resync)",
	},
	run: async () => {
		const client = createClient();

		try {
			const { meta } = await syncTasksCache(client, {
				forceFull: true,
				quiet: true,
			});
			console.log(`✓ Cache refreshed (${meta.taskCount} tasks)`);
		} catch (error) {
			if (error instanceof Error) {
				console.error(`Error: ${error.message}`);
			} else {
				console.error("Unknown error occurred");
			}
			process.exit(1);
		}
	},
});

export const cacheCommand = defineCommand({
	meta: {
		name: "cache",
		description: "Local cache management",
	},
	subCommands: {
		refresh: cacheRefreshCommand,
	},
	run: async () => {
		console.log("Cache subcommands:");
		console.log("  refresh - Full resync of local task cache");
	},
});
