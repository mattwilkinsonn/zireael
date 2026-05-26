import { describe, expect, test } from "bun:test";
import { spawnCli } from "./spawn-cli";

describe("spawnCli", () => {
	test("captures stdout + exit code from --help", async () => {
		const result = await spawnCli(["--help"]);
		expect(result.exitCode).toBe(0);
		expect(result.stdout.length).toBeGreaterThan(0);
	});

	test("passes env vars through", async () => {
		const result = await spawnCli(["--help"], {
			env: { AF_API_BASE: "http://127.0.0.1:1" },
		});
		expect(result.exitCode).toBe(0);
	});
});
