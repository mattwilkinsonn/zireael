import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export interface TestEnv {
	cacheDir: string;
	configDir: string;
	credentialsPath: string;
	/** Env vars to pass to spawnCli. */
	env: Record<string, string>;
	cleanup(): void;
}

/**
 * Build an isolated test environment: temp cache dir + temp config dir
 * with a fake JWT credentials file. JWT payload has user_id=42 and
 * exp far in the future. Auth flow short-circuits on a present
 * credentials.json — no browser extraction triggered.
 */
export function makeTestEnv(apiBase: string): TestEnv {
	const cacheDir = mkdtempSync(join(tmpdir(), "af-bdd-cache-"));
	const configDir = mkdtempSync(join(tmpdir(), "af-bdd-config-"));
	const afConfigDir = join(configDir, "af");
	mkdirSync(afConfigDir, { recursive: true });
	const credentialsPath = join(afConfigDir, "credentials.json");

	// Fake JWT: header.payload.signature. exp=4102444800 = 2099-12-31.
	const header = Buffer.from(
		JSON.stringify({ alg: "HS256", typ: "JWT" }),
	).toString("base64url");
	const payload = Buffer.from(
		JSON.stringify({ user_id: 42, exp: 4102444800 }),
	).toString("base64url");
	const fakeJwt = `${header}.${payload}.signature`;
	writeFileSync(
		credentialsPath,
		JSON.stringify({
			token: fakeJwt,
			clientId: "test-client",
			// Unix ms, year 2099 — matches the Credentials interface in storage.ts
			expiryTimestamp: 4102444800000,
			refreshToken: "fake-refresh",
		}),
		{ mode: 0o600 },
	);

	return {
		cacheDir,
		configDir,
		credentialsPath,
		env: {
			AF_API_BASE: apiBase,
			AF_CACHE_DIR: cacheDir,
			AF_CONFIG_DIR: afConfigDir,
			AF_NO_AUTO_SYNC: "1",
		},
		cleanup: () => {
			rmSync(cacheDir, { recursive: true, force: true });
			rmSync(configDir, { recursive: true, force: true });
		},
	};
}
