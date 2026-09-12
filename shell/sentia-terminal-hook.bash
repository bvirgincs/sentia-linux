#!/usr/bin/env bash
# shellcheck shell=bash

__sentia_hook_exit_or_return() {
  return 0 2>/dev/null || exit 0
}

[[ "${_SENTIA_TERMINAL:-}" == "1" ]] || __sentia_hook_exit_or_return

_SENTIA_HOOK_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=/dev/null
source "${_SENTIA_HOOK_DIR}/sentia-command-not-found-lib.bash"

: "${SENTIA_TERMINAL_ASSIST_MODE:=command-not-found}"
: "${SENTIA_CAPTURE_MAX:=128}"

declare -ag __sentia_capture_events=()

__sentia_assist_enabled() {
  case "${SENTIA_TERMINAL_ASSIST_MODE}" in
    command-not-found|auto-suggest)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

__sentia_append_prompt_command() {
  local hook_name=${1:?hook name required}
  local pc_decl
  pc_decl=$(declare -p PROMPT_COMMAND 2>/dev/null || true)

  if [[ "$pc_decl" == declare\ -a* ]]; then
    local hook
    for hook in "${PROMPT_COMMAND[@]}"; do
      [[ "$hook" == "$hook_name" ]] && return 0
    done
    PROMPT_COMMAND+=("$hook_name")
    return 0
  fi

  case ";${PROMPT_COMMAND:-};" in
    *";${hook_name};"*) ;;
    *) PROMPT_COMMAND="${PROMPT_COMMAND:+${PROMPT_COMMAND};}${hook_name}" ;;
  esac
}

__sentia_capture_prompt() {
  local command_status=$?
  local command_line
  command_line=$(history 1 2>/dev/null | sed -E 's/^[[:space:]]*[0-9]+[[:space:]]+//')

  [[ -n "$command_line" ]] || return 0

  local ts
  ts=$(date -Is)
  __sentia_capture_events+=("${ts}\t${PWD}\t${command_status}\tcombined-stream\t${command_line}")

  local max_entries=${SENTIA_CAPTURE_MAX:-128}
  if (( ${#__sentia_capture_events[@]} > max_entries )); then
    __sentia_capture_events=("${__sentia_capture_events[@]: -$max_entries}")
  fi

  return 0
}

sentia_capture_dump() {
  printf '%s\n' "${__sentia_capture_events[@]}"
}

if [[ "${SENTIA_TERMINAL_CAPTURE:-0}" == "1" ]]; then
  __sentia_append_prompt_command "__sentia_capture_prompt"
fi

command_not_found_handle() {
  local original_line="$*"
  local missing_command=${1:-}

  if ! __sentia_assist_enabled; then
    printf 'bash: %s: command not found\n' "$missing_command" >&2
    return 127
  fi

  local classification
  classification=$(sentia_classify_not_found "$original_line")

  local class=""
  local suggestion=""
  local packages=""
  local ai_hint=""

  while IFS='=' read -r key value; do
    case "$key" in
      CLASS) class="$value" ;;
      SUGGESTION) suggestion="$value" ;;
      PACKAGES) packages="$value" ;;
      AI_HINT) ai_hint="$value" ;;
    esac
  done <<<"$classification"

  case "$class" in
    LIKELY_TYPO)
      printf 'sentia: command not found: %s\n' "$missing_command" >&2
      printf 'sentia: likely typo, try: %s\n' "$suggestion" >&2
      ;;
    MISSING_PACKAGE)
      printf 'sentia: command not found: %s\n' "$missing_command" >&2
      printf 'sentia: package candidate(s): %s\n' "$packages" >&2
      ;;
    NATURAL_LANGUAGE)
      printf 'sentia: that looks like natural language.\n' >&2
      printf 'sentia: route through AI explicitly (not auto-run): %s\n' "$ai_hint" >&2
      ;;
    *)
      printf 'sentia: command not found: %s\n' "$missing_command" >&2
      printf 'sentia: no safe automatic correction. Run `ai -- %q` if you want help.\n' "$original_line" >&2
      ;;
  esac

  return 127
}
