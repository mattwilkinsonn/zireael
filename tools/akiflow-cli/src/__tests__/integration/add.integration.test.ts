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
	// Echo back the upserted task(s) so `af add` sees data to confirm creation
	server.respondTo("PATCH", "/v5/tasks", ({ body }: { body: string }) => {
		const upserts = JSON.parse(body) as Array<Record<string, unknown>>;
		return { success: true, message: null, data: upserts };
	});
	env = makeTestEnv(server.url);
});
afterEach(async () => {
	await server.stop();
	env.cleanup();
});

describe("af add (BDD — locks current upstream behavior)", () => {
	test("creates a task via PATCH /v5/tasks with the given title", async () => {
		const result = await spawnCli(["add", "Test new task"], { env: env.env });
		if (result.exitCode !== 0) {
			console.error("STDOUT:", result.stdout);
			console.error("STDERR:", result.stderr);
		}
		expect(result.exitCode).toBe(0);

		const patchReq = server.requests.find(
			(r) => r.method === "PATCH" && r.url.pathname === "/v5/tasks",
		);
		expect(patchReq).toBeDefined();
		const body = JSON.parse(patchReq!.body);
		// PATCH body shape from upstream: array of task payloads
		expect(Array.isArray(body) || (body && typeof body === "object")).toBe(
			true,
		);
		const firstTask = Array.isArray(body) ? body[0] : body;
		expect(firstTask.title).toBe("Test new task");
	});
});
