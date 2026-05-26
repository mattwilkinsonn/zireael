import { mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

export interface Credentials {
	token: string;
	clientId: string;
	expiryTimestamp: number;
	refreshToken?: string;
}

function getConfigPath(): string {
	// Allow tests/advanced users to override config dir.
	// Default: ~/.config/af
	return process.env.AF_CONFIG_DIR ?? join(homedir(), ".config", "af");
}

function getCredentialsPath(): string {
	return join(getConfigPath(), "credentials.json");
}

async function ensureConfigDirectory(): Promise<void> {
	const configPath = getConfigPath();
	await mkdir(configPath, { recursive: true });
}

export async function saveCredentials(
	token: string,
	clientId?: string,
	expiryTimestamp?: number,
	refreshToken?: string,
): Promise<void> {
	const credentials: Credentials = {
		token,
		clientId: clientId || crypto.randomUUID(),
		expiryTimestamp: expiryTimestamp || Date.now() + 24 * 60 * 60 * 1000,
		refreshToken,
	};

	await ensureConfigDirectory();
	await Bun.write(getCredentialsPath(), JSON.stringify(credentials, null, 2));
}

export async function loadCredentials(): Promise<Credentials | null> {
	const credentialsFile = Bun.file(getCredentialsPath());

	if (!(await credentialsFile.exists())) {
		return null;
	}

	try {
		return (await credentialsFile.json()) as Credentials;
	} catch {
		return null;
	}
}

export async function clearCredentials(): Promise<void> {
	const credentialsPath = getCredentialsPath();
	const credentialsFile = Bun.file(credentialsPath);

	if (await credentialsFile.exists()) {
		await Bun.$`rm ${credentialsPath}`;
	}
}
