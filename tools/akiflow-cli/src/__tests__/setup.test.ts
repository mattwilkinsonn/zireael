import { describe, expect, it } from "bun:test";

describe("Bun test setup", () => {
	it("verifies Bun test runner works", () => {
		// given
		const value = 1;

		// when
		const result = value + 1;

		// then
		expect(result).toBe(2);
	});

	it("verifies basic string operations", () => {
		// given
		const message = "Akiflow CLI";

		// when
		const result = message.toLowerCase();

		// then
		expect(result).toBe("akiflow cli");
	});
});
