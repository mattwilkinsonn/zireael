/**
 * Records arriving via delta sync may carry tombstone markers — either
 * `deleted_at != null` (the universal signal) or `status === 9`
 * (Task-specific). The cache applies them by removing the matching
 * locally-stored records.
 */

type Tombstoneable = {
	id: string;
	deleted_at?: string | null;
	status?: number | null;
};

export function isTombstone<T extends Tombstoneable>(record: T): boolean {
	if (record.deleted_at != null) return true;
	if (record.status === 9) return true;
	return false;
}

/**
 * Partition incoming delta records into "tombstone IDs" and "upserts".
 * Returns the existing local records with tombstoned + about-to-be-upserted
 * IDs removed, plus the upsert array. The caller appends upserts to land
 * the final state.
 */
export function applyTombstones<T extends Tombstoneable>(
	existing: T[],
	incoming: T[],
	keyOf: (r: T) => string,
): { kept: T[]; upserts: T[] } {
	const tombstoneIds = new Set<string>();
	const upserts: T[] = [];
	for (const r of incoming) {
		if (isTombstone(r)) {
			tombstoneIds.add(keyOf(r));
		} else {
			upserts.push(r);
		}
	}
	const upsertIds = new Set(upserts.map(keyOf));
	const kept = existing.filter(
		(r) => !tombstoneIds.has(keyOf(r)) && !upsertIds.has(keyOf(r)),
	);
	return { kept, upserts };
}
