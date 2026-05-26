/// <reference types="bun" />
import { Database } from "bun:sqlite";
import {
	copyFileSync,
	existsSync,
	readdirSync,
	readFileSync,
	statSync,
	unlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
	type BrowserPath,
	type BrowserType,
	getAllBrowserPaths,
} from "../browser-paths";
import { createLogger } from "../log";

const log = createLogger({ module: "auth.extract", file: "auth.log" });

export interface ExtractedToken {
	browser: BrowserType;
	token: string;
	source: string;
	expiresAt?: Date;
	refreshToken?: string;
}

const AKIFLOW_INDEXEDDB_FOLDER = "https_web.akiflow.com_0.indexeddb.leveldb";
// Match any JWT (both HS256 and RS256)
const JWT_PATTERN = /eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g;
const JWT_MATCHER = /eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/;
const REFRESH_TOKEN_PATTERN = /def50200[a-f0-9]{200,}/g;

/**
 * Chrome encryption constants
 */
const CHROME_SALT = "saltysalt";
const CHROME_ITERATIONS = 1003;
const CHROME_KEY_LENGTH = 16; // 128-bit key for AES-128

/**
 * Akiflow domain to search for cookies
 */
const AKIFLOW_DOMAIN = "akiflow.com";

/**
 * Cookie name pattern for Akiflow auth
 */
const COOKIE_NAME_PATTERN = "remember_web_";

/**
 * Get Chrome Safe Storage password from macOS Keychain
 * @param browserName - Name of the browser in keychain
 * @returns Password string or null if not found
 */
export async function getKeychainPassword(
	browserName: string,
): Promise<string | null> {
	const serviceNames: Record<string, string> = {
		chrome: "Chrome Safe Storage",
		arc: "Arc Safe Storage",
		brave: "Brave Safe Storage",
		edge: "Microsoft Edge Safe Storage",
	};

	const serviceName = serviceNames[browserName];
	if (!serviceName) {
		log.warn("keychain_service_name", "unknown browser for keychain lookup", {
			browser: browserName,
		});
		return null;
	}

	const proc = Bun.spawn(
		["security", "find-generic-password", "-w", "-s", serviceName],
		{
			stdout: "pipe",
			stderr: "pipe",
		},
	);

	const output = await new Response(proc.stdout).text();
	await proc.exited;

	if (proc.exitCode !== 0) {
		log.warn("keychain_missing", "no keychain entry for browser safe-storage", {
			browser: browserName,
			service: serviceName,
			exit_code: proc.exitCode,
		});
		return null;
	}

	return output.trim();
}

/**
 * Derive AES key from keychain password using PBKDF2
 * @param password - Password from keychain
 * @returns AES key as Uint8Array
 */
export async function deriveKey(password: string): Promise<Uint8Array> {
	const encoder = new TextEncoder();
	const keyMaterial = await crypto.subtle.importKey(
		"raw",
		encoder.encode(password),
		"PBKDF2",
		false,
		["deriveBits"],
	);

	const derivedBits = await crypto.subtle.deriveBits(
		{
			name: "PBKDF2",
			salt: encoder.encode(CHROME_SALT),
			iterations: CHROME_ITERATIONS,
			hash: "SHA-1",
		},
		keyMaterial,
		CHROME_KEY_LENGTH * 8,
	);

	return new Uint8Array(derivedBits);
}

/**
 * Decrypt Chrome-encrypted cookie value
 * @param encryptedValue - Encrypted cookie value (with v10/v11 prefix)
 * @param key - Derived AES key
 * @returns Decrypted value string or null if decryption fails
 */
export async function decryptChromeValue(
	encryptedValue: Uint8Array,
	key: Uint8Array,
): Promise<string | null> {
	// Check for v10 prefix (macOS Chrome encryption)
	if (encryptedValue.length < 3) {
		return null;
	}

	const prefix = String.fromCharCode(
		encryptedValue[0] ?? 0,
		encryptedValue[1] ?? 0,
		encryptedValue[2] ?? 0,
	);

	if (prefix !== "v10" && prefix !== "v11") {
		// Not encrypted, return as string
		return new TextDecoder().decode(encryptedValue);
	}

	// Skip v10 prefix, get IV (16 bytes, but Chrome uses empty IV)
	const ciphertext = encryptedValue.slice(3);

	// Chrome on macOS uses AES-128-CBC with a 16-byte IV of spaces (0x20)
	const iv = new Uint8Array(16).fill(0x20);

	const cryptoKey = await crypto.subtle.importKey(
		"raw",
		key.buffer as ArrayBuffer,
		{ name: "AES-CBC" },
		false,
		["decrypt"],
	);

	const decrypted = await crypto.subtle.decrypt(
		{ name: "AES-CBC", iv },
		cryptoKey,
		ciphertext,
	);

	// Remove PKCS7 padding
	const decryptedBytes = new Uint8Array(decrypted);
	const paddingLength = decryptedBytes[decryptedBytes.length - 1] ?? 0;
	const unpaddedBytes = decryptedBytes.slice(
		0,
		decryptedBytes.length - paddingLength,
	);

	return new TextDecoder().decode(unpaddedBytes);
}

