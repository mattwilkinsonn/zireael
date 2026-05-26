import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { doCommand } from "../../commands/do";

class ExitError extends Error {
	constructor(public code: number) {
		super(`process.exit(${code})`);
	}
}

const testCacheDir = join(homedir(), ".cache", "af");
const testContextFile = join(testCacheDir, "last-list.json");

const mockContextFile = {
	tasks: [
		{ shortId: 1, id: "abc123def456", title: "Buy groceries" },
		{ shortId: 2, id: "xyz789uvw012", title: "Write report" },
		{ shortId: 3, id: "pqr345stu678", title: "Call client" },
	],
	timestamp: Date.now(),
};

describe("do command", () => {
	beforeEach(async () => {
		mkdirSync(testCacheDir, { recursive: true });
		writeFileSync(testContextFile, JSON.stringify(mockContextFile));

		// Prevent hitting real ~/.config/af credentials.
		spyOn(
			await import("../../lib/auth/storage"),
			"loadCredentials",
		).mockResolvedValue({
			token: "test-token",
			clientId: "test-client-id",
			expiryTimestamp: Date.now() + 60_000,
		});

		spyOn(process, "exit").mockImplementation((code?: number) => {
			throw new ExitError(code ?? 0);
		});
	});

	afterEach(() => {
		try {
			rmSync(testContextFile);
		} catch {
			// file may not exist
		}
	});

	it("completes single task by short ID", async () => {
		const consoleSpy = spyOn(console, "log");
		const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(
				JSON.stringify({
					success: true,
					message: null,
					data: [
						{
							id: "abc123def456",
							done: true,
							done_at: new Date().toISOString(),
							status: 2,
						},
					],
				}),
				{ status: 200 },
			),
		);

		const mockContext = {
			args: { ids: "1" },
		};

		await doCommand.run?.(mockContext as any);

		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("✓ Completed 1 task(s):"),
		);
		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("Buy groceries"),
		);
		expect(fetchSpy).toHaveBeenCalled();
	});

	it("completes multiple tasks by short IDs", async () => {
		const consoleSpy = spyOn(console, "log");
		const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(
				JSON.stringify({
					success: true,
					message: null,
					data: [
						{
							id: "abc123def456",
							done: true,
							done_at: new Date().toISOString(),
							status: 2,
						},
						{
							id: "xyz789uvw012",
							done: true,
							done_at: new Date().toISOString(),
							status: 2,
						},
					],
				}),
				{ status: 200 },
			),
		);

		const mockContext = {
			args: { ids: ["1", "2"] },
		};

		await doCommand.run?.(mockContext as any);

		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("✓ Completed 2 task(s):"),
		);
		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("Buy groceries"),
		);
		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("Write report"),
		);
		expect(fetchSpy).toHaveBeenCalled();
	});

	it("completes task by partial UUID", async () => {
		const consoleSpy = spyOn(console, "log");
		const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(
				JSON.stringify({
					success: true,
					message: null,
					data: [
						{
							id: "abc123def456",
							done: true,
							done_at: new Date().toISOString(),
							status: 2,
						},
					],
				}),
				{ status: 200 },
			),
		);

		const mockContext = {
			args: { ids: "abc123" },
		};

		await doCommand.run?.(mockContext as any);

		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("✓ Completed 1 task(s):"),
		);
		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("Buy groceries"),
		);
		expect(fetchSpy).toHaveBeenCalled();
	});

	it("handles invalid short ID", async () => {
		const consoleErrorSpy = spyOn(console, "error");
		const mockContext = {
			args: { ids: "999" },
		};

		try {
			await doCommand.run?.(mockContext as any);
			throw new Error("Expected ExitError");
		} catch (error) {
			if (!(error instanceof ExitError)) {
				throw error;
			}
		}

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			expect.stringContaining("Could not resolve task IDs"),
		);
	});

	it("handles ambiguous UUID", async () => {
		const consoleErrorSpy = spyOn(console, "error");
		const contextWithDuplicates = {
			tasks: [
				{ shortId: 1, id: "abc123def456", title: "Task 1" },
				{ shortId: 2, id: "abc123xyz789", title: "Task 2" },
			],
			timestamp: Date.now(),
		};
		writeFileSync(testContextFile, JSON.stringify(contextWithDuplicates));

		const mockContext = {
			args: { ids: "abc123" },
		};

		try {
			await doCommand.run?.(mockContext as any);
			throw new Error("Expected ExitError");
		} catch (error) {
			if (!(error instanceof ExitError)) {
				throw error;
			}
		}

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			expect.stringContaining("Ambiguous UUID"),
		);
	});

	it("handles missing context file", async () => {
		rmSync(testContextFile);
		const consoleErrorSpy = spyOn(console, "error");

		const mockContext = {
			args: { ids: "1" },
		};

		try {
			await doCommand.run?.(mockContext as any);
			throw new Error("Expected ExitError");
		} catch (error) {
			if (!(error instanceof ExitError)) {
				throw error;
			}
		}

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			expect.stringContaining("No task context found"),
		);
	});

	it("handles API error", async () => {
		const consoleErrorSpy = spyOn(console, "error");
		const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(
				JSON.stringify({
					success: false,
					message: "API error",
					data: [],
				}),
				{ status: 200 },
			),
		);

		const mockContext = {
			args: { ids: "1" },
		};

		try {
			await doCommand.run?.(mockContext as any);
			throw new Error("Expected ExitError");
		} catch (error) {
			if (!(error instanceof ExitError)) {
				throw error;
			}
		}

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			expect.stringContaining("Failed to complete tasks"),
		);
		expect(fetchSpy).toHaveBeenCalled();
	});

	it("handles network error", async () => {
		const consoleErrorSpy = spyOn(console, "error");
		const fetchSpy = spyOn(globalThis, "fetch").mockRejectedValue(
			new Error("Network error"),
		);

		const mockContext = {
			args: { ids: "1" },
		};

		try {
			await doCommand.run?.(mockContext as any);
			throw new Error("Expected ExitError");
		} catch (error) {
			if (!(error instanceof ExitError)) {
				throw error;
			}
		}

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			expect.stringContaining("Failed to complete tasks"),
		);
		expect(fetchSpy).toHaveBeenCalled();
	});

	it("sends correct payload to API", async () => {
		const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(
				JSON.stringify({
					success: true,
					message: null,
					data: [],
				}),
				{ status: 200 },
			),
		);

		const mockContext = {
			args: { ids: "1" },
		};

		await doCommand.run?.(mockContext as any);

		const callArgs = fetchSpy.mock.calls[0];
		if (!callArgs?.[1]) {
			throw new Error("fetch was not called");
		}

		const requestBody = JSON.parse(callArgs[1].body as string);

		expect(requestBody[0]).toHaveProperty("id", "abc123def456");
		expect(requestBody[0]).toHaveProperty("done", true);
		expect(requestBody[0]).toHaveProperty("status", 2);
		expect(requestBody[0]).toHaveProperty("done_at");
		expect(requestBody[0]).toHaveProperty("global_updated_at");
	});

	it("completes multiple tasks with mixed short IDs and UUIDs", async () => {
		const consoleSpy = spyOn(console, "log");
		const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(
				JSON.stringify({
					success: true,
					message: null,
					data: [
						{
							id: "abc123def456",
							done: true,
							done_at: new Date().toISOString(),
							status: 2,
						},
						{
							id: "pqr345stu678",
							done: true,
							done_at: new Date().toISOString(),
							status: 2,
						},
					],
				}),
				{ status: 200 },
			),
		);

		const mockContext = {
			args: { ids: ["1", "pqr345"] },
		};

		await doCommand.run?.(mockContext as any);

		expect(consoleSpy).toHaveBeenCalledWith(
			expect.stringContaining("✓ Completed 2 task(s):"),
		);
		expect(fetchSpy).toHaveBeenCalled();
	});
});
