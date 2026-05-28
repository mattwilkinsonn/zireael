#!/usr/bin/env bash
# Force every BSP split on the active space to vertical (left/right) and
# equalize per-leaf area. Result: all windows are equal-width columns,
# regardless of how the tree was shaped before.

export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

# Walk every horizontal split and flip it to vertical. Each --toggle split
# call flips one parent node; iterating windows whose split-type is
# horizontal hits every horizontal parent at least once. After a toggle, the
# affected sibling's split-type also updates, so we re-query each loop.
while true; do
	next=$(yabai -m query --windows --space |
		jq -r '[.[] | select(."is-floating"==false and ."is-minimized"==false and ."is-visible"==true and ."split-type"=="horizontal")] | .[0].id // empty')
	[[ -z $next ]] && break
	yabai -m window "$next" --toggle split 2>/dev/null || break
done

yabai -m space --balance
