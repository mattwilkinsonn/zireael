import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

/**
 * Supported browser types for cookie extraction
 */
export type BrowserType = "chrome" | "arc" | "brave" | "edge" | "safari";

/**
 * Browser path configuration
 */
export interface BrowserPath {
	/** Browser identifier */
	id: BrowserType;
	/** Display name */
	name: string;
	/** Path to cookies database */
	cookiePath: string;
	/** Path to IndexedDB folder (Chrome-family only) */
	indexedDBPath: string | null;
	/** Encryption method used by this browser */
	encryptionMethod: "pbkdf2" | "binary";
}

/**
 * Browser cookie paths for macOS
 *
 * Cookie encryption details:
 * - Chrome-family (Chrome, Arc, Brave, Edge): Uses PBKDF2 with salt "saltysalt" and 1003 iterations
 * - Safari: Uses binary format (not SQLite), stored in Cookies.binarycookies
 */
export const BROWSER_PATHS: Record<BrowserType, string> = {
	/**
	 * Chrome cookies path
	 * Encryption: PBKDF2 (salt: "saltysalt", iterations: 1003)
	 * Database: SQLite
	 */
	chrome: join(
		homedir(),
		"Library/Application Support/Google/Chrome/Default/Cookies",
	),

	/**
	 * Arc browser cookies path
	 * Encryption: PBKDF2 (salt: "saltysalt", iterations: 1003)
	 * Database: SQLite
	 */
	arc: join(
		homedir(),
		"Library/Application Support/Arc/User Data/Default/Cookies",
	),

	/**
	 * Brave browser cookies path
	 * Encryption: PBKDF2 (salt: "saltysalt", iterations: 1003)
	 * Database: SQLite
	 */
	brave: join(
		homedir(),
		"Library/Application Support/BraveSoftware/Brave-Browser/Default/Cookies",
	),

	/**
	 * Microsoft Edge cookies path
	 * Encryption: PBKDF2 (salt: "saltysalt", iterations: 1003)
	 * Database: SQLite
	 */
	edge: join(
		homedir(),
		"Library/Application Support/Microsoft Edge/Default/Cookies",
	),

	/**
	 * Safari cookies path
	 * Encryption: Binary format (proprietary)
	 * Database: Binary format (not SQLite)
	 */
	safari: join(homedir(), "Library/Cookies/Cookies.binarycookies"),
};

/**
 * IndexedDB paths for Chrome-family browsers
 */
export const INDEXEDDB_PATHS: Record<BrowserType, string | null> = {
	chrome: join(
		homedir(),
		"Library/Application Support/Google/Chrome/Default/IndexedDB",
	),
	arc: join(
		homedir(),
		"Library/Application Support/Arc/User Data/Default/IndexedDB",
	),
	brave: join(
		homedir(),
		"Library/Application Support/BraveSoftware/Brave-Browser/Default/IndexedDB",
	),
	edge: join(
		homedir(),
		"Library/Application Support/Microsoft Edge/Default/IndexedDB",
	),
	safari: null, // Safari doesn't use IndexedDB for Akiflow tokens
};

/**
 * Get all browser paths with metadata
 * @returns Array of browser path configurations
 */
export function getAllBrowserPaths(): BrowserPath[] {
	return [
		{
			id: "chrome",
			name: "Google Chrome",
			cookiePath: BROWSER_PATHS.chrome,
			indexedDBPath: INDEXEDDB_PATHS.chrome,
			encryptionMethod: "pbkdf2",
		},
		{
			id: "arc",
			name: "Arc Browser",
			cookiePath: BROWSER_PATHS.arc,
			indexedDBPath: INDEXEDDB_PATHS.arc,
			encryptionMethod: "pbkdf2",
		},
		{
			id: "brave",
			name: "Brave Browser",
			cookiePath: BROWSER_PATHS.brave,
			indexedDBPath: INDEXEDDB_PATHS.brave,
			encryptionMethod: "pbkdf2",
		},
		{
			id: "edge",
			name: "Microsoft Edge",
			cookiePath: BROWSER_PATHS.edge,
			indexedDBPath: INDEXEDDB_PATHS.edge,
			encryptionMethod: "pbkdf2",
		},
		{
			id: "safari",
			name: "Safari",
			cookiePath: BROWSER_PATHS.safari,
			indexedDBPath: INDEXEDDB_PATHS.safari,
			encryptionMethod: "binary",
		},
	];
}

/**
 * Check if a browser is installed by verifying cookie database exists
 * @param browser - Browser type to check
 * @returns true if browser cookies database exists, false otherwise
 */
export function isBrowserInstalled(browser: BrowserType): boolean {
	const cookiePath = BROWSER_PATHS[browser];
	return existsSync(cookiePath);
}

/**
 * Get installed browsers
 * @returns Array of installed browser types
 */
export function getInstalledBrowsers(): BrowserType[] {
	const browsers: BrowserType[] = ["chrome", "arc", "brave", "edge", "safari"];
	return browsers.filter(isBrowserInstalled);
}
