import { existsSync, statSync } from "node:fs";
import { defineCommand } from "citty";
import { createClient } from "../lib/api/client";
import { loadCredentials } from "../lib/auth/storage";
import { type BrowserType, getAllBrowserPaths } from "../lib/browser-paths";
import { readAllRecords } from "../lib/cache/jsonl-store";
import { readTokens } from "../lib/cache/tokens";
import { cacheFile, cachePath } from "../lib/platform-config";

interface BrowserReport {
	name: BrowserType;
	detected: boolean;
	path?: string;
}

interface DoctorReport {
	credentials: {
		has_creds: boolean;
		user_id: number | null;
		expires_at: string | null;
		has_refresh_token: boolean;
	};
	browsers: BrowserReport[];
	cache: {
		path: string;
		exists: boolean;
		last_full_sync_at: string | null;
		resources: Record<string, { records: number; mtime: string | null }>;
	};
	api: {
		user_settings_status: number | null;
		elapsed_ms: number | null;
		error: string | null;
	};
}

const RESOURCES = [
	"tasks",
	"events",
	"time_slots",
	"labels",
	"tags",
	"calendars",
	"accounts",
	"contacts",
] as const;

export async function runDoctor(opts: {
	json?: boolean;
}): Promise<DoctorReport> {
	const report: DoctorReport = {
		credentials: await checkCredentials(),
		browsers: checkBrowsers(),
		cache: await checkCache(),
		api: await checkApi(),
	};
	if (!opts.json) printReport(report);
	else console.log(JSON.stringify(report, null, 2));
	return report;
}

async function checkCredentials(): Promise<DoctorReport["credentials"]> {
	const creds = await loadCredentials();
	if (!creds) {
		return {
			has_creds: false,
			user_id: null,
			expires_at: null,
			has_refresh_token: false,
		};
	}
	const payload = decodeJwt(creds.token);
	return {
		has_creds: true,
		user_id: payload?.user_id ?? null,
		expires_at: creds.expiryTimestamp
			? new Date(creds.expiryTimestamp).toISOString()
			: null,
		has_refresh_token: !!creds.refreshToken,
	};
}

function decodeJwt(token: string): { user_id?: number; exp?: number } | null {
	try {
		const parts = token.split(".");
		if (parts.length < 2 || !parts[1]) return null;
		return JSON.parse(Buffer.from(parts[1], "base64").toString("utf8")) as {
			user_id?: number;
			exp?: number;
		};
	} catch {
		return null;
	}
}

function checkBrowsers(): BrowserReport[] {
	const paths = getAllBrowserPaths();
	return paths.map((b) => ({
		name: b.id,
		detected: existsSync(b.cookiePath),
		path: existsSync(b.cookiePath) ? b.cookiePath : undefined,
	}));
}

async function checkCache(): Promise<DoctorReport["cache"]> {
	const path = cachePath();
	const exists = existsSync(path);
	const tokens = exists ? await readTokens() : {};
	const resources: Record<string, { records: number; mtime: string | null }> =
		{};
	for (const res of RESOURCES) {
		const file = cacheFile(`${res}.jsonl`);
		if (existsSync(file)) {
			const records = await readAllRecords<unknown>(file);
			resources[res] = {
				records: records.length,
				mtime: statSync(file).mtime.toISOString(),
			};
		} else {
			resources[res] = { records: 0, mtime: null };
		}
	}
	return {
		path,
		exists,
		last_full_sync_at: tokens.last_full_sync_at ?? null,
		resources,
	};
}

async function checkApi(): Promise<DoctorReport["api"]> {
	try {
		const client = createClient();
		const start = Date.now();
		const resp = await client.get<unknown>("/v5/user/settings", {});
		return {
			user_settings_status: resp.success ? 200 : 500,
			elapsed_ms: Date.now() - start,
			error: null,
		};
	} catch (err) {
		return {
			user_settings_status: null,
			elapsed_ms: null,
			error: (err as Error).message,
		};
	}
}

function printReport(r: DoctorReport): void {
	const line = (s: string): void => {
		console.log(s);
	};
	line(`af doctor — diagnostic report (${new Date().toISOString()})\n`);

	line("Credentials");
	line(
		`  credentials.json                  ${r.credentials.has_creds ? "✓ present" : "✗ MISSING"}`,
	);
	if (r.credentials.user_id != null)
		line(`  JWT user_id                       ${r.credentials.user_id}`);
	if (r.credentials.expires_at)
		line(`  JWT expires_at                    ${r.credentials.expires_at}`);
	line(
		`  Refresh token                     ${r.credentials.has_refresh_token ? "✓ present" : "✗ missing"}\n`,
	);

	line("Browser sources");
	for (const b of r.browsers) {
		line(
			`  ${b.name.padEnd(30)} ${b.detected ? "✓ detected" : "✗ not installed"}`,
		);
	}
	line("");

	line("Cache");
	line(
		`  ${r.cache.path.padEnd(30)}    ${r.cache.exists ? "✓ exists" : "✗ MISSING"}`,
	);
	if (r.cache.last_full_sync_at)
		line(`  Last full sync                    ${r.cache.last_full_sync_at}`);
	for (const [res, info] of Object.entries(r.cache.resources)) {
		line(
			`  ${res.padEnd(30)} ${String(info.records).padStart(5)} records${info.mtime ? `, ${info.mtime}` : ""}`,
		);
	}
	line("");

	line("API health");
	if (r.api.error) {
		line(`  GET /v5/user/settings             ✗ ${r.api.error}`);
	} else {
		line(
			`  GET /v5/user/settings             ✓ ${r.api.user_settings_status} (${r.api.elapsed_ms}ms)`,
		);
	}
}

export const doctorCommand = defineCommand({
	meta: {
		name: "doctor",
		description:
			"Diagnostic report on credentials, browsers, cache, and API health",
	},
	args: {
		json: {
			type: "boolean",
			description: "Output as JSON instead of human-readable text",
		},
	},
	run: async ({ args }) => {
		await runDoctor({ json: args.json });
	},
});
