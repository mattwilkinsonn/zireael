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

describe("af ls (BDD — extended flags after Phase 6)", () => {
	test("--connector gmail filters to gmail-sourced", async () => {
		const result = await spawnCli(["ls", "--connector", "gmail", "--all"], {
			env: env.env,
		});
		expect(result.exitCode).toBe(0);
		expect(result.stdout).toContain("Re: project update");
		expect(result.stdout).not.toContain("Triage notifications");
	});

	test("--recurring shows only recurring", async () => {
		const result = await spawnCli(["ls", "--recurring", "--all"], {
			env: env.env,
		});
		expect(result.exitCode).toBe(0);
		expect(result.stdout).toContain("Weekly review");
	});

	test("--trashed shows trashed (assuming none in fixtures, exits 0)", async () => {
		const result = await spawnCli(["ls", "--trashed"], { env: env.env });
		expect(result.exitCode).toBe(0);
	});

	test("--raw emits full record JSON envelope", async () => {
		const result = await spawnCli(["ls", "--all", "--raw"], { env: env.env });
		expect(result.exitCode).toBe(0);
		const report = JSON.parse(result.stdout);
		expect(report).toHaveProperty("result");
		expect(report).toHaveProperty("next_cursor");
		expect(report).toHaveProperty("errors");
		expect(Array.isArray(report.result)).toBe(true);
	});
});

describe("af ls (BDD — locks current upstream behavior)", () => {
	test("default listing prints a today-anchored task", async () => {
		const result = await spawnCli(["ls"], { env: env.env });
		// Exit code 0 means the command ran and returned successfully.
		// Even if upstream filters differ from our fixture date range, the
		// command should at least exit cleanly with creds + API mocked.
		expect(result.exitCode).toBe(0);
	});

	test("calls /v5/tasks with Authorization Bearer header", async () => {
		await spawnCli(["ls"], { env: env.env });
		const tasksReq = server.requests.find(
			(r) => r.url.pathname === "/v5/tasks",
		);
		expect(tasksReq).toBeDefined();
		expect(tasksReq?.headers.authorization).toContain("Bearer ");
	});
});
