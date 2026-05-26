import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { cacheFile } from "../platform-config";

export interface Tokens {
	tasks?: string;
	events?: string;
	time_slots?: string;
	labels?: string;
	tags?: string;
	calendars?: string;
	accounts?: string;
	contacts?: string;
	last_full_sync_at?: string;
	user_id?: number;
}

export async function readTokens(): Promise<Tokens> {
	const path = cacheFile("tokens.json");
	if (!existsSync(path)) return {};
	return JSON.parse(await readFile(path, "utf8")) as Tokens;
}

export async function writeTokens(tokens: Tokens): Promise<void> {
	await writeFile(
		cacheFile("tokens.json"),
		JSON.stringify(tokens, null, 2),
		"utf8",
	);
}
