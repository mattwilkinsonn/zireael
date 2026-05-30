import { describe, expect, it } from "bun:test";
import {
	createDateTimeUTC,
	getLocalTimezone,
	getTodayDate,
	getTomorrowDate,
	parseDate,
	parseTime,
} from "../../lib/date-parser";

describe("parseDate", () => {
	it("returns null when date string cannot be parsed", () => {
		// given
		const invalidDate = "not a date";

		// when
		const result = parseDate(invalidDate);

		// then
		expect(result).toBeNull();
	});

	it("parses 'today' and returns correct ISO date", () => {
		// given
		const today = new Date();
		const expectedYear = today.getFullYear();
		const expectedMonth = String(today.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(today.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = parseDate("today");

		// then
		expect(result).toBe(expected);
	});

	it("parses 'tomorrow' and returns correct ISO date", () => {
		// given
		const tomorrow = new Date();
		tomorrow.setDate(tomorrow.getDate() + 1);
		const expectedYear = tomorrow.getFullYear();
		const expectedMonth = String(tomorrow.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(tomorrow.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = parseDate("tomorrow");

		// then
		expect(result).toBe(expected);
	});

	it("parses 'next monday' and returns correct ISO date", () => {
		// given
		const today = new Date();
		const dayOfWeek = today.getDay();
		const daysUntilMonday = (8 - dayOfWeek) % 7 || 7;
		const nextMonday = new Date(today);
		nextMonday.setDate(today.getDate() + daysUntilMonday);
		const expectedYear = nextMonday.getFullYear();
		const expectedMonth = String(nextMonday.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(nextMonday.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = parseDate("next monday");

		// then
		expect(result).toBe(expected);
	});

	it("parses 'next friday' and returns correct ISO date", () => {
		// given
		// chrono-node "next friday" means the upcoming Friday: 1-7 days
		// from today, wrapping if today IS Friday (then 7 days, never 0).
		const today = new Date();
		const dayOfWeek = today.getDay();
		const daysUntilNextFriday = (5 - dayOfWeek + 7) % 7 || 7;
		const nextFriday = new Date(today);
		nextFriday.setDate(today.getDate() + daysUntilNextFriday);
		const expectedYear = nextFriday.getFullYear();
		const expectedMonth = String(nextFriday.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(nextFriday.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = parseDate("next friday");

		// then
		expect(result).toBe(expected);
	});

	it("parses 'in 3 days' and returns correct ISO date", () => {
		// given
		const futureDate = new Date();
		futureDate.setDate(futureDate.getDate() + 3);
		const expectedYear = futureDate.getFullYear();
		const expectedMonth = String(futureDate.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(futureDate.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = parseDate("in 3 days");

		// then
		expect(result).toBe(expected);
	});

	it("parses 'next week' and returns correct ISO date", () => {
		// given
		const futureDate = new Date();
		futureDate.setDate(futureDate.getDate() + 7);
		const expectedYear = futureDate.getFullYear();
		const expectedMonth = String(futureDate.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(futureDate.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = parseDate("next week");

		// then
		expect(result).toBe(expected);
	});

	it("returns date in correct ISO format (YYYY-MM-DD)", () => {
		// given
		const dateString = "today";

		// when
		const result = parseDate(dateString);

		// then
		expect(result).toMatch(/^\d{4}-\d{2}-\d{2}$/);
	});
});

describe("getTodayDate", () => {
	it("returns today's date in ISO format", () => {
		// given
		const today = new Date();
		const expectedYear = today.getFullYear();
		const expectedMonth = String(today.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(today.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = getTodayDate();

		// then
		expect(result).toBe(expected);
	});

	it("returns date in correct ISO format (YYYY-MM-DD)", () => {
		// when
		const result = getTodayDate();

		// then
		expect(result).toMatch(/^\d{4}-\d{2}-\d{2}$/);
	});
});

describe("getTomorrowDate", () => {
	it("returns tomorrow's date in ISO format", () => {
		// given
		const tomorrow = new Date();
		tomorrow.setDate(tomorrow.getDate() + 1);
		const expectedYear = tomorrow.getFullYear();
		const expectedMonth = String(tomorrow.getMonth() + 1).padStart(2, "0");
		const expectedDay = String(tomorrow.getDate()).padStart(2, "0");
		const expected = `${expectedYear}-${expectedMonth}-${expectedDay}`;

		// when
		const result = getTomorrowDate();

		// then
		expect(result).toBe(expected);
	});

	it("returns date in correct ISO format (YYYY-MM-DD)", () => {
		// when
		const result = getTomorrowDate();

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
