#!/bin/bash
# Sentia live-image acceptance checks.
#
# This script is NOT part of the product. It is delivered to the guest on a
# separate read-only check ISO by tests/vm/live_probe.py, so that no test hook
# has to be shipped inside the Sentia image itself.
#
# Every check prints exactly one line:
#   CHECK <id> PASS <detail>
#   CHECK <id> FAIL <detail>
# The harness parses those lines; anything else is diagnostic noise.

export LC_ALL=C
export PAGER=cat
export SYSTEMD_COLORS=0
# The serial console login this runs from does not always provide the sbin
# directories, and several checks use tools that live there (runuser, ip).
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# Boot completion is asynchronous: the login prompt the harness waits for can
# appear while late units are still activating. Give the boot a bounded chance
# to settle before asserting anything about unit state.
timeout 300 systemctl is-system-running --wait >/dev/null 2>&1 || true

check() {
  local id="$1" detail="$2" status="$3"
  printf 'CHECK %s %s %s\n' "${id}" "${status}" "${detail//$'\n'/ | }"
}

# LIVE-IDENTITY: the image must identify itself as Sentia, not as Debian.
os_id="$(. /usr/lib/os-release && printf '%s' "${ID}")"
os_pretty="$(. /usr/lib/os-release && printf '%s' "${PRETTY_NAME}")"
if [[ "${os_id}" == "sentia" ]]; then
  check LIVE-IDENTITY "${os_pretty}" PASS
else
  check LIVE-IDENTITY "ID=${os_id} PRETTY_NAME=${os_pretty}" FAIL
fi

# The /etc path must resolve to the file Debian's base-files owns.
etc_target="$(readlink -f /etc/os-release || true)"
if [[ "${etc_target}" == "/usr/lib/os-release" ]]; then
  check LIVE-OSRELEASE-LINK "${etc_target}" PASS
else
  check LIVE-OSRELEASE-LINK "resolves to ${etc_target:-<missing>}" FAIL
fi

# LIVE-VENDOR: dpkg must report Sentia as the vendor with Debian as parent.
# dpkg-vendor ships in dpkg-dev, which a minimal desktop image has no reason to
# install, so read the origin file that dpkg-vendor itself reads.
vendor_origin="$(readlink -f /etc/dpkg/origins/default 2>/dev/null || true)"
vendor="$(sed -n 's/^Vendor:[[:space:]]*//p' "${vendor_origin}" 2>/dev/null | head -n 1)"
parent="$(sed -n 's/^Parent:[[:space:]]*//p' "${vendor_origin}" 2>/dev/null | head -n 1)"
if [[ "${vendor}" == "Sentia" && "${parent}" == "Debian" ]]; then
  check LIVE-VENDOR "vendor=${vendor} parent=${parent}" PASS
else
  check LIVE-VENDOR "vendor=${vendor} parent=${parent}" FAIL
fi

# LIVE-HOSTNAME: a Sentia live session must not present itself as "debian".
hostname_value="$(hostname)"
if [[ "${hostname_value}" == sentia* ]]; then
  check LIVE-HOSTNAME "${hostname_value}" PASS
else
  check LIVE-HOSTNAME "${hostname_value}" FAIL
fi

# LIVE-SYSTEMD: the boot must actually finish.
system_state="$(systemctl is-system-running 2>&1 || true)"
if [[ "${system_state}" == "running" ]]; then
  check LIVE-SYSTEMD "${system_state}" PASS
else
  check LIVE-SYSTEMD "${system_state}" FAIL
fi

failed_units="$(systemctl list-units --state=failed --no-legend --plain 2>/dev/null | awk '{print $1}' | tr '\n' ' ')"
if [[ -z "${failed_units// /}" ]]; then
  check LIVE-NO-FAILED-UNITS "none" PASS
