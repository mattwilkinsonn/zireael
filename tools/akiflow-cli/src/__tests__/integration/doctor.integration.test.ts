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

describe("af doctor (BDD)", () => {
	test("--json emits a structured report with all top-level keys", async () => {
		const result = await spawnCli(["doctor", "--json"], { env: env.env });
		expect(result.exitCode).toBe(0);
		const report = JSON.parse(result.stdout);
		expect(report).toHaveProperty("credentials");
		expect(report).toHaveProperty("browsers");
		expect(report).toHaveProperty("cache");
		expect(report).toHaveProperty("api");
		expect(report.credentials.has_creds).toBe(true);
		expect(report.credentials.user_id).toBe(42);
		expect(report.api.user_settings_status).toBe(200);
	});

	test("default human output prints sections", async () => {
		const result = await spawnCli(["doctor"], { env: env.env });
		expect(result.exitCode).toBe(0);
		expect(result.stdout).toContain("Credentials");
		expect(result.stdout).toContain("Browser sources");
		expect(result.stdout).toContain("Cache");
		expect(result.stdout).toContain("API health");
	});
});
