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

describe("af refresh (BDD)", () => {
	test("--rebuild --json full-syncs every resource", async () => {
		const result = await spawnCli(["refresh", "--rebuild", "--json"], {
			env: env.env,
		});
		expect(result.exitCode).toBe(0);
		const report = JSON.parse(result.stdout);
		expect(report.mode).toBe("rebuild");
		expect(report.summary).toHaveProperty("tasks");
		// Every resource hit by the cache layer should appear in summary
		for (const res of [
			"tasks",
			"events",
			"time_slots",
			"labels",
			"tags",
			"calendars",
			"accounts",
			"contacts",
		]) {
			expect(report.summary).toHaveProperty(res);
		}
		// Server should have received GETs for each resource
		for (const res of [
			"tasks",
			"events",
			"time_slots",
			"labels",
			"calendars",
		]) {
			expect(
				server.requests.find((r) => r.url.pathname === `/v5/${res}`),
			).toBeDefined();
		}
	});
});