/**
 * Extract cookies from Chrome-family browser SQLite database
 * @param browserPath - Browser path configuration
 * @returns Array of extracted tokens
 */
export async function extractFromChrome(
	browserPath: BrowserPath,
): Promise<ExtractedToken[]> {
	const tokens: ExtractedToken[] = [];

	if (!existsSync(browserPath.cookiePath)) {
		return tokens;
	}

	// Get keychain password
	const password = await getKeychainPassword(browserPath.id);
	if (!password) {
		return tokens;
	}

	// Derive encryption key
	const key = await deriveKey(password);

	// Copy database to temp location (browser may have lock on it)
	const tempPath = join(tmpdir(), `${browserPath.id}_cookies_${Date.now()}.db`);

	copyFileSync(browserPath.cookiePath, tempPath);

	const database = new Database(tempPath, { readonly: true });

	// Query for Akiflow cookies
	const query = database.query(`
    SELECT name, value, encrypted_value, host_key
    FROM cookies
    WHERE host_key LIKE $domain
      AND name LIKE $pattern
  `);

	const rows = query.all({
		$domain: `%${AKIFLOW_DOMAIN}%`,
		$pattern: `${COOKIE_NAME_PATTERN}%`,
	}) as Array<{
		name: string;
		value: string;
		encrypted_value: Uint8Array;
		host_key: string;
	}>;

	for (const row of rows) {
		let tokenValue: string | null = null;

		// Try to get value from encrypted_value first
		if (row.encrypted_value && row.encrypted_value.length > 0) {
			tokenValue = await decryptChromeValue(row.encrypted_value, key);
		}

		// Fall back to plain value if available
		if (!tokenValue && row.value) {
			tokenValue = row.value;
		}

		if (tokenValue) {
			const jwt = isJwtToken(tokenValue)
				? tokenValue
				: extractJwtFromString(tokenValue);
			if (!jwt) {
				continue;
			}

			tokens.push({
				browser: browserPath.id,
				token: jwt,
				source: browserPath.cookiePath,
				expiresAt: parseJWTExpiration(jwt),
			});
		}
	}

	database.close();

	// Cleanup temp file
	unlinkSync(tempPath);

	return tokens;
}

/**
 * Safari binary cookies format parser
 *
 * Binary format structure:
 * - Header: "cook" (4 bytes)
 * - Number of pages: 4 bytes (big-endian)
 * - Page sizes: 4 bytes each (big-endian)
 * - Pages: variable length
 *
 * Each page:
 * - Page header: 4 bytes (0x00000100)
 * - Number of cookies: 4 bytes (little-endian)
 * - Cookie offsets: 4 bytes each (little-endian)
 * - Cookie data
 */
