import { describe, expect, test } from "bun:test";
import { parseMonth, resolveRange } from "../../lib/date-parser";

describe("resolveRange", () => {
	test("'today' covers start to end of today (local)", () => {
		const r = resolveRange("today", new Date("2026-05-21T15:00:00"));
		expect(r.from.getDate()).toBe(21);
		expect(r.to.getDate()).toBe(21);
		expect(r.from.getHours()).toBe(0);
		expect(r.to.getHours()).toBe(23);
	});

	test("'this-week' spans Monday through Sunday of current ISO week", () => {
		// 2026-05-21 is a Thursday
		const r = resolveRange("this-week", new Date("2026-05-21T15:00:00"));
		// Monday = day 1, Sunday = day 0 in JS
		expect(r.from.getDay()).toBe(1);
		expect(r.to.getDay()).toBe(0);
	});

	test("'this-month' spans first to last day of month", () => {
		const r = resolveRange("this-month", new Date("2026-05-21T15:00:00"));
		expect(r.from.getDate()).toBe(1);
		expect(r.to.getDate()).toBe(31);
		expect(r.from.getMonth()).toBe(4); // May = 4
	});

	test("'next-month' returns June for May input", () => {
		const r = resolveRange("next-month", new Date("2026-05-21T15:00:00"));
		expect(r.from.getMonth()).toBe(5); // June = 5
		expect(r.from.getDate()).toBe(1);
		expect(r.to.getDate()).toBe(30); // June has 30 days
	});
});

describe("parseMonth", () => {
	test("'2026-05' → year+month", () => {
		expect(parseMonth("2026-05")).toEqual({ year: 2026, month: 5 });
	});
	test("'may' alone uses current year", () => {
		const now = new Date("2026-05-21");
		expect(parseMonth("may", now)).toEqual({ year: 2026, month: 5 });
	});
	test("'may 2026' parses both", () => {
		expect(parseMonth("may 2026")).toEqual({ year: 2026, month: 5 });
	});
	test("returns null for garbage input", () => {
		expect(parseMonth("xyzzy")).toBe(null);
	});
});