else
  check LIVE-NO-FAILED-UNITS "${failed_units}" FAIL
  # Diagnostic noise, not a result line: a failed unit is useless without the
  # reason, and re-running the whole probe to learn it costs a full boot.
  for unit in ${failed_units}; do
    echo "--- diagnostics for ${unit} ---"
    systemctl status --no-pager --full --lines=0 "${unit}" 2>&1 | head -n 12
    journalctl --no-pager --no-hostname -u "${unit}" --lines=40 2>&1 | tail -n 40
    echo "--- end diagnostics for ${unit} ---"
  done
fi

# LIVE-GRAPHICAL: graphical.target is the real gate, not a console message.
graphical_state="$(systemctl is-active graphical.target 2>&1 || true)"
if [[ "${graphical_state}" == "active" ]]; then
  check LIVE-GRAPHICAL "graphical.target ${graphical_state}" PASS
else
  check LIVE-GRAPHICAL "graphical.target ${graphical_state}" FAIL
fi

for unit_check in LIVE-LIGHTDM:lightdm.service LIVE-NETWORKMANAGER:NetworkManager.service; do
  id="${unit_check%%:*}"
  unit="${unit_check##*:}"
  state="$(systemctl is-active "${unit}" 2>&1 || true)"
  if [[ "${state}" == "active" ]]; then
    check "${id}" "${unit} ${state}" PASS
  else
    check "${id}" "${unit} ${state}" FAIL
  fi
done

# LIVE-XORG: LightDM being active is not proof that a display server runs.
if pgrep -a Xorg >/dev/null 2>&1; then
  check LIVE-XORG "$(pgrep -a Xorg | head -n 1)" PASS
else
  check LIVE-XORG "no Xorg process" FAIL
fi

# LIVE-XSESSION: the greeter must have an Xfce session to offer.
if [[ -f /usr/share/xsessions/xfce.desktop ]]; then
  check LIVE-XSESSION "/usr/share/xsessions/xfce.desktop" PASS
else
  check LIVE-XSESSION "no xfce.desktop session file" FAIL
fi

# The applications the live image exists to provide.
for bin_check in LIVE-BROWSER:chromium LIVE-INSTALLER:calamares LIVE-FILEMANAGER:thunar; do
  id="${bin_check%%:*}"
  binary="${bin_check##*:}"
  path="$(command -v "${binary}" 2>/dev/null || true)"
  if [[ -n "${path}" ]]; then
    check "${id}" "${path}" PASS
  else
    check "${id}" "${binary} not installed" FAIL
  fi
done

# LIVE-CHROMIUM-RUNS: the browser must execute as an ordinary user, not merely
# be present on disk.
chromium_version="$(chromium --version 2>&1 | head -n 1 || true)"
if [[ "${chromium_version}" == *Chromium* ]]; then
  check LIVE-CHROMIUM-RUNS "${chromium_version}" PASS
else
  check LIVE-CHROMIUM-RUNS "${chromium_version:-no output}" FAIL
fi

# LIVE-SENTIA-PACKAGES: the Sentia overlay must be present in the image.
missing_packages=""
for package in sentia-base sentia-desktop sentia-release sentia-archive-keyring \
  sentia-repository-config sentia-offline-repository sentia-calamares-settings; do
  if ! dpkg-query -W -f='${Status}' "${package}" 2>/dev/null | grep -q "install ok installed"; then
    missing_packages="${missing_packages}${package} "
  fi
done
if [[ -z "${missing_packages}" ]]; then
  check LIVE-SENTIA-PACKAGES "all present" PASS
else
  check LIVE-SENTIA-PACKAGES "missing: ${missing_packages}" FAIL
fi

# LIVE-APT-NO-STABLE-ALIAS: installed sources must name literal trixie suites so
# that a future Debian release cannot silently upgrade Sentia users.
apt_sources="$(cat /etc/apt/sources.list /etc/apt/sources.list.d/* 2>/dev/null || true)"
if grep -qE '^[[:space:]]*(Suites:.*\bstable\b|deb .*[[:space:]]stable[[:space:]])' <<<"${apt_sources}"; then
  check LIVE-APT-NO-STABLE-ALIAS "unversioned stable suite configured" FAIL
