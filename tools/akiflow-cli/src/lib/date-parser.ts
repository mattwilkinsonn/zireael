import { parseDate as chronoParseDate } from "chrono-node";

/**
 * Parse natural language date string to ISO date format (YYYY-MM-DD).
 * Supports: "today", "tomorrow", "next monday", "next friday", "in 3 days", "next week"
 *
 * @param dateString - Natural language date string
 * @param now - Reference "now" for parsing. Defaults to the current time;
 *              tests pass a fixed Date to keep assertions deterministic.
 * @returns ISO date string (YYYY-MM-DD) or null if parsing fails
 */
export function parseDate(
	dateString: string,
	now: Date = new Date(),
): string | null {
	const result = chronoParseDate(dateString, now, { forwardDate: true });

	if (!result) {
		return null;
	}

	const year = result.getFullYear();
	const month = String(result.getMonth() + 1).padStart(2, "0");
	const day = String(result.getDate()).padStart(2, "0");

	return `${year}-${month}-${day}`;
}

/**
 * Get today's date in ISO format (YYYY-MM-DD).
 *
 * @param now - Reference "now". Defaults to the current time; tests pass
 *              a fixed Date to keep assertions deterministic.
 * @returns Today's date as ISO string
 */
export function getTodayDate(now: Date = new Date()): string {
	const year = now.getFullYear();
	const month = String(now.getMonth() + 1).padStart(2, "0");
	const day = String(now.getDate()).padStart(2, "0");

	return `${year}-${month}-${day}`;
}

/**
 * Get tomorrow's date in ISO format (YYYY-MM-DD).
 *
 * @param now - Reference "now". Defaults to the current time; tests pass
 *              a fixed Date to keep assertions deterministic.
 * @returns Tomorrow's date as ISO string
 */
export function getTomorrowDate(now: Date = new Date()): string {
	const tomorrow = new Date(now);
	tomorrow.setDate(tomorrow.getDate() + 1);

	const year = tomorrow.getFullYear();
	const month = String(tomorrow.getMonth() + 1).padStart(2, "0");
	const day = String(tomorrow.getDate()).padStart(2, "0");

	return `${year}-${month}-${day}`;
}

/**
 * Parse time string (HH:MM or H:MM format) to hours and minutes.
 *
 * @param timeString - Time string (e.g., "21:00", "9:30", "14:30")
 * @returns Object with hours and minutes, or null if parsing fails
 */
export function parseTime(
	timeString: string,
): { hours: number; minutes: number } | null {
	const trimmed = timeString.trim();
	const match = trimmed.match(/^(\d{1,2}):(\d{2})$/);

	if (!match) {
		return null;
	}

	const hours = parseInt(match[1]!, 10);
	const minutes = parseInt(match[2]!, 10);

	if (hours < 0 || hours > 23 || minutes < 0 || minutes > 59) {
		return null;
	}

	return { hours, minutes };
}

/**
 * Create UTC datetime string from date and time.
 *
 * @param dateString - ISO date string (YYYY-MM-DD)
 * @param hours - Hours (0-23)
 * @param minutes - Minutes (0-59)
 * @returns UTC ISO datetime string
 */
export function createDateTimeUTC(
	dateString: string,
	hours: number,
	minutes: number,
): string {
	const [year, month, day] = dateString.split("-").map(Number);
	const localDate = new Date(year!, month! - 1, day!, hours, minutes, 0, 0);
	return localDate.toISOString();
}

/**
 * Get local timezone identifier.
 *
 * @returns IANA timezone string (e.g., "Asia/Seoul")
 */
export function getLocalTimezone(): string {
	return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

// ============================================================
// Named ranges + month parsing (added in fork v0.1 for ls/cal filters)
// ============================================================

export type NamedRange =
	| "today"
	| "tomorrow"
	| "yesterday"
	| "this-week"
	| "next-week"
	| "this-month"
	| "next-month";

export interface DateRange {
	from: Date;
	to: Date;
}

const startOfDay = (d: Date): Date =>
	new Date(d.getFullYear(), d.getMonth(), d.getDate(), 0, 0, 0, 0);
const endOfDay = (d: Date): Date =>
	new Date(d.getFullYear(), d.getMonth(), d.getDate(), 23, 59, 59, 999);

/**
 * Resolve a named range to a {from, to} pair (local time). Week starts on
 * Monday (ISO 8601).
 */
export function resolveRange(
	name: NamedRange,
	now: Date = new Date(),
): DateRange {
	switch (name) {
		case "today":
			return { from: startOfDay(now), to: endOfDay(now) };
		case "tomorrow": {
			const t = new Date(now);
			t.setDate(t.getDate() + 1);
			return { from: startOfDay(t), to: endOfDay(t) };
		}
		case "yesterday": {
			const y = new Date(now);
			y.setDate(y.getDate() - 1);
			return { from: startOfDay(y), to: endOfDay(y) };
		}
		case "this-week": {
			const dow = (now.getDay() + 6) % 7; // 0 = Monday
			const monday = new Date(now);
			monday.setDate(now.getDate() - dow);
			const sunday = new Date(monday);
			sunday.setDate(monday.getDate() + 6);
			return { from: startOfDay(monday), to: endOfDay(sunday) };
		}
		case "next-week": {
			const dow = (now.getDay() + 6) % 7;
			const monday = new Date(now);
			monday.setDate(now.getDate() - dow + 7);
			const sunday = new Date(monday);
			sunday.setDate(monday.getDate() + 6);
			return { from: startOfDay(monday), to: endOfDay(sunday) };
		}
		case "this-month": {
			const first = new Date(now.getFullYear(), now.getMonth(), 1);
			const last = new Date(now.getFullYear(), now.getMonth() + 1, 0);
			return { from: startOfDay(first), to: endOfDay(last) };
		}
		case "next-month": {
			const first = new Date(now.getFullYear(), now.getMonth() + 1, 1);
			const last = new Date(now.getFullYear(), now.getMonth() + 2, 0);
			return { from: startOfDay(first), to: endOfDay(last) };
		}
	}
}

const MONTH_NAMES = [
	"jan",
	"feb",
	"mar",
	"apr",
	"may",
	"jun",
	"jul",
	"aug",
	"sep",
	"oct",
	"nov",
	"dec",
];

/**
 * Parse a month identifier into {year, month}. Accepts:
 *   "2026-05"      → { year: 2026, month: 5 }
 *   "may"          → { year: <now>, month: 5 }
 *   "may 2026"     → { year: 2026, month: 5 }
 */
export function parseMonth(
	input: string,
	now: Date = new Date(),
): { year: number; month: number } | null {
	const trimmed = input.trim().toLowerCase();
	const m1 = trimmed.match(/^(\d{4})-(\d{1,2})$/);
	if (m1) return { year: Number(m1[1]), month: Number(m1[2]) };
	const parts = trimmed.split(/\s+/);
	const monthIdx = MONTH_NAMES.findIndex((m) => parts[0]?.startsWith(m));
	if (monthIdx < 0) return null;
	const year = parts[1] ? Number(parts[1]) : now.getFullYear();
	return { year, month: monthIdx + 1 };
}