export async function extractFromSafari(
	browserPath: BrowserPath,
): Promise<ExtractedToken[]> {
	const tokens: ExtractedToken[] = [];

	if (!existsSync(browserPath.cookiePath)) {
		return tokens;
	}

	const file = Bun.file(browserPath.cookiePath);
	const buffer = await file.arrayBuffer();
	const data = new DataView(buffer);
	const bytes = new Uint8Array(buffer);

	// Verify magic "cook"
	const magic = String.fromCharCode(
		bytes[0] ?? 0,
		bytes[1] ?? 0,
		bytes[2] ?? 0,
		bytes[3] ?? 0,
	);
	if (magic !== "cook") {
		return tokens;
	}

	// Number of pages (big-endian)
	const numPages = data.getUint32(4, false);

	// Read page sizes
	const pageSizes: number[] = [];
	let offset = 8;
	for (let i = 0; i < numPages; i++) {
		pageSizes.push(data.getUint32(offset, false));
		offset += 4;
	}

	// Process each page
	for (let pageIndex = 0; pageIndex < numPages; pageIndex++) {
		const pageSize = pageSizes[pageIndex];
		if (!pageSize) continue;

		const pageStart = offset;
		const pageData = new DataView(buffer, pageStart, pageSize);

		// Page header should be 0x00000100
		const pageHeader = pageData.getUint32(0, true);
		if (pageHeader !== 0x00000100) {
			offset += pageSize;
			continue;
		}

		// Number of cookies (little-endian)
		const numCookies = pageData.getUint32(4, true);

		// Read cookie offsets
		const cookieOffsets: number[] = [];
		let cookieOffsetPos = 8;
		for (let i = 0; i < numCookies; i++) {
			cookieOffsets.push(pageData.getUint32(cookieOffsetPos, true));
			cookieOffsetPos += 4;
		}

		// Parse each cookie
		for (const cookieOffset of cookieOffsets) {
			const cookie = parseSafariCookie(pageData, cookieOffset, pageSize);
			if (
				cookie?.domain.includes(AKIFLOW_DOMAIN) &&
				cookie.name.startsWith(COOKIE_NAME_PATTERN)
			) {
				const jwt = isJwtToken(cookie.value)
					? cookie.value
					: extractJwtFromString(cookie.value);
				if (!jwt) {
					continue;
				}

				tokens.push({
					browser: "safari",
					token: jwt,
					source: browserPath.cookiePath,
					expiresAt: parseJWTExpiration(jwt),
				});
			}
		}

		offset += pageSize;
	}

	return tokens;
}

interface SafariCookie {
	name: string;
	value: string;
	domain: string;
	path: string;
}

/**
 * Parse a single Safari cookie from binary data
 */
function parseSafariCookie(
	pageData: DataView,
	cookieOffset: number,
	pageSize: number,
): SafariCookie | null {
	if (cookieOffset + 48 > pageSize) {
		return null;
	}

	// Cookie structure:
	// 0-3: cookie size (little-endian)
	// 4-7: unknown
	// 8-11: flags
	// 12-15: unknown
	// 16-19: domain offset (little-endian)
	// 20-23: name offset (little-endian)
	// 24-27: path offset (little-endian)
	// 28-31: value offset (little-endian)
	// 32-39: end of cookie (8 bytes)
	// 40-47: expiration date (8 bytes)
	// 48-55: creation date (8 bytes)

	const cookieSize = pageData.getUint32(cookieOffset, true);
	if (cookieOffset + cookieSize > pageSize) {
		return null;
	}

	const domainOffset = pageData.getUint32(cookieOffset + 16, true);
	const nameOffset = pageData.getUint32(cookieOffset + 20, true);
	const pathOffset = pageData.getUint32(cookieOffset + 24, true);
	const valueOffset = pageData.getUint32(cookieOffset + 28, true);

	const readString = (stringOffset: number): string => {
		const bytes = new Uint8Array(
			pageData.buffer,
			pageData.byteOffset + cookieOffset + stringOffset,
		);
		let end = 0;
		while (end < bytes.length && bytes[end] !== 0) {
			end++;
		}
		return new TextDecoder().decode(bytes.slice(0, end));
	};

	return {
		domain: readString(domainOffset),
		name: readString(nameOffset),
		path: readString(pathOffset),
		value: readString(valueOffset),
	};
}

export async function scanBrowsers(): Promise<ExtractedToken[]> {
	const browserPaths = getAllBrowserPaths();
	const now = new Date();

	const isValidToken = (token: ExtractedToken): boolean => {
		if (!isJwtToken(token.token)) return false;
		if (token.expiresAt && token.expiresAt < now) return false;
		return true;
	};

	for (const browserPath of browserPaths) {
		const indexedDBTokens = await extractFromIndexedDB(browserPath);
		const validTokens = indexedDBTokens.filter(isValidToken);
		if (validTokens.length > 0) {
			return validTokens;
		}
	}

	for (const browserPath of browserPaths) {
		if (!existsSync(browserPath.cookiePath)) continue;

		let tokens: ExtractedToken[] = [];
		if (browserPath.encryptionMethod === "pbkdf2") {
			tokens = await extractFromChrome(browserPath);
		} else if (browserPath.encryptionMethod === "binary") {
			tokens = await extractFromSafari(browserPath);
		}

		const validTokens = tokens.filter(isValidToken);
		if (validTokens.length > 0) {
			return validTokens;
		}
	}

	return [];
}

/**
 * Extract token from a specific browser
 * @param browser - Browser type to extract from
 * @returns Array of extracted tokens or empty array if browser not found
 */
