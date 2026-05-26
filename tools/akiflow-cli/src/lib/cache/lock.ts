import {
	existsSync,
	mkdirSync,
	readFileSync,
	unlinkSync,
	writeFileSync,
} from "node:fs";
import { dirname } from "node:path";

const STALE_AFTER_MS = 60_000;

/**
 * Run `fn` while holding an exclusive lock on `lockPath`. Concurrent callers
 * wait. Lock is auto-released after fn returns or throws. Stale locks
 * (held by a crashed process for >60s) are stolen.
 */
export async function withLock<T>(
	lockPath: string,
	fn: () => Promise<T>,
): Promise<T> {
	mkdirSync(dirname(lockPath), { recursive: true });
	await acquire(lockPath);
	try {
		return await fn();
	} finally {
		release(lockPath);
	}
}

async function acquire(lockPath: string): Promise<void> {
	for (let attempt = 0; attempt < 100; attempt++) {
		try {
			writeFileSync(lockPath, `${process.pid}\n${Date.now()}\n`, {
				flag: "wx",
			});
			return;
		} catch (err: unknown) {
			if (!isExistsErr(err)) throw err;
			if (isStale(lockPath)) {
				try {
					unlinkSync(lockPath);
				} catch {
					/* race */
				}
				continue;
			}
			await new Promise((r) => setTimeout(r, 50 + Math.random() * 50));
		}
	}
	throw new Error(`could not acquire ${lockPath} after 100 attempts (~10s)`);
}

function release(lockPath: string): void {
	try {
		unlinkSync(lockPath);
	} catch {
		/* already gone */
	}
}

function isExistsErr(err: unknown): boolean {
	return (
		typeof err === "object" &&
		err !== null &&
		(err as { code?: string }).code === "EEXIST"
	);
}

function isStale(lockPath: string): boolean {
	if (!existsSync(lockPath)) return false;
	try {
		const parts = readFileSync(lockPath, "utf8").split("\n");
		const ts = Number(parts[1]);
		if (!Number.isFinite(ts)) return true;
		return Date.now() - ts > STALE_AFTER_MS;
	} catch {
		return true;
	}
}