else
  check LIVE-APT-NO-STABLE-ALIAS "literal suites only" PASS
fi

if grep -qs "Signed-By" /etc/apt/sources.list.d/sentia.sources; then
  check LIVE-APT-SENTIA-SIGNED-BY "$(grep -h 'Signed-By' /etc/apt/sources.list.d/sentia.sources)" PASS
else
  check LIVE-APT-SENTIA-SIGNED-BY "sentia.sources missing or has no Signed-By" FAIL
fi

# LIVE-APT-OFFLINE-ARCHIVE: apt must read the signed local Sentia archive with
# no network at all, which is what an offline installation depends on.
apt_update_output="$(sudo apt-get \
  -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/sentia.sources \
  -o Dir::Etc::sourceparts=/dev/null update 2>&1 || true)"
if grep -q "sentia" <<<"${apt_update_output}" && ! grep -qi "^E:" <<<"${apt_update_output}"; then
  check LIVE-APT-OFFLINE-ARCHIVE "${apt_update_output}" PASS
else
  check LIVE-APT-OFFLINE-ARCHIVE "${apt_update_output}" FAIL
fi

# LIVE-NO-BUILD-ARCHIVE: build-time trust must not be shipped in the image.
leaked=""
[[ -e /srv/sentia-build-archive ]] && leaked="${leaked}/srv/sentia-build-archive "
[[ -e /etc/apt/sources.list.d/sentia-overlay.list ]] && leaked="${leaked}/etc/apt/sources.list.d/sentia-overlay.list "
if [[ -z "${leaked}" ]]; then
  check LIVE-NO-BUILD-ARCHIVE "clean" PASS
else
  check LIVE-NO-BUILD-ARCHIVE "leaked: ${leaked}" FAIL
fi

# ---------------------------------------------------------------------------
# Local AI. These are the checks that distinguish a Sentia image from a plain
# Debian live image, so they are deliberately concrete: a binary, a weights
# file, a running service and a real answer produced with networking down.
# ---------------------------------------------------------------------------

MODEL_PATH=/usr/share/sentia/models/granite-4.2-3b/granite-4.2-3b-Q4_K_M.gguf
LLAMA_SOCKET=/run/sentia-inference/llama.sock
BROKER_SOCKET=/run/sentia-local/broker.sock

# AI-RUNTIME-BINARY: the packaged llama.cpp server must actually be present.
# The package once built cleanly while containing no server at all, so this is
# checked directly rather than inferred from the package being installed.
if [[ -x /usr/bin/llama-server ]]; then
  check AI-RUNTIME-BINARY "$(/usr/bin/llama-server --version 2>&1 | head -1)" PASS
else
  check AI-RUNTIME-BINARY "/usr/bin/llama-server missing or not executable" FAIL
fi

# AI-MODEL-PRESENT: the weights must ship in the image, not be downloaded.
if [[ -r "${MODEL_PATH}" ]]; then
  model_bytes="$(stat -c %s "${MODEL_PATH}")"
  if [[ "${model_bytes}" == "2244011552" ]]; then
    check AI-MODEL-PRESENT "${model_bytes} bytes" PASS
  else
    check AI-MODEL-PRESENT "unexpected size ${model_bytes}" FAIL
  fi
else
  check AI-MODEL-PRESENT "${MODEL_PATH} missing or unreadable" FAIL
fi

# AI-RUNTIME-ACTIVE: loading a 2.2 GB model on an emulated CPU is slow, so wait
# rather than sampling once. A failure here must not be mistaken for "slow".
llama_state=""
for _ in $(seq 1 60); do
  llama_state="$(systemctl is-active sentia-local-llama.service 2>&1)"
  [[ "${llama_state}" == "active" || "${llama_state}" == "failed" ]] && break
  sleep 5
done
if [[ "${llama_state}" == "active" ]]; then
  check AI-RUNTIME-ACTIVE "sentia-local-llama.service active" PASS
else
  check AI-RUNTIME-ACTIVE "state=${llama_state}: $(systemctl show -p Result -p ExecMainStatus --value sentia-local-llama.service 2>&1 | tr '\n' ' ')" FAIL
