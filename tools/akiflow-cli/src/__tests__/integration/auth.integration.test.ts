import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { FakeAkiflowServer } from "./helpers/fake-server";
import { spawnCli } from "./helpers/spawn-cli";
import { makeTestEnv } from "./helpers/test-env";

let server: FakeAkiflowServer;
let env: ReturnType<typeof makeTestEnv>;

beforeEach(async () => {
	server = new FakeAkiflowServer();
	await server.start();
	env = makeTestEnv(server.url);
});
afterEach(async () => {
	await server.stop();
	env.cleanup();
});

describe("af auth status (BDD — locks current upstream behavior)", () => {
	test("reports authenticated when credentials.json is present + valid JWT", async () => {
		const result = await spawnCli(["auth", "status"], { env: env.env });
		expect(result.exitCode).toBe(0);
		const out = (result.stdout + result.stderr).toLowerCase();
		expect(
			out.includes("authenticated") ||
				out.includes("logged in") ||
				out.includes("valid"),
		).toBe(true);
	});
});
