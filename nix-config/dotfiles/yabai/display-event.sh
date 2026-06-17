#!/usr/bin/env bash
# Debounce the burst of display events macOS fires on monitor connect/
# disconnect. A single physical plug-in produces a 10-30s storm of
# display_added / display_removed / display_moved events while macOS settles
# resolutions, reorders displays, and relocates windows. Running the full
# layout cascade (display-setup + rules + aw-layout) on every event amplifies
# the thrash — each pass force-moves windows against a target that is still
# moving. Instead, each event only stamps a "last seen" time and a single
# background waiter runs the cascade ONCE, after events have been quiet for
# QUIET_SECS.
#
# Wired in yabairc as the action for display_added/removed/moved.

set -u
export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

# Seconds of event silence before the layout cascade runs.
QUIET_SECS=3

STAMP="/tmp/yabai-display-event.stamp"
LOCK="/tmp/yabai-display-event.waiter.lock"

DISPLAY_SETUP="$HOME/.config/yabai/display-setup.sh"
RULES="$HOME/.config/yabai/rules.sh"
AW_LAYOUT="$HOME/.config/yabai/aw-layout.sh"

# Record the time of this event (epoch seconds). Every invocation does this,
# so the waiter below always sees the latest event time.
date +%s >"$STAMP"

# Only one waiter at a time. mkdir is atomic: if the lock dir already exists a
# waiter is running and will observe the timestamp we just wrote, so this
# event is accounted for and we exit immediately — UNLESS the lock is stale.
# The EXIT trap below can't fire if the waiter is SIGKILL'd (macOS memory
# pressure, `kill -9`), which would otherwise wedge the lock forever and stop
# every future cascade. Guard against that: if the recorded PID is no longer
# alive, steal the lock instead of surrendering to a dead waiter.
if ! mkdir "$LOCK" 2>/dev/null; then
	pid_file="$LOCK/pid"
	if [[ -f $pid_file ]] && kill -0 "$(cat "$pid_file" 2>/dev/null)" 2>/dev/null; then
		exit 0 # live waiter; it will observe our stamp
	fi
	# Stale lock from a dead waiter — reclaim it. A concurrent event may win
	# the race and steal it first; that's fine, the cascade is idempotent.
	rm -rf "$LOCK" 2>/dev/null || exit 0
	mkdir "$LOCK" 2>/dev/null || exit 0
fi
echo $$ >"$LOCK/pid"
trap 'rm -rf "$LOCK" 2>/dev/null' EXIT

while :; do
	# Wait until no event has landed for QUIET_SECS. The timestamp that
	# satisfies the quiet condition IS the one we settled on, so capture it
	# as acted_on directly — re-reading STAMP after the loop would reopen a
	# race where an event landing between the break and the read silently
	# re-anchors us to a still-settling display.
	while :; do
		acted_on=$(cat "$STAMP" 2>/dev/null || echo 0)
		now=$(date +%s)
		[[ $((now - acted_on)) -ge $QUIET_SECS ]] && break
		sleep 1
	done

	# Run the cascade, then check whether a fresh event arrived mid-cascade.
	# If so, loop and settle again so the tail-end event isn't dropped;
	# otherwise we're done.
	"$DISPLAY_SETUP"
	"$RULES"
	"$AW_LAYOUT"
	latest=$(cat "$STAMP" 2>/dev/null || echo 0)
	[[ $latest == "$acted_on" ]] && break
done
