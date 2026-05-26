import type { AkiflowClient } from "./api/client";

export async function getDefaultCalendarId(
	client: AkiflowClient,
): Promise<string | null> {
	try {
		const response = await client.getTimeSlots({ limit: 10 });
		const timeSlots = response.data;

		if (timeSlots.length === 0) {
			return null;
		}

		return timeSlots[0]?.calendar_id ?? null;
	} catch {
		return null;
	}
}
