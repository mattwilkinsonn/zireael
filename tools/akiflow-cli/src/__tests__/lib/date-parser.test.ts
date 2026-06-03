import { describe, expect, it } from "bun:test";
import {
	createDateTimeUTC,
	getLocalTimezone,
	getTodayDate,
	getTomorrowDate,
	parseDate,
	parseTime,
} from "../../lib/date-parser";

// Fixed reference: Wednesday, June 3, 2026 12:00 local time. Returned
// as a fresh Date each call so a future mutation of the `now` argument
// inside any date-parser function can't leak across test cases.
const refNow = () => new Date(2026, 5, 3, 12, 0, 0, 0);

describe("parseDate", () => {
	it("returns null when date string cannot be parsed", () => {
		// given
		const invalidDate = "not a date";

		// when
		const result = parseDate(invalidDate, refNow());

		// then
		expect(result).toBeNull();
	});

	it("parses 'today' and returns correct ISO date", () => {
		// when
		const result = parseDate("today", refNow());

		// then
		expect(result).toBe("2026-06-03");
	});

	it("parses 'tomorrow' and returns correct ISO date", () => {
		// when
		const result = parseDate("tomorrow", refNow());

		// then
		expect(result).toBe("2026-06-04");
	});

	it("parses 'next monday' and returns correct ISO date", () => {
		// when
		const result = parseDate("next monday", refNow());

		// then — chrono returns the Monday of the following week (2026-06-08).
		expect(result).toBe("2026-06-08");
	});

	it("parses 'next friday' and returns correct ISO date", () => {
		// when
		const result = parseDate("next friday", refNow());

		// then — chrono 2.x's "next friday" means next week's Friday
		// (Sunday-anchored week), so from Wed 2026-06-03 → 2026-06-12,
		// not the upcoming Friday (2026-06-05).
		expect(result).toBe("2026-06-12");
	});

	it("parses 'in 3 days' and returns correct ISO date", () => {
		// when
		const result = parseDate("in 3 days", refNow());

		// then
		expect(result).toBe("2026-06-06");
	});

	it("parses 'next week' and returns correct ISO date", () => {
		// when
		const result = parseDate("next week", refNow());

		// then — chrono returns the same weekday one week out (Wed → Wed).
		expect(result).toBe("2026-06-10");
	});

	it("returns date in correct ISO format (YYYY-MM-DD)", () => {
		// when
		const result = parseDate("today", refNow());

		// then
		expect(result).toMatch(/^\d{4}-\d{2}-\d{2}$/);
	});
});

describe("getTodayDate", () => {
	it("returns today's date in ISO format", () => {
		// when
		const result = getTodayDate(refNow());

		// then
		expect(result).toBe("2026-06-03");
	});

	it("returns date in correct ISO format (YYYY-MM-DD)", () => {
		// when
		const result = getTodayDate(refNow());

		// then
		expect(result).toMatch(/^\d{4}-\d{2}-\d{2}$/);
	});
});

describe("getTomorrowDate", () => {
	it("returns tomorrow's date in ISO format", () => {
		// when
		const result = getTomorrowDate(refNow());

		// then
		expect(result).toBe("2026-06-04");
	});

	it("returns date in correct ISO format (YYYY-MM-DD)", () => {
		// when
		const result = getTomorrowDate(refNow());

		// then
		expect(result).toMatch(/^\d{4}-\d{2}-\d{2}$/);
	});
});

describe("parseTime", () => {
	it("parses valid 24-hour time format HH:MM", () => {
		// given
		const timeString = "21:00";

		// when
		const result = parseTime(timeString);

		// then
		expect(result).toEqual({ hours: 21, minutes: 0 });
	});

	it("parses single digit hour format H:MM", () => {
		// given
		const timeString = "9:30";

		// when
		const result = parseTime(timeString);

		// then
		expect(result).toEqual({ hours: 9, minutes: 30 });
	});

	it("parses midnight correctly", () => {
		// given
		const timeString = "00:00";

		// when
		const result = parseTime(timeString);

		// then
		expect(result).toEqual({ hours: 0, minutes: 0 });
	});

	it("parses end of day correctly", () => {
		// given
		const timeString = "23:59";

		// when
		const result = parseTime(timeString);

		// then
		expect(result).toEqual({ hours: 23, minutes: 59 });
	});

	it("returns null for invalid time format", () => {
		// given
		const invalidTime = "25:00";

		// when
		const result = parseTime(invalidTime);

		// then
		expect(result).toBeNull();
	});

	it("returns null for invalid minutes", () => {
		// given
		const invalidTime = "14:60";

		// when
		const result = parseTime(invalidTime);

		// then
		expect(result).toBeNull();
	});

	it("returns null for non-time string", () => {
		// given
		const invalidTime = "not a time";

		// when
		const result = parseTime(invalidTime);

		// then
		expect(result).toBeNull();
	});

	it("trims whitespace from input", () => {
		// given
		const timeString = "  14:30  ";

		// when
		const result = parseTime(timeString);

		// then
		expect(result).toEqual({ hours: 14, minutes: 30 });
	});
});

describe("createDateTimeUTC", () => {
	it("creates UTC datetime string from date and time", () => {
		// given
		const dateString = "2025-01-03";
		const hours = 14;
		const minutes = 30;

		// when
		const result = createDateTimeUTC(dateString, hours, minutes);

		// then
		expect(result).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.\d{3}Z$/);
	});

	it("converts local time to UTC correctly", () => {
		// given
		const dateString = "2025-01-03";
		const hours = 12;
		const minutes = 0;

		// when
		const result = createDateTimeUTC(dateString, hours, minutes);
		const parsedDate = new Date(result);

		// then
		const localDate = new Date(2025, 0, 3, 12, 0, 0);
		expect(parsedDate.getTime()).toBe(localDate.getTime());
	});
});

describe("getLocalTimezone", () => {
	it("returns a valid IANA timezone string", () => {
		// when
		const result = getLocalTimezone();

		// then
		expect(result).toBeTruthy();
		expect(typeof result).toBe("string");
	});
});
