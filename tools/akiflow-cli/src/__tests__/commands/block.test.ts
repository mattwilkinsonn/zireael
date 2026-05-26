import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import { block } from "../../commands/block";
import { createClient } from "../../lib/api/client";
import type { TimeSlot } from "../../lib/api/types";

describe("block command", () => {
	let mockGetTimeSlots: ReturnType<typeof spyOn>;
	let mockUpsertTasks: ReturnType<typeof spyOn>;

	beforeEach(() => {
		mockGetTimeSlots = spyOn(
			createClient().constructor.prototype,
			"getTimeSlots",
		);
		mockUpsertTasks = spyOn(
			createClient().constructor.prototype,
			"upsertTasks",
		);
	});

	afterEach(() => {
		mockGetTimeSlots.mockRestore();
		mockUpsertTasks.mockRestore();
	});

	const mockTimeSlots: TimeSlot[] = [
		{
			id: "1",
			user_id: 1,
			recurring_id: null,
			calendar_id: "cal1",
			label_id: null,
			section_id: null,
			status: "confirmed",
			title: "Morning standup",
			description: null,
			original_start_time: null,
			start_time: new Date(new Date().setHours(9, 0, 0)).toISOString(),
			end_time: new Date(new Date().setHours(10, 0, 0)).toISOString(),
			start_datetime_tz: new Date().toISOString(),
			recurrence: null,
			color: null,
			content: {},
			global_label_id_updated_at: null,
			global_created_at: new Date().toISOString(),
			global_updated_at: new Date().toISOString(),
			data: {},
			deleted_at: null,
		},
	];

	it("creates time block with 1h duration", async () => {
		// given
		mockGetTimeSlots.mockResolvedValue({
			success: true,
			message: null,
			data: mockTimeSlots,
		});

		const createdTask = {
			id: "test-id-1",
			title: "Deep Work",
			datetime: new Date(new Date().setHours(10, 0, 0)).toISOString(),
			datetime_tz: new Date().toISOString(),
			duration: 60 * 60 * 1000,
			global_created_at: new Date().toISOString(),
			global_updated_at: new Date().toISOString(),
		};

		mockUpsertTasks.mockResolvedValue({
			success: true,
			message: null,
			data: [createdTask],
		});

		const consoleLogSpy = spyOn(console, "log");

		// when
		await block.run!({
			args: { duration: "1h", title: "Deep Work", _: [] },
			rawArgs: [],
		} as never);

		// then
		expect(mockGetTimeSlots).toHaveBeenCalled();
		expect(mockUpsertTasks).toHaveBeenCalled();
		const createdPayload = mockUpsertTasks.mock.calls[0][0];
		expect(Array.isArray(createdPayload)).toBe(true);
		expect(createdPayload).toHaveLength(1);
		expect(createdPayload[0].title).toBe("Deep Work");
		expect(createdPayload[0].duration).toBe(60 * 60 * 1000);
		expect(createdPayload[0].calendar_id).toBe("cal1");

		expect(consoleLogSpy).toHaveBeenCalledWith(
			"✓ Time block created successfully",
		);
		expect(consoleLogSpy).toHaveBeenCalledWith("  ID: test-id-1");
		expect(consoleLogSpy).toHaveBeenCalledWith("  Title: Deep Work");
		expect(consoleLogSpy).toHaveBeenCalledWith(
			expect.stringMatching(/\s+Time: \d{2}:\d{2}/),
		);

		consoleLogSpy.mockRestore();
	});

	it("creates time block with 30m duration", async () => {
		// given
		mockGetTimeSlots.mockResolvedValue({
			success: true,
			message: null,
			data: mockTimeSlots,
		});

		const createdTask = {
			id: "test-id-2",
			title: "Review",
			datetime: new Date(new Date().setHours(10, 30, 0)).toISOString(),
			datetime_tz: new Date().toISOString(),
			duration: 30 * 60 * 1000,
			global_created_at: new Date().toISOString(),
			global_updated_at: new Date().toISOString(),
		};

		mockUpsertTasks.mockResolvedValue({
			success: true,
			message: null,
			data: [createdTask],
		});

		const consoleLogSpy = spyOn(console, "log");

		// when
		await block.run!({
			args: { duration: "30m", title: "Review", _: [] },
			rawArgs: [],
		} as never);

		// then
		expect(mockUpsertTasks).toHaveBeenCalled();
		const createdPayload = mockUpsertTasks.mock.calls[0][0];
		expect(createdPayload[0].duration).toBe(30 * 60 * 1000);
		expect(consoleLogSpy).toHaveBeenCalledWith(
			"✓ Time block created successfully",
		);

		consoleLogSpy.mockRestore();
	});

	it("handles invalid duration format", async () => {
		// given
		const consoleErrorSpy = spyOn(console, "error");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when
		try {
			await block.run!({
				args: { duration: "invalid", title: "Test", _: [] },
				rawArgs: [],
			} as never);
		} catch {}

		// then
		expect(consoleErrorSpy).toHaveBeenCalledWith(
			'Error: Invalid duration format: "invalid". Expected format: <number><unit> (e.g., 1h, 2d, 1w)',
		);
		expect(processExitSpy).toHaveBeenCalledWith(1);

		consoleErrorSpy.mockRestore();
		processExitSpy.mockRestore();
	});

	it("shows error when no free slot available for duration", async () => {
		// given
		const fullDaySlots: TimeSlot[] = [
			{
				id: "1",
				user_id: 1,
				recurring_id: null,
				calendar_id: "cal1",
				label_id: null,
				section_id: null,
				status: "confirmed",
				title: "Full day event",
				description: null,
				original_start_time: null,
				start_time: new Date(new Date().setHours(0, 0, 0)).toISOString(),
				end_time: new Date(new Date().setHours(23, 59, 59)).toISOString(),
				start_datetime_tz: new Date().toISOString(),
				recurrence: null,
				color: null,
				content: {},
				global_label_id_updated_at: null,
				global_created_at: new Date().toISOString(),
				global_updated_at: new Date().toISOString(),
				data: {},
				deleted_at: null,
			},
		];

		mockGetTimeSlots.mockResolvedValue({
			success: true,
			message: null,
			data: fullDaySlots,
		});

		const consoleErrorSpy = spyOn(console, "error");
		const consoleLogSpy = spyOn(console, "log");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when
		try {
			await block.run!({
				args: { duration: "1h", title: "Test", _: [] },
				rawArgs: [],
			} as never);
		} catch {}

		// then
		expect(consoleErrorSpy).toHaveBeenCalledWith(
			"Error: No available time slot for 1h today",
		);
		expect(consoleLogSpy).toHaveBeenCalledWith("\nAvailable free time slots:");
		expect(processExitSpy).toHaveBeenCalledWith(1);

		consoleErrorSpy.mockRestore();
		consoleLogSpy.mockRestore();
		processExitSpy.mockRestore();
	});

	it("handles authentication errors", async () => {
		// given
		const authError = new Error("Authentication failed");
		(authError as any).name = "AuthError";

		mockGetTimeSlots.mockRejectedValue(authError);

		const consoleErrorSpy = spyOn(console, "error");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when
		try {
			await block.run!({
				args: { duration: "1h", title: "Test", _: [] },
				rawArgs: [],
			} as never);
		} catch {}

		// then
		expect(mockGetTimeSlots).toHaveBeenCalled();
		expect(consoleErrorSpy).toHaveBeenCalledWith(
			"Error: Authentication failed. Please run 'af auth' to login.",
		);
		expect(processExitSpy).toHaveBeenCalledWith(1);

		consoleErrorSpy.mockRestore();
		processExitSpy.mockRestore();
	});
});
