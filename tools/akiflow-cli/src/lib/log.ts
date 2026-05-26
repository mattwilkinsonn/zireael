import { appendFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { cacheFile } from "./platform-config";

type Level = "debug" | "info" | "warn" | "error";

interface LoggerOptions {
	module: string;
	/** Log file basename in cachePath(). Default: af.log */
	file?: string;
}

export interface Logger {
	debug(phase: string, msg: string, data?: Record<string, unknown>): void;
	info(phase: string, msg: string, data?: Record<string, unknown>): void;
	warn(phase: string, msg: string, data?: Record<string, unknown>): void;
	error(phase: string, msg: string, data?: Record<string, unknown>): void;
}

/**
 * Structured logger. Silent by default — error-level still surfaces to
 * stderr. Set AF_LOG=1 to write JSON Lines to ~/.cache/af/<file>
 * (default af.log). Set AF_DEBUG=1 to also mirror to stderr.
 */
export function createLogger(opts: LoggerOptions): Logger {
	const enabled = !!process.env.AF_LOG || !!process.env.AF_DEBUG;
	const path = cacheFile(opts.file ?? "af.log");

	const emit = (
		level: Level,
		phase: string,
		msg: string,
		data?: Record<string, unknown>,
	): void => {
		if (!enabled) {
			// Errors still surface to stderr even with logging disabled
			if (level === "error") {
				console.error(`[${opts.module}] ${msg}`);
			}
			return;
		}
		const line = JSON.stringify({
			ts: new Date().toISOString(),
			level,
			module: opts.module,
			phase,
			msg,
			...(data && { data }),
		});
		try {
			mkdirSync(dirname(path), { recursive: true });
			appendFileSync(path, `${line}\n`, "utf8");
		} catch {
			// Logging must never throw — swallow file errors
		}
		if (process.env.AF_DEBUG) {
			console.error(line);
		}
	};

	return {
		debug: (p, m, d) => emit("debug", p, m, d),
		info: (p, m, d) => emit("info", p, m, d),
		warn: (p, m, d) => emit("warn", p, m, d),
		error: (p, m, d) => emit("error", p, m, d),
	};
}
