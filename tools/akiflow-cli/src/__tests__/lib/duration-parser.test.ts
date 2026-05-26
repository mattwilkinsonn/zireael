import { describe, expect, it } from "bun:test";
import {
	parseDuration,
	parseDurationToSeconds,
} from "../../lib/duration-parser";

describe("parseDuration", () => {
	it("parses minutes correctly", () => {
		// given
		const duration = "30m";

		// when
		const result = parseDuration(duration);

		// then
		expect(result).toBe(30 * 60 * 1000);
	});

	it("parses hours correctly", () => {
		// given
		const duration = "2h";

		// when
		const result = parseDuration(duration);

		// then
		expect(result).toBe(2 * 60 * 60 * 1000);
	});

	it("parses days correctly", () => {
		// given
		const duration = "1d";

		// when
		const result = parseDuration(duration);

		// then
		expect(result).toBe(24 * 60 * 60 * 1000);
	});

	it("parses weeks correctly", () => {
		// given
		const duration = "1w";

		// when
		const result = parseDuration(duration);

		// then
		expect(result).toBe(7 * 24 * 60 * 60 * 1000);
	});

	it("throws error for invalid format", () => {
		// given
		const invalidDuration = "invalid";

		// when/then
		expect(() => parseDuration(invalidDuration)).toThrow();
	});
});

describe("parseDurationToSeconds", () => {
	it("converts 30 minutes to 1800 seconds", () => {
		// given
		const duration = "30m";

		// when
		const result = parseDurationToSeconds(duration);

		// then
		expect(result).toBe(1800);
	});

	it("converts 1 hour to 3600 seconds", () => {
		// given
		const duration = "1h";

		// when
		const result = parseDurationToSeconds(duration);

		// then
		expect(result).toBe(3600);
	});

	it("converts 2 hours to 7200 seconds", () => {
		// given
		const duration = "2h";

		// when
		const result = parseDurationToSeconds(duration);

		// then
		expect(result).toBe(7200);
	});
});
