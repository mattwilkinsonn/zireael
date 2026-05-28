#!/usr/bin/env bash
# AW (3440w) default layout:
#   Left  — Akiflow + Linear stacked
#   Right — Arc
# Idempotent: safe to run multiple times. Silent no-op if AW isn't connected
# or none of the three target apps are open.
#
# Triggers (wired in yabairc):
#   - application_launched / window_created signal for Akiflow, Linear, Arc
#   - display_added / display_moved signal (G9 de-PIP can dump AW windows
#     onto G9 — this script claws them back)

set -u
export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

LOCK="/tmp/yabai-aw-layout.lock"
if ! mkdir "$LOCK" 2>/dev/null; then
	exit 0
fi
trap 'rmdir "$LOCK" 2>/dev/null' EXIT

# Resolve AW display by resolution (stable per monitor, unlike UUID which can
# regenerate on reconnect).
displays=$(yabai -m query --displays 2>/dev/null) || exit 0
aw_idx=$(printf '%s' "$displays" | jq -r '[.[] | select(.frame.w == 3440)] | .[0].index // empty')
[[ -z $aw_idx ]] && exit 0

aw_space=$(printf '%s' "$displays" | jq -r --argjson i "$aw_idx" \
	'.[] | select(.index == $i) | .spaces[0] // empty')
[[ -z $aw_space ]] && exit 0

# Look up an app's window globally (any display, any space). macOS sometimes
# moves AW-tagged windows to a different display on geometry changes (e.g.
# G9 enters/exits PIP mode and momentarily resigns its primary slot —
# windows on AW dump to G9). Querying globally + force-moving back to AW
# keeps the layout sticky across these events. Falls back to space-scoped
# query for apps that may have stale windows from a prior session.
win_id_for() {
	yabai -m query --windows 2>/dev/null |
		jq -r --arg app "$1" '[.[] | select(.app == $app and ."is-minimized"==false)] | .[0].id // empty'
}

akiflow=$(win_id_for "Akiflow")
linear=$(win_id_for "Linear")
arc=$(win_id_for "Arc")

# Need at least one of the three open anywhere.
[[ -z $akiflow && -z $linear && -z $arc ]] && exit 0

# Pull each target window back onto the AW space if it isn't already
# there. yabai's `--space` move retiles the destination space's BSP tree
# after, so the warp/stack steps below land on the correct layout.
for id in "$akiflow" "$linear" "$arc"; do
	[[ -z $id ]] && continue
	current_space=$(yabai -m query --windows --window "$id" 2>/dev/null |
		jq -r '.space // empty')
	if [[ -n $current_space && $current_space != "$aw_space" ]]; then
		yabai -m window "$id" --space "$aw_space" 2>/dev/null || true
	fi
done

# Untile any of the three that happen to be floating (e.g. user manually
# floated Arc earlier and we want to reset).
for id in "$akiflow" "$linear" "$arc"; do
	[[ -z $id ]] && continue
	is_floating=$(yabai -m query --windows --window "$id" 2>/dev/null | jq -r '."is-floating" // false')
	[[ $is_floating == "true" ]] && yabai -m window "$id" --toggle float 2>/dev/null || true
done

# Left anchor = Akiflow if present, else Linear. Arc → east of it.
if [[ -n $akiflow ]]; then
	left_anchor="$akiflow"
elif [[ -n $linear ]]; then
	left_anchor="$linear"
else
	left_anchor=""
fi

# Stack Linear onto Akiflow (only when both are open and distinct).
if [[ -n $akiflow && -n $linear && $akiflow != "$linear" ]]; then
	yabai -m window "$linear" --stack "$akiflow" 2>/dev/null || true
fi

# Place Arc east of the left anchor. `--warp <target>` moves the window into
# the same subtree as target; subsequent `--warp east` ensures the split is
# left|right with Arc on the right.
if [[ -n $arc && -n $left_anchor && $arc != "$left_anchor" ]]; then
	yabai -m window "$arc" --warp "$left_anchor" 2>/dev/null || true
fi

# 50/50 split between left column and right column.
if [[ -n $left_anchor ]]; then
	yabai -m window "$left_anchor" --ratio abs:0.5 2>/dev/null || true
fi
