import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import { cal } from "../../commands/cal";
import { createClient } from "../../lib/api/client";
import type { TimeSlot } from "../../lib/api/types";

describe("cal command", () => {
	let mockGetTimeSlots: ReturnType<typeof spyOn>;

	beforeEach(() => {
		mockGetTimeSlots = spyOn(
			createClient().constructor.prototype,
			"getTimeSlots",
		);
	});

	afterEach(() => {
		mockGetTimeSlots.mockRestore();
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
		{
			id: "2",
			user_id: 1,
			recurring_id: null,
			calendar_id: "cal1",
			label_id: null,
			section_id: null,
			status: "confirmed",
			title: "Deep work",
			description: null,
			original_start_time: null,
			start_time: new Date(new Date().setHours(14, 0, 0)).toISOString(),
			end_time: new Date(new Date().setHours(16, 0, 0)).toISOString(),
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

	it("lists today's events in timeline format", async () => {
		// given
		mockGetTimeSlots.mockResolvedValue({
			success: true,
			message: null,
			data: mockTimeSlots,
		});

		const consoleLogSpy = spyOn(console, "log");

		// when
		await cal.run!({
			args: { free: false, _: [] } as never,
			rawArgs: [],
		} as never);

		// then
		expect(mockGetTimeSlots).toHaveBeenCalled();
		expect(consoleLogSpy).toHaveBeenCalled();
		const output = consoleLogSpy.mock.calls.join("\n");
		expect(output).toContain("Today's Schedule");
		expect(output).toContain("09:00");
		expect(output).toContain("10:00");
		expect(output).toContain("Morning standup");
		expect(output).toContain("14:00");
		expect(output).toContain("16:00");
		expect(output).toContain("Deep work");

		consoleLogSpy.mockRestore();
	});

	it("lists free time slots when --free flag is used", async () => {
		// given
		mockGetTimeSlots.mockResolvedValue({
			success: true,
			message: null,
			data: mockTimeSlots,
		});

		const consoleLogSpy = spyOn(console, "log");

		// when
		await cal.run!({
			args: { free: true, _: [] } as never,
			rawArgs: [],
		} as never);

		// then
		expect(mockGetTimeSlots).toHaveBeenCalled();
		expect(consoleLogSpy).toHaveBeenCalled();
		const output = consoleLogSpy.mock.calls.join("\n");
		expect(output).toContain("Free Time Slots Today");
		expect(output).toContain("available");

		consoleLogSpy.mockRestore();
	});

	it("shows no events message when schedule is empty", async () => {
		// given
		mockGetTimeSlots.mockResolvedValue({
			success: true,
			message: null,
			data: [],
		});

		const consoleLogSpy = spyOn(console, "log");

		// when
		await cal.run!({
			args: { free: false, _: [] } as never,
			rawArgs: [],
		} as never);

		// then
		expect(mockGetTimeSlots).toHaveBeenCalled();
		expect(consoleLogSpy).toHaveBeenCalledWith(
			"No events or time blocks scheduled for today.",
		);

		consoleLogSpy.mockRestore();
	});

	it("shows free slots header when minimal slots available", async () => {
		// given
		// Even with a full day event (00:00-23:59:59), a tiny gap may exist
		// due to Date precision - this tests the free slots display format
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

		const consoleLogSpy = spyOn(console, "log");

		// when
		await cal.run!({
			args: { free: true, _: [] } as never,
			rawArgs: [],
		} as never);

		// then
		expect(mockGetTimeSlots).toHaveBeenCalled();
		const output = consoleLogSpy.mock.calls.join("\n");
		expect(output).toContain("Free Time Slots Today");

		consoleLogSpy.mockRestore();
	});

	it("handles authentication errors", async () => {
		// given
		const authError = new Error("Authentication failed");
		authError.name = "AuthError";
		mockGetTimeSlots.mockRejectedValue(authError);

		const consoleErrorSpy = spyOn(console, "error");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when
		try {
			await cal.run!({
				args: { free: false, _: [] } as never,
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

	it("handles network errors", async () => {
		// given
		const networkError = new Error("Connection failed");
		networkError.name = "NetworkError";
		mockGetTimeSlots.mockRejectedValue(networkError);

		const consoleErrorSpy = spyOn(console, "error");
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("process.exit");
		});

		// when
		try {
			await cal.run!({
				args: { free: false, _: [] } as never,
				rawArgs: [],
			} as never);
		} catch {}

		// then
		expect(mockGetTimeSlots).toHaveBeenCalled();
		expect(consoleErrorSpy).toHaveBeenCalledWith(
			"Error: Failed to fetch calendar",
			"Connection failed",
		);
		expect(processExitSpy).toHaveBeenCalledWith(1);

		consoleErrorSpy.mockRestore();
		processExitSpy.mockRestore();
	});
});