fi

# AI-RUNTIME-SOCKET: the runtime must listen on a private local socket and must
# not be reachable from anywhere else.
# The mode is asserted, not merely reported: sentia-local-broker refuses to use
# a runtime socket that is not exactly 0600 in a 0700 directory, so a permissive
# umask here disables local AI while every unit still reports active.
if [[ -S "${LLAMA_SOCKET}" ]]; then
  socket_mode="$(stat -c '%a %U:%G' "${LLAMA_SOCKET}")"
  directory_mode="$(stat -c '%a %U:%G' "$(dirname "${LLAMA_SOCKET}")")"
  if [[ "${socket_mode}" == "600 sentia-inference:sentia-inference" \
     && "${directory_mode}" == "700 sentia-inference:sentia-inference" ]]; then
    check AI-RUNTIME-SOCKET "${socket_mode} in ${directory_mode}" PASS
  else
    check AI-RUNTIME-SOCKET "socket=${socket_mode} dir=${directory_mode}" FAIL
  fi
else
  check AI-RUNTIME-SOCKET "${LLAMA_SOCKET} is not a socket" FAIL
fi

# AI-RUNTIME-NOT-EXPOSED: no inference port may be listening on a real address.
exposed="$(ss -Hltnp 2>/dev/null | awk '{print $4}' | grep -vE '^(127\.|\[::1\]|\*:631)' || true)"
if [[ -z "${exposed}" ]]; then
  check AI-RUNTIME-NOT-EXPOSED "no externally bound listeners" PASS
else
  check AI-RUNTIME-NOT-EXPOSED "listening: ${exposed//$'\n'/ }" FAIL
fi

# AI-RUNTIME-READY: the shipped readiness probe performs a real generation over
# the runtime socket. Checking it separates a broken model or runtime from a
# broken broker or router, which otherwise present the same symptom.
readiness_report=/var/log/sentia-local-llama/readiness.json
readiness_result="$(systemctl show sentia-local-llama-readiness.service -p Result --value 2>&1)"
if [[ "${readiness_result}" == "success" ]] && grep -q '"success":true' "${readiness_report}" 2>/dev/null; then
  check AI-RUNTIME-READY "$(sed -n 's/.*\("generation_seconds":[0-9]*\).*/\1/p' "${readiness_report}")" PASS
else
  check AI-RUNTIME-READY "result=${readiness_result} report=$(head -c 300 "${readiness_report}" 2>&1)" FAIL
fi

# AI-BROKER-ACTIVE: the peer-authenticated broker is the only path users get.
broker_state="$(systemctl is-active sentia-local-broker.service 2>&1)"
if [[ "${broker_state}" == "active" ]]; then
  check AI-BROKER-ACTIVE "sentia-local-broker.service active" PASS
else
  check AI-BROKER-ACTIVE "state=${broker_state}" FAIL
fi

# AI-BROKER-SOCKET: the broker path must be reachable by an ordinary login
# session. It was once bound 0660 root of the sentia-inference group, which no
# desktop account is a member of, so every AI request failed with EACCES while
# both services still reported active. Authorisation comes from SO_PEERCRED.
if [[ -S "${BROKER_SOCKET}" ]]; then
  broker_mode="$(stat -c '%a' "${BROKER_SOCKET}")"
  if [[ "${broker_mode}" == "666" ]]; then
    check AI-BROKER-SOCKET "${broker_mode} $(stat -c '%U:%G' "${BROKER_SOCKET}")" PASS
  else
    check AI-BROKER-SOCKET "mode=${broker_mode} is not user-reachable" FAIL
  fi
else
  check AI-BROKER-SOCKET "${BROKER_SOCKET} is not a socket" FAIL
fi
router_socket="/run/user/$(id -u "${SENTIA_TEST_USER:-user}" 2>/dev/null || echo 1000)/sentia/router.sock"
if [[ -S "${router_socket}" ]]; then
  check AI-ROUTER-SOCKET "$(stat -c '%a %U' "${router_socket}")" PASS
