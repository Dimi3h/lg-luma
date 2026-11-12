#!/bin/sh
# Optional replacement of LG's screensaver with Luma's clock. The webosbrew hook has no .sh suffix.
STATE=/var/lib/luma-screensaver
# The cold-boot hook is a symlink to the copied $STATE/screensaver.sh: find luma-helper beside its target.
SCRIPT=$0
if [ -L "$SCRIPT" ]; then
  LINK=$(readlink "$SCRIPT") || exit 1
  case "$LINK" in /*) SCRIPT=$LINK ;; *) SCRIPT=$(dirname -- "$SCRIPT")/$LINK ;; esac
fi
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$SCRIPT")" && pwd) || exit 1
COMMAND=${1:-boot}
case "$COMMAND" in
  enable|disable|restore|status|apply|boot) ;;
  *) printf '%s\n' 'Usage: screensaver.sh enable [APP_DIR] | disable | status' >&2; exit 1 ;;
esac
[ "$(id -u)" = 0 ] || { printf '%s\n' 'Run as root.' >&2; exit 1; }
# boot only schedules the copied $STATE/screensaver.sh apply, which checks for the helper itself.
[ "$COMMAND" = boot ] || [ -x "$SCRIPT_DIR/luma-helper" ] ||
  { printf '%s\n' 'The native Luma TV helper is missing.' >&2; exit 1; }
if [ "$COMMAND" = enable ]; then
  printf '%s\n' \
    "Replacing LG's screensaver with Luma's clock will:" \
    "  1. Create $STATE with integration scripts, saved state and a staged screensaver app" \
    '     (a hidden web app with a copy of the clock page and its font).' \
    "  2. Bind-mount $STATE/app over /usr/palm/applications/com.webos.app.screensaver." \
    '  3. Temporarily block and release the screensaver so SAM rereads its manifest.' \
    '  4. Install /var/lib/webosbrew/init.d/10-luma-screensaver to reapply the mount at cold boot.' \
    "  5. Turn off Luma's own screensaver timer while the TV's screensaver shows Luma's clock." \
    '' \
    "LG's screensaver files remain underneath the mount; its timing settings stay with the TV." \
    "Restore LG's screensaver with: sh $STATE/screensaver.sh disable" \
    ''
  while :; do
    printf '%s' 'Apply these changes? [y/n]: '
    if ! IFS= read -r answer; then
      printf '\n%s\n' 'Cancelled. No screensaver changes applied.'
      exit 1
    fi
    case "$answer" in
      y|Y) break ;;
      n|N) printf '%s\n' 'Cancelled. No screensaver changes applied.'; exit 0 ;;
      *) printf '%s\n' 'Please answer y or n.' ;;
    esac
  done
fi
[ ! -L "$STATE" ] || exit 1
if [ ! -d "$STATE" ]; then mkdir -m 755 "$STATE" || exit 1; fi
[ "$(stat -c %u "$STATE")" = 0 ] || exit 1
if [ "$COMMAND" = boot ]; then
  # Apply detached, so Homebrew Channel moves on to the next hook at once.
  if [ "$(wc -c <"$STATE/boot.log" 2>/dev/null || echo 0)" -gt 65536 ]; then
    mv -f "$STATE/boot.log" "$STATE/boot.log.1"
  fi
  (
    printf 'Luma screensaver boot hook: %s, uptime %s s\n' "$(date '+%Y-%m-%d %H:%M:%S')" \
      "$(cut -d ' ' -f 1 /proc/uptime 2>/dev/null)"
    exec sh "$STATE/screensaver.sh" apply
  ) </dev/null >>"$STATE/boot.log" 2>&1 &
  exit 0
fi
exec 9>"$STATE/lock"
flock -n 9 || { printf '%s\n' 'The screensaver integration is already being changed.' >&2; exit 1; }
exec "$SCRIPT_DIR/luma-helper" screensaver "$COMMAND" "${2:-$SCRIPT_DIR}"
