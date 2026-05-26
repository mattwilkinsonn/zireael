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
	// Echo back the upserted task(s) so `af do` sees data to confirm completion
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

describe("af do (BDD — locks current upstream behavior)", () => {
	test("marks task as done via PATCH /v5/tasks", async () => {
		// `af do` expects --ids (per the command's citty surface)
		const result = await spawnCli(["do", "--ids", "task-today-1"], {
			env: env.env,
		});
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
		const firstTask = Array.isArray(body) ? body[0] : body;
		expect(firstTask.done).toBe(true);
	});
});