export async function extractFromBrowser(
	browser: BrowserType,
): Promise<ExtractedToken[]> {
	const browserPaths = getAllBrowserPaths();
	const browserPath = browserPaths.find((b) => b.id === browser);

	if (!browserPath) {
		log.warn("browser_unknown", "browser id not in supported list", {
			browser,
		});
		return [];
	}

	// Try IndexedDB first (contains the actual JWT)
	const indexedDBTokens = await extractFromIndexedDB(browserPath);
	if (indexedDBTokens.length > 0) {
		return indexedDBTokens;
	}

	// Fall back to cookies
	if (!existsSync(browserPath.cookiePath)) {
		log.warn("cookie_path_missing", "cookie file not found", {
			browser: browserPath.id,
			path: browserPath.cookiePath,
		});
		return [];
	}

	if (browserPath.encryptionMethod === "pbkdf2") {
		const tokens = await extractFromChrome(browserPath);
		if (tokens.length === 0) {
			log.warn(
				"chrome_extraction_empty",
				"PBKDF2-decrypted cookies yielded no JWT — Akiflow may have rotated cookie name or format",
				{ browser: browserPath.id },
			);
		}
		return tokens;
	} else if (browserPath.encryptionMethod === "binary") {
		const tokens = await extractFromSafari(browserPath);
		if (tokens.length === 0) {
			log.warn(
				"safari_extraction_empty",
				"binary cookie parse yielded no JWT — Akiflow may have rotated cookie name or format",
				{ browser: browserPath.id },
			);
		}
		return tokens;
	}

	log.warn("no_extraction_method", "browserPath has no encryptionMethod set", {
		browser: browserPath.id,
	});
	return [];
}

function decodeBase64Url(input: string): string {
	const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
	const padding =
		normalized.length % 4 === 0 ? "" : "=".repeat(4 - (normalized.length % 4));
	return atob(`${normalized}${padding}`);
}

function parseJWTExpiration(token: string): Date | undefined {
	const parts = token.split(".");
	if (parts.length !== 3 || !parts[1]) return undefined;

	try {
		const payload = JSON.parse(decodeBase64Url(parts[1])) as { exp?: number };
		if (payload.exp) {
			return new Date(payload.exp * 1000);
		}
	} catch {
		return undefined;
	}
	return undefined;
}

function isJwtToken(token: string): boolean {
	return JWT_MATCHER.test(token);
}

function extractJwtFromString(value: string): string | null {
	const match = value.match(JWT_MATCHER);
	return match ? match[0] : null;
}

export async function extractFromIndexedDB(
	browserPath: BrowserPath,
): Promise<ExtractedToken[]> {
	const tokens: ExtractedToken[] = [];

	if (!browserPath.indexedDBPath) {
		return tokens;
	}

	const akiflowDBPath = join(
		browserPath.indexedDBPath,
		AKIFLOW_INDEXEDDB_FOLDER,
	);

	if (!existsSync(akiflowDBPath)) {
		return tokens;
	}

	const files = readdirSync(akiflowDBPath);
	const uniqueTokens = new Map<string, ExtractedToken>();
	const allRefreshTokens = new Set<string>();

	for (const file of files) {
		if (!file.endsWith(".log") && !file.endsWith(".ldb")) {
			continue;
		}

		const fullPath = join(akiflowDBPath, file);
		const stat = statSync(fullPath);

		if (!stat.isFile()) {
			continue;
		}

		try {
			const content = readFileSync(fullPath);
			const str = content.toString("utf-8");

			const refreshMatches = str.match(REFRESH_TOKEN_PATTERN);
			if (refreshMatches) {
				for (const rt of refreshMatches) {
					allRefreshTokens.add(rt);
				}
			}

			const matches = str.match(JWT_PATTERN);
			if (!matches) {
				continue;
			}

			for (const jwt of matches) {
				const expiresAt = parseJWTExpiration(jwt);
				const isExpired = expiresAt ? expiresAt < new Date() : false;

				if (!isExpired && !uniqueTokens.has(jwt)) {
					uniqueTokens.set(jwt, {
						browser: browserPath.id,
						token: jwt,
						source: fullPath,
						expiresAt,
					});
				}
			}
		} catch {}
	}

	const longestRefreshToken = Array.from(allRefreshTokens).sort(
		(a, b) => b.length - a.length,
	)[0];

	const sortedTokens = Array.from(uniqueTokens.values()).sort((a, b) => {
		const aTime = a.expiresAt?.getTime() ?? 0;
		const bTime = b.expiresAt?.getTime() ?? 0;
		return bTime - aTime;
	});

	if (longestRefreshToken && sortedTokens.length > 0 && sortedTokens[0]) {
		sortedTokens[0].refreshToken = longestRefreshToken;
	}

	return sortedTokens;
}
