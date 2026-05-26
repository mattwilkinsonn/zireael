export function parseDuration(duration: string): number {
	const trimmed = duration.trim().toLowerCase();
	const match = trimmed.match(/^(\d+)\s*([mhdw])$/);

	if (!match) {
		throw new Error(
			`Invalid duration format: "${duration}". Expected format: <number><unit> (e.g., 1h, 2d, 1w)`,
		);
	}

	const value = parseInt(match[1]!, 10);
	const unit = match[2]!;

	const msPerMinute = 60 * 1000;
	const msPerHour = msPerMinute * 60;
	const msPerDay = msPerHour * 24;
	const msPerWeek = msPerDay * 7;

	switch (unit) {
		case "m":
			return value * msPerMinute;
		case "h":
			return value * msPerHour;
		case "d":
			return value * msPerDay;
		case "w":
			return value * msPerWeek;
		default:
			throw new Error(
				`Invalid duration unit: "${unit}". Supported units: m, h, d, w`,
			);
	}
}

export function parseDurationToSeconds(duration: string): number {
	return Math.floor(parseDuration(duration) / 1000);
}