else
  check AI-ROUTER-SOCKET "${router_socket} missing" FAIL
fi

# AI-OFFLINE-ANSWER: the product claim. Take the network down first so that a
# remote provider cannot possibly be answering, then ask a real question as the
# unprivileged desktop user and require a non-empty answer.
for iface in $(ls /sys/class/net | grep -v '^lo$'); do
  ip link set "${iface}" down 2>/dev/null || true
done
ai_user="${SENTIA_TEST_USER:-user}"
ai_uid="$(id -u "${ai_user}")"

# This script runs as the live user over the serial console, so runuser is not
# available to it; sudo is the path live-config grants.
as_ai_user() {
  if [[ "$(id -u)" == "${ai_uid}" ]]; then
    env XDG_RUNTIME_DIR="/run/user/${ai_uid}" \
      DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${ai_uid}/bus" "$@"
  else
    sudo -n -u "${ai_user}" env XDG_RUNTIME_DIR="/run/user/${ai_uid}" \
      DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${ai_uid}/bus" "$@"
  fi
}

# LIVE-NO-FAILED-USER-UNITS: the router is a user unit, so the system manager's
# failed-unit list says nothing about it. A failed user unit was invisible to
# every check here until it showed up as a refused connection.
failed_user="$(as_ai_user systemctl --user --failed --no-legend 2>&1 |
  awk '{print $1}' | tr '\n' ' ')"
if [[ -z "${failed_user// /}" ]]; then
  check LIVE-NO-FAILED-USER-UNITS "none" PASS
else
  check LIVE-NO-FAILED-USER-UNITS "${failed_user}" FAIL
fi

# The per-user router is socket-activated and starts with the session, so it can
# still be coming up when the serial console reaches a prompt. Wait for it
# explicitly: an earlier run reported a refused connection that was only this
# race, which is indistinguishable from a real fault in the result line.
router_wait=0
until [[ "$(as_ai_user systemctl --user is-active sentia-router.socket 2>&1)" == "active" ]]; do
  if [[ "${router_wait}" -ge 60 ]]; then
    break
  fi
  sleep 2
  router_wait=$((router_wait + 2))
done

# The exit status is the result, not a keyword search of the output: an earlier
# version accepted "runuser: may not be used by non-root users" as an answer.
ai_output="$(as_ai_user timeout 600 \
  ai --policy LOCAL_ONLY 'name the command that lists open files' 2>&1)"
ai_status=$?
if [[ "${ai_status}" -eq 0 && -n "${ai_output//[[:space:]]/}" ]]; then
  check AI-OFFLINE-ANSWER "${ai_output:0:300}" PASS
else
  check AI-OFFLINE-ANSWER "status=${ai_status} ${ai_output:0:300}" FAIL
  # A refused router connection costs a whole boot cycle to diagnose without
  # this, and the reason is almost always in the user manager's own journal.
  echo "--- sentia-router diagnostics ---"
  as_ai_user ls -la "/run/user/${ai_uid}/sentia/" 2>&1 | head -10 || true
  # Which process, if any, actually holds a listening socket on that path. A
  # refused connection to an existing socket file means nobody does.
  ss -lxp 2>&1 | grep -a "router.sock" || echo "ss: no listener on router.sock"
  as_ai_user systemctl --user show sentia-router.service \
    -p ExecMainPID -p NRestarts -p Result -p ActiveState 2>&1 || true
  as_ai_user systemctl --user status sentia-router.socket sentia-router.service \
    --no-pager -l 2>&1 | head -40 || true
  as_ai_user journalctl --user -u sentia-router.service -u sentia-router.socket \
    -b --no-pager -n 60 2>&1 | tail -60 || true
  echo "--- end sentia-router diagnostics ---"
fi
for iface in $(ls /sys/class/net | grep -v '^lo$'); do
  ip link set "${iface}" up 2>/dev/null || true
done

echo "SENTIA_CHECKS_COMPLETE"
