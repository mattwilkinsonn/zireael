import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import * as fs from "node:fs";
import { taskPlanCommand } from "../../commands/task/index";
import * as storage from "../../lib/auth/storage";

const mockCredentials = {
	token: "test-jwt-token",
	clientId: "test-client-id-12345",
	expiryTimestamp: Date.now() + 86400000,
};

const mockContextFile = {
	tasks: [
		{ shortId: 1, id: "task-uuid-1", title: "Task 1" },
		{ shortId: 2, id: "task-uuid-2", title: "Task 2" },
	],
	timestamp: Date.now(),
};

describe("taskPlanCommand", () => {
	let fetchSpy: ReturnType<typeof spyOn>;
	let loadCredentialsSpy: ReturnType<typeof spyOn>;
	let readFileSyncSpy: ReturnType<typeof spyOn>;

	beforeEach(() => {
		fetchSpy = spyOn(globalThis, "fetch");
		loadCredentialsSpy = spyOn(storage, "loadCredentials").mockResolvedValue(
			mockCredentials,
		);
		readFileSyncSpy = spyOn(fs, "readFileSync").mockReturnValue(
			JSON.stringify(mockContextFile),
		);
	});

	afterEach(() => {
		fetchSpy.mockRestore();
		loadCredentialsSpy.mockRestore();
		readFileSyncSpy.mockRestore();
	});

	it("schedules task with YYYY-MM-DD date format", async () => {
		// given
		fetchSpy.mockResolvedValue(
			new Response(JSON.stringify({ success: true, message: null, data: [] }), {
				status: 200,
			}),
		);
		const consoleLogSpy = spyOn(console, "log");

		// when
		await taskPlanCommand.run!({
			args: { id: "task-uuid-1", date: "2026-02-10", at: undefined, _: [] },
			rawArgs: [],
		} as any);

		// then
		expect(fetchSpy).toHaveBeenCalled();
		const fetchCall = fetchSpy.mock.calls[0];
		const requestBody = JSON.parse(fetchCall[1]?.body as string);
		expect(requestBody[0].date).toBe("2026-02-10");
		expect(requestBody[0].datetime).toBeUndefined();
		expect(requestBody[0].datetime_tz).toBeUndefined();
		expect(consoleLogSpy).toHaveBeenCalledWith(
			'✓ Scheduled task "task-uuid-1" for 2026-02-10',
		);

		consoleLogSpy.mockRestore();
	});

	it("schedules task with natural language date 'today'", async () => {
		// given
		const today = new Date();
		const expectedDate = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;

		fetchSpy.mockResolvedValue(
			new Response(JSON.stringify({ success: true, message: null, data: [] }), {
				status: 200,
			}),
		);
		const consoleLogSpy = spyOn(console, "log");

		// when
		await taskPlanCommand.run!({
			args: { id: "task-uuid-1", date: "today", at: undefined, _: [] },
			rawArgs: [],
		} as any);

		// then
		const fetchCall = fetchSpy.mock.calls[0];
		const requestBody = JSON.parse(fetchCall[1]?.body as string);
		expect(requestBody[0].date).toBe(expectedDate);
		expect(consoleLogSpy).toHaveBeenCalledWith(
			`✓ Scheduled task "task-uuid-1" for ${expectedDate}`,
		);

		consoleLogSpy.mockRestore();
	});

	it("schedules task with natural language date 'tomorrow'", async () => {
		// given
		const tomorrow = new Date();
		tomorrow.setDate(tomorrow.getDate() + 1);
		const expectedDate = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, "0")}-${String(tomorrow.getDate()).padStart(2, "0")}`;

		fetchSpy.mockResolvedValue(
			new Response(JSON.stringify({ success: true, message: null, data: [] }), {
				status: 200,
			}),
		);
		const consoleLogSpy = spyOn(console, "log");

		// when
		await taskPlanCommand.run!({
			args: { id: "task-uuid-1", date: "tomorrow", at: undefined, _: [] },
			rawArgs: [],
		} as any);

		// then
		const fetchCall = fetchSpy.mock.calls[0];
		const requestBody = JSON.parse(fetchCall[1]?.body as string);
		expect(requestBody[0].date).toBe(expectedDate);
		expect(consoleLogSpy).toHaveBeenCalledWith(
			`✓ Scheduled task "task-uuid-1" for ${expectedDate}`,
		);

		consoleLogSpy.mockRestore();
	});

	it("schedules task with date and time", async () => {
		// given
		fetchSpy.mockResolvedValue(
			new Response(JSON.stringify({ success: true, message: null, data: [] }), {
				status: 200,
			}),
		);
		const consoleLogSpy = spyOn(console, "log");

		// when
		await taskPlanCommand.run!({
			args: { id: "task-uuid-1", date: "2026-02-10", at: "21:00", _: [] },
			rawArgs: [],
		} as any);

		// then
		const fetchCall = fetchSpy.mock.calls[0];
		const requestBody = JSON.parse(fetchCall[1]?.body as string);
		expect(requestBody[0].date).toBe("2026-02-10");
		expect(requestBody[0].datetime).toBeTruthy();
		expect(requestBody[0].datetime_tz).toBeTruthy();
		expect(consoleLogSpy).toHaveBeenCalledWith(
			'✓ Scheduled task "task-uuid-1" for 2026-02-10 at 21:00',
		);

		consoleLogSpy.mockRestore();
	});

	it("schedules task with 'today' and time", async () => {
		// given
		const today = new Date();
		const expectedDate = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;

		fetchSpy.mockResolvedValue(
			new Response(JSON.stringify({ success: true, message: null, data: [] }), {
				status: 200,
			}),
		);
		const consoleLogSpy = spyOn(console, "log");

		// when
		await taskPlanCommand.run!({
			args: { id: "task-uuid-1", date: "today", at: "14:30", _: [] },
			rawArgs: [],
		} as any);

		// then
		const fetchCall = fetchSpy.mock.calls[0];
		const requestBody = JSON.parse(fetchCall[1]?.body as string);
		expect(requestBody[0].date).toBe(expectedDate);
		expect(requestBody[0].datetime).toBeTruthy();
		expect(requestBody[0].datetime_tz).toBeTruthy();
		expect(consoleLogSpy).toHaveBeenCalledWith(
			`✓ Scheduled task "task-uuid-1" for ${expectedDate} at 14:30`,
		);

		consoleLogSpy.mockRestore();
	});

	it("schedules task with 'tomorrow' and time", async () => {
		// given
		const tomorrow = new Date();
		tomorrow.setDate(tomorrow.getDate() + 1);
		const expectedDate = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, "0")}-${String(tomorrow.getDate()).padStart(2, "0")}`;

		fetchSpy.mockResolvedValue(
			new Response(JSON.stringify({ success: true, message: null, data: [] }), {
				status: 200,
			}),
		);
		const consoleLogSpy = spyOn(console, "log");

		// when
		await taskPlanCommand.run!({
			args: { id: "task-uuid-1", date: "tomorrow", at: "09:00", _: [] },
			rawArgs: [],
		} as any);

		// then
		const fetchCall = fetchSpy.mock.calls[0];
		const requestBody = JSON.parse(fetchCall[1]?.body as string);
		expect(requestBody[0].date).toBe(expectedDate);
		expect(requestBody[0].datetime).toBeTruthy();
		expect(requestBody[0].datetime_tz).toBeTruthy();
		expect(consoleLogSpy).toHaveBeenCalledWith(
			`✓ Scheduled task "task-uuid-1" for ${expectedDate} at 09:00`,
		);

		consoleLogSpy.mockRestore();
	});

	it("defaults to today when only --at is specified", async () => {
		// given
		const today = new Date();
		const expectedDate = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;

		fetchSpy.mockResolvedValue(
			new Response(JSON.stringify({ success: true, message: null, data: [] }), {
				status: 200,
			}),
		);
		const consoleLogSpy = spyOn(console, "log");

		// when
		await taskPlanCommand.run!({
			args: { id: "task-uuid-1", date: undefined, at: "16:30", _: [] },
			rawArgs: [],
		} as any);

		// then
		const fetchCall = fetchSpy.mock.calls[0];
		const requestBody = JSON.parse(fetchCall[1]?.body as string);
		expect(requestBody[0].date).toBe(expectedDate);
		expect(requestBody[0].datetime).toBeTruthy();
		expect(requestBody[0].datetime_tz).toBeTruthy();
		expect(consoleLogSpy).toHaveBeenCalledWith(
			`✓ Scheduled task "task-uuid-1" for ${expectedDate} at 16:30`,
		);

		consoleLogSpy.mockRestore();
	});

	it("exits with error for invalid date format", async () => {
		// given
		const consoleErrorSpy = spyOn(console, "error");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when/then
		await expect(
			taskPlanCommand.run!({
				args: { id: "task-uuid-1", date: "invalid-date", at: undefined, _: [] },
				rawArgs: [],
			} as any),
		).rejects.toThrow();

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			expect.stringContaining("Invalid date format"),
		);

		consoleErrorSpy.mockRestore();
		processExitSpy.mockRestore();
	});

	it("exits with error for invalid time format", async () => {
		// given
		const consoleErrorSpy = spyOn(console, "error");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when/then
		await expect(
			taskPlanCommand.run!({
				args: { id: "task-uuid-1", date: "2026-02-10", at: "invalid", _: [] },
				rawArgs: [],
			} as any),
		).rejects.toThrow();

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			expect.stringContaining("Invalid time format"),
		);

		consoleErrorSpy.mockRestore();
		processExitSpy.mockRestore();
	});

	it("exits with error when neither date nor at is specified", async () => {
		// given
		const consoleErrorSpy = spyOn(console, "error");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when/then
		await expect(
			taskPlanCommand.run!({
				args: { id: "task-uuid-1", date: undefined, at: undefined, _: [] },
				rawArgs: [],
			} as any),
		).rejects.toThrow();

		expect(consoleErrorSpy).toHaveBeenCalledWith(
			"Error: Either --date or --at must be specified.",
		);

		consoleErrorSpy.mockRestore();
		processExitSpy.mockRestore();
	});
});
