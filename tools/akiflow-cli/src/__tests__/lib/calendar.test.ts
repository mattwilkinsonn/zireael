import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import { AkiflowClient } from "../../lib/api/client";
import type { TimeSlot } from "../../lib/api/types";
import { getDefaultCalendarId } from "../../lib/calendar";

describe("getDefaultCalendarId", () => {
	let mockGetTimeSlots: ReturnType<typeof spyOn>;

	beforeEach(() => {
		mockGetTimeSlots = spyOn(AkiflowClient.prototype, "getTimeSlots");
	});

	afterEach(() => {
		mockGetTimeSlots.mockRestore();
	});

	it("returns calendar_id from first time slot", async () => {
		// given
		const mockTimeSlots: TimeSlot[] = [
			{
				id: "1",
				user_id: 1,
				recurring_id: null,
				calendar_id: "cal-123",
				label_id: null,
				section_id: null,
				status: "confirmed",
				title: "Test event",
				description: null,
				original_start_time: null,
				start_time: new Date().toISOString(),
				end_time: new Date().toISOString(),
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
			data: mockTimeSlots,
		});

		const client = new AkiflowClient({
			credentials: { token: "test", clientId: "test" },
		});

		// when
		const result = await getDefaultCalendarId(client);

		// then
		expect(result).toBe("cal-123");
	});

	it("returns null when no time slots exist", async () => {
		// given
		mockGetTimeSlots.mockResolvedValue({
			success: true,
			message: null,
			data: [],
		});

		const client = new AkiflowClient({
			credentials: { token: "test", clientId: "test" },
		});

		// when
		const result = await getDefaultCalendarId(client);

		// then
		expect(result).toBeNull();
	});

	it("returns null when API call fails", async () => {
		// given
		mockGetTimeSlots.mockRejectedValue(new Error("API error"));

		const client = new AkiflowClient({
			credentials: { token: "test", clientId: "test" },
		});

		// when
		const result = await getDefaultCalendarId(client);

		// then
		expect(result).toBeNull();
	});
});
