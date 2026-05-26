import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, statSync } from "node:fs";
import { makeTestEnv } from "./test-env";

describe("makeTestEnv", () => {
	test("creates isolated cache + config dirs with fake creds", () => {
		const env = makeTestEnv("http://127.0.0.1:0");
		try {
			expect(existsSync(env.cacheDir)).toBe(true);
			expect(existsSync(env.configDir)).toBe(true);
			expect(existsSync(env.credentialsPath)).toBe(true);

			const creds = JSON.parse(readFileSync(env.credentialsPath, "utf8")) as {
				token: string;
				clientId: string;
				refreshToken: string;
			};
			expect(creds.token).toMatch(
				/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/,
			);
			expect(creds.refreshToken).toBe("fake-refresh");

			// 0600 perms
			const mode = statSync(env.credentialsPath).mode & 0o777;
			expect(mode).toBe(0o600);

			// Env vars include the api base + cache/config dirs
			expect(env.env.AF_API_BASE).toBe("http://127.0.0.1:0");
			expect(env.env.AF_CACHE_DIR).toBe(env.cacheDir);
			expect(env.env.AF_CONFIG_DIR?.endsWith("/af") ?? false).toBe(true);
		} finally {
			env.cleanup();
			expect(existsSync(env.cacheDir)).toBe(false);
		}
	});

	test("two envs are isolated from each other", () => {
		const a = makeTestEnv("http://a");
		const b = makeTestEnv("http://b");
		try {
			expect(a.cacheDir).not.toBe(b.cacheDir);
			expect(a.configDir).not.toBe(b.configDir);
		} finally {
			a.cleanup();
			b.cleanup();
		}
	});
});
