# Sentia terminal integration for interactive Bash sessions.
#
# This must never change how ordinary commands behave. It installs a
# command_not_found_handle and an optional failed-command hook; both are
# advisory, neither ever executes a suggestion on its own.
#
# Set SENTIA_TERMINAL_ASSIST=disabled to opt out entirely.

case $- in
  *i*) ;;
  *) return ;;
esac

if [ -r /usr/share/sentia/shell/sentia-terminal-hook.bash ]; then
  . /usr/share/sentia/shell/sentia-terminal-hook.bash
fi
