import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { FakeAkiflowServer } from "./fake-server";

const FIXTURES_DIR = join(import.meta.dir, "..", "fixtures");

const RESOURCES: Array<{ resource: string; file: string }> = [
	{ resource: "tasks", file: "tasks.json" },
	{ resource: "events", file: "events.json" },
	{ resource: "time_slots", file: "time-slots.json" },
	{ resource: "labels", file: "labels.json" },
	{ resource: "tags", file: "tags.json" },
	{ resource: "calendars", file: "calendars.json" },
	{ resource: "accounts", file: "accounts.json" },
	{ resource: "contacts", file: "contacts.json" },
];

/**
 * Populate the fake server with canned responses for every v5 resource.
 * Each endpoint returns the fixture data as a single page
 * (has_next_page=false). Tests can override individual endpoints by
 * calling server.respondTo() after this.
 */
export function loadAllFixtures(
	server: FakeAkiflowServer,
	syncToken: string = "test-token-1",
): void {
	for (const { resource, file } of RESOURCES) {
		let data: unknown[] = [];
		try {
			data = JSON.parse(readFileSync(join(FIXTURES_DIR, file), "utf8"));
		} catch {
			data = [];
		}
		server.respondTo("GET", `/v5/${resource}`, {
			success: true,
			message: null,
			data,
			sync_token: syncToken,
			has_next_page: false,
		});
	}
	// Settings + client registration — minimal canned responses
	server.respondTo("GET", "/v5/user/settings", {
		success: true,
		message: null,
		data: { timezone: "America/New_York" },
	});
	server.respondTo("POST", "/v5/clients", {
		success: true,
		message: null,
		data: { id: "client-1" },
	});
}
