#!/usr/bin/env bash
# Reset every BSP split ratio on the active space to 0.5. Restores default
# BSP proportions (3 windows → 50/25/25, 4+ → 50% primary + stacked side).
# Different from `yabai -m space --balance`, which equalizes per-leaf area.

export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

for id in $(yabai -m query --windows --space |
	jq -r '.[] | select(."is-floating"==false and ."is-minimized"==false and ."is-visible"==true) | .id'); do
	yabai -m window "$id" --ratio abs:0.5 2>/dev/null || true
done
