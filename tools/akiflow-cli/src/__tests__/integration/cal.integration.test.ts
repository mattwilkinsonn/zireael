import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { FakeAkiflowServer } from "./helpers/fake-server";
import { loadAllFixtures } from "./helpers/load-fixtures";
import { spawnCli } from "./helpers/spawn-cli";
import { makeTestEnv } from "./helpers/test-env";

let server: FakeAkiflowServer;
let env: ReturnType<typeof makeTestEnv>;

beforeEach(async () => {
	server = new FakeAkiflowServer();
	await server.start();
	loadAllFixtures(server);
	env = makeTestEnv(server.url);
});
afterEach(async () => {
	await server.stop();
	env.cleanup();
});

describe("af cal (BDD — new merged-timeline mode)", () => {
	test("--this-week --json emits cleaned merged timeline including the fixture event", async () => {
		// The fixture event lives at a fixed date (2026-05-21). Use a wide
		// enough range that the test isn't day-of-week-sensitive.
		const result = await spawnCli(
			["cal", "--from", "2026-05-18", "--to", "2026-05-24", "--json"],
			{
				env: env.env,
			},
		);
		expect(result.exitCode).toBe(0);
		const report = JSON.parse(result.stdout);
		expect(report).toHaveProperty("result");
		expect(Array.isArray(report.result)).toBe(true);
		const event = report.result.find(
			(e: { id: string }) => e.id === "event-meeting-1",
		);
		expect(event).toBeDefined();
		expect(event.type).toBe("event");
		// Cleaned shape nests meeting under a single object
		expect(event.meeting.url).toBe("https://meet.google.com/abc-defg-hij");
		expect(event.meeting.solution).toBe("google_meet");
	});

	test("--raw emits full record envelope", async () => {
		const result = await spawnCli(
			["cal", "--from", "2026-05-18", "--to", "2026-05-24", "--raw"],
			{
				env: env.env,
			},
		);
		expect(result.exitCode).toBe(0);
		const report = JSON.parse(result.stdout);
		expect(report).toHaveProperty("result");
		expect(report).toHaveProperty("next_cursor");
	});

	test("--no-events excludes events from JSON output", async () => {
		const result = await spawnCli(
			[
				"cal",
				"--from",
				"2026-05-18",
				"--to",
				"2026-05-24",
				"--no-events",
				"--json",
			],
			{ env: env.env },
		);
		expect(result.exitCode).toBe(0);
		const report = JSON.parse(result.stdout);
		const eventTypes = report.result.map((e: { type: string }) => e.type);
		expect(eventTypes).not.toContain("event");
	});
});
