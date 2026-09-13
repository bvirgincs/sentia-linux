#!/bin/sh
# Sentia local inference readiness probe.
#
# Proves that the packaged model actually loads and generates, which a
# listening socket does not. Runs as a systemd oneshot after the runtime.
#
# Deliberately POSIX shell and curl rather than Python: the shipped AI stack
# must not pull a language runtime into the image for a health check.
set -eu

socket=""
max_wait_seconds=180
n_predict=16
prompt="Reply with exactly: OK"
report_path=""

usage() {
  echo "usage: $0 --socket PATH [--max-wait-seconds N] [--n-predict N]" >&2
  echo "          [--prompt TEXT] [--report-path PATH]" >&2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --socket) socket="$2"; shift 2 ;;
    --max-wait-seconds) max_wait_seconds="$2"; shift 2 ;;
    --n-predict) n_predict="$2"; shift 2 ;;
    --prompt) prompt="$2"; shift 2 ;;
    --report-path) report_path="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

[ -n "${socket}" ] || { usage; exit 2; }

# The report is diagnostic output, so a failure to write it must not mask the
# probe result, but it must not pass silently either.
write_report() {
  [ -n "${report_path}" ] || return 0
  directory="$(dirname "${report_path}")"
  if ! mkdir -p "${directory}" 2>/dev/null; then
    echo "readiness: cannot create ${directory}" >&2
    return 0
  fi
  if ! printf '%s\n' "$1" > "${report_path}.tmp" 2>/dev/null; then
    echo "readiness: cannot write ${report_path}" >&2
    return 0
  fi
  mv -f "${report_path}.tmp" "${report_path}"
}

json_string() {
  # Escape the few characters that would otherwise produce invalid JSON.
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/\t/\\t/g' |
    tr -d '\r' | sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g'
}

health_attempts=0
health_ready=0
last_health=""
started="$(date +%s)"
while [ "$(( $(date +%s) - started ))" -lt "${max_wait_seconds}" ]; do
  health_attempts=$(( health_attempts + 1 ))
  if last_health="$(curl --silent --show-error --max-time 10 \
      --unix-socket "${socket}" http://localhost/health 2>&1)"; then
    case "${last_health}" in
      *'"status"'*'"ok"'*) health_ready=1; break ;;
    esac
  fi
  sleep 1
done

if [ "${health_ready}" -ne 1 ]; then
  write_report "{\"socket\":\"$(json_string "${socket}")\",\
\"health_attempts\":${health_attempts},\"health_ready\":false,\
\"last_health_response\":\"$(json_string "${last_health}")\",\"success\":false}"
  echo "readiness: llama-server did not become healthy within ${max_wait_seconds}s" >&2
  exit 1
fi

generation_started="$(date +%s)"
request="$(printf '{"prompt":"%s","n_predict":%s,"temperature":0}' \
  "$(json_string "${prompt}")" "${n_predict}")"
completion="$(curl --silent --show-error --max-time 600 \
  --unix-socket "${socket}" \
  --header 'Content-Type: application/json' \
  --data "${request}" \
  http://localhost/completion 2>&1)" || completion=""
generation_seconds="$(( $(date +%s) - generation_started ))"

# A well-formed response whose content is empty is a failure, not an answer:
# that is exactly what a reasoning model returns when it thinks past its budget.
content="$(printf '%s' "${completion}" |
  sed -n 's/.*"content"[[:space:]]*:[[:space:]]*"\(\([^"\\]\|\\.\)*\)".*/\1/p')"
if [ -n "${content}" ]; then
  success=true
else
  success=false
fi

write_report "{\"socket\":\"$(json_string "${socket}")\",\
\"health_attempts\":${health_attempts},\"health_ready\":true,\
\"generation_seconds\":${generation_seconds},\
\"generation_response\":\"$(json_string "${completion}")\",\
\"success\":${success}}"

if [ "${success}" != "true" ]; then
  echo "readiness: local inference returned no content" >&2
  exit 1
fi

echo "readiness: local inference answered in ${generation_seconds}s"
exit 0
