import { homedir } from "node:os";
import { join } from "node:path";

/**
 * Cache directory root for af.
 * Default: ~/.cache/af
 * Override: $AF_CACHE_DIR (used by tests + advanced users).
 */
export function cachePath(): string {
	return process.env.AF_CACHE_DIR ?? join(homedir(), ".cache", "af");
}

export function cacheFile(name: string): string {
	return join(cachePath(), name);
}
