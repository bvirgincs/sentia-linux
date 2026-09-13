#!/bin/bash
# Sentia installed-system acceptance checks.
#
# Run on a machine booted from its own disk with the installation media
# detached. Delivered on a read-only check ISO, so nothing here ships inside
# Sentia.
#
# Every check prints exactly one line:
#   CHECK <id> PASS <detail>
#   CHECK <id> FAIL <detail>

export LC_ALL=C
export PAGER=cat
export SYSTEMD_COLORS=0
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

check() {
  local id="$1" detail="$2" status="$3"
  printf 'CHECK %s %s %s\n' "${id}" "${status}" "${detail//$'\n'/ | }"
}

timeout 300 systemctl is-system-running --wait >/dev/null 2>&1 || true

# DISK-IDENTITY: the installed system is Sentia, not the Debian it came from.
os_id="$(. /usr/lib/os-release && printf '%s' "${ID}")"
os_pretty="$(. /usr/lib/os-release && printf '%s' "${PRETTY_NAME}")"
if [[ "${os_id}" == "sentia" ]]; then
  check DISK-IDENTITY "${os_pretty}" PASS
else
  check DISK-IDENTITY "ID=${os_id} PRETTY_NAME=${os_pretty}" FAIL
fi

# DISK-NOT-LIVE: booting from disk must not be the live image in disguise.
if [[ -d /run/live/medium || -d /lib/live/mount ]]; then
  check DISK-NOT-LIVE "live medium still mounted" FAIL
else
  check DISK-NOT-LIVE "no live medium" PASS
fi

# DISK-HOSTNAME: the hostname the installer was given must have been applied.
check DISK-HOSTNAME "$(hostname)" \
  "$([[ "$(hostname)" == "sentia-installed" ]] && echo PASS || echo FAIL)"

# DISK-NO-LIVE-PACKAGES: live-only packages must have been removed.
live_residue="$(dpkg-query -W -f='${Package} ${Status}\n' \
  sentia-live live-boot live-config calamares 2>/dev/null |
  awk '$3 == "installed" {print $1}' | tr '\n' ' ')"
if [[ -z "${live_residue// /}" ]]; then
  check DISK-NO-LIVE-PACKAGES "none installed" PASS
else
  check DISK-NO-LIVE-PACKAGES "${live_residue}" FAIL
fi

# DISK-NO-LIVE-USER: the live account must not exist on the installed system.
if getent passwd user >/dev/null 2>&1; then
  check DISK-NO-LIVE-USER "the live account survived installation" FAIL
else
  check DISK-NO-LIVE-USER "absent" PASS
fi

# DISK-USER: the account the installer created must exist with sudo rights.
if id sentia >/dev/null 2>&1 && id -nG sentia | tr ' ' '\n' | grep -qx sudo; then
  check DISK-USER "sentia in $(id -nG sentia)" PASS
else
  check DISK-USER "missing account or sudo group" FAIL
fi

# DISK-SYSTEMD / DISK-NO-FAILED-UNITS: the boot must actually finish cleanly.
system_state="$(systemctl is-system-running 2>&1 || true)"
if [[ "${system_state}" == "running" ]]; then
  check DISK-SYSTEMD "${system_state}" PASS
else
  check DISK-SYSTEMD "${system_state}" FAIL
fi

failed_units="$(systemctl list-units --state=failed --no-legend --plain 2>/dev/null |
  awk '{print $1}' | tr '\n' ' ')"
if [[ -z "${failed_units// /}" ]]; then
  check DISK-NO-FAILED-UNITS "none" PASS
else
  check DISK-NO-FAILED-UNITS "${failed_units}" FAIL
  for unit in ${failed_units}; do
    echo "--- diagnostics for ${unit} ---"
    systemctl status --no-pager --full --lines=0 "${unit}" 2>&1 | head -n 12
    journalctl --no-pager --no-hostname -u "${unit}" --lines=40 2>&1 | tail -n 40
    echo "--- end diagnostics for ${unit} ---"
  done
fi

# DISK-BOOTLOADER: the system must have booted through its own ESP.
if [[ -d /sys/firmware/efi ]] && findmnt -n /boot/efi >/dev/null 2>&1; then
  check DISK-BOOTLOADER "UEFI with $(findmnt -n -o SOURCE /boot/efi)" PASS
else
  check DISK-BOOTLOADER "no EFI boot or no mounted ESP" FAIL
fi

# DISK-SWAP: the installer was asked for a swap file.
if swapon --noheadings --show=NAME 2>/dev/null | grep -q .; then
  check DISK-SWAP "$(swapon --noheadings --show=NAME,SIZE | tr '\n' ' ')" PASS
else
  check DISK-SWAP "no swap active" FAIL
fi

for unit_check in DISK-LIGHTDM:lightdm.service DISK-NETWORKMANAGER:NetworkManager.service; do
  id="${unit_check%%:*}"
  unit="${unit_check##*:}"
  state="$(systemctl is-active "${unit}" 2>&1 || true)"
  if [[ "${state}" == "active" ]]; then
    check "${id}" "${unit} ${state}" PASS
  else
    check "${id}" "${unit} ${state}" FAIL
  fi
done

graphical_state="$(systemctl is-active graphical.target 2>&1 || true)"
check DISK-GRAPHICAL "graphical.target ${graphical_state}" \
  "$([[ "${graphical_state}" == "active" ]] && echo PASS || echo FAIL)"

# DISK-BROWSER: Chromium must run, not merely be installed.
chromium_version="$(chromium --version 2>&1 | head -n 1 || true)"
if [[ "${chromium_version}" == *Chromium* ]]; then
  check DISK-BROWSER "${chromium_version}" PASS
else
  check DISK-BROWSER "${chromium_version}" FAIL
fi

# DISK-DEBIAN-SOURCES: literal Trixie suites, never the moving stable alias.
sources="$(cat /etc/apt/sources.list.d/*.sources /etc/apt/sources.list 2>/dev/null || true)"
if grep -qE '^\s*Suites:.*\btrixie\b' <<<"${sources}" &&
   ! grep -qE '^\s*Suites:.*\bstable\b' <<<"${sources}"; then
  check DISK-DEBIAN-SOURCES "literal suites only" PASS
else
  check DISK-DEBIAN-SOURCES "unexpected suites: $(grep -hE '^\s*Suites:' <<<"${sources}" | tr '\n' ' ')" FAIL
fi

# DISK-SENTIA-SOURCE: the Sentia overlay must be trusted by its own keyring.
if grep -qE '^\s*Signed-By:\s*/usr/share/keyrings/sentia-archive-keyring.gpg' <<<"${sources}"; then
  check DISK-SENTIA-SIGNED-BY "Signed-By: sentia-archive-keyring.gpg" PASS
else
  check DISK-SENTIA-SIGNED-BY "no Signed-By Sentia source" FAIL
fi

# DISK-SECURITY-UPDATES: Debian security updates must apply themselves.
if systemctl is-enabled apt-daily-upgrade.timer >/dev/null 2>&1 &&
   grep -rqs 'Unattended-Upgrade' /etc/apt/apt.conf.d/; then
  check DISK-SECURITY-UPDATES "unattended-upgrades configured and timer enabled" PASS
else
  check DISK-SECURITY-UPDATES "automatic security updates are not configured" FAIL
fi

# DISK-APT-OFFLINE-ARCHIVE: the installed local archive must still verify.
apt_update="$(sudo -n apt-get update -o Dir::Etc::sourcelist=/dev/null \
  -o Dir::Etc::sourceparts=/etc/apt/sources.list.d 2>&1 | tail -n 5 || true)"
if grep -q 'sentia' <<<"${apt_update}" && ! grep -qi 'NO_PUBKEY\|not signed' <<<"${apt_update}"; then
  check DISK-APT-SENTIA-VERIFIES "${apt_update}" PASS
else
  check DISK-APT-SENTIA-VERIFIES "${apt_update}" FAIL
fi

# DISK-AI-*: the product claim, on the installed system.
for unit_check in DISK-AI-RUNTIME:sentia-local-llama.service DISK-AI-BROKER:sentia-local-broker.service; do
  id="${unit_check%%:*}"
  unit="${unit_check##*:}"
  deadline=$((SECONDS + 300))
  state="$(systemctl is-active "${unit}" 2>&1 || true)"
  while [[ "${state}" != "active" && "${SECONDS}" -lt "${deadline}" ]]; do
    sleep 5
    state="$(systemctl is-active "${unit}" 2>&1 || true)"
  done
  check "${id}" "${unit} ${state}" \
    "$([[ "${state}" == "active" ]] && echo PASS || echo FAIL)"
done

model="/usr/share/sentia/models/granite-4.2-3b/granite-4.2-3b-Q4_K_M.gguf"
model_bytes="$(stat -c %s "${model}" 2>/dev/null || echo 0)"
if [[ "${model_bytes}" == "2244011552" ]]; then
  check DISK-AI-MODEL "${model_bytes} bytes" PASS
else
  check DISK-AI-MODEL "unexpected model size: ${model_bytes}" FAIL
fi

ai_uid="$(id -u)"
router_socket="/run/user/${ai_uid}/sentia/router.sock"
deadline=$((SECONDS + 180))
while [[ ! -S "${router_socket}" && "${SECONDS}" -lt "${deadline}" ]]; do sleep 5; done
if [[ -S "${router_socket}" ]]; then
  check DISK-AI-ROUTER-SOCKET "$(stat -c '%a %U' "${router_socket}")" PASS
else
  check DISK-AI-ROUTER-SOCKET "${router_socket} missing" FAIL
fi

# DISK-TOOLS-*: deterministic system tools must answer without the model.
if apt_output="$(timeout 120 ai tools apt_search openssh-server 2>&1)" &&
   grep -q 'openssh-server' <<<"${apt_output}"; then
  check DISK-TOOLS-APT-SEARCH "${apt_output:0:200}" PASS
else
  check DISK-TOOLS-APT-SEARCH "${apt_output:0:200}" FAIL
fi

if sim_output="$(timeout 180 ai tools apt_simulate_install openssh-server 2>&1)" &&
   grep -qi 'openssh-server' <<<"${sim_output}"; then
  check DISK-TOOLS-APT-SIMULATE "${sim_output:0:200}" PASS
else
  check DISK-TOOLS-APT-SIMULATE "${sim_output:0:200}" FAIL
fi

if health_output="$(timeout 120 ai tools memory_status 2>&1)" &&
   grep -qi 'total' <<<"${health_output}"; then
  check DISK-HEALTH-METRICS "${health_output:0:200}" PASS
else
  check DISK-HEALTH-METRICS "${health_output:0:200}" FAIL
fi

# DISK-SHELL-TYPO: an obvious misspelling must be corrected deterministically.
typo_output="$(timeout 120 bash -lc 'sudp apt update' 2>&1 || true)"
if grep -qi 'sudo' <<<"${typo_output}"; then
  check DISK-SHELL-TYPO "${typo_output:0:200}" PASS
else
  check DISK-SHELL-TYPO "${typo_output:0:200}" FAIL
fi

# DISK-OFFLINE-ANSWER: take the network down and require a real local answer.
for iface in $(ls /sys/class/net | grep -v '^lo$'); do
  sudo -n ip link set "${iface}" down 2>/dev/null || true
done
offline_output="$(timeout 900 ai --policy LOCAL_ONLY \
  'name the command that lists open files' 2>&1)"
offline_status=$?
if [[ "${offline_status}" -eq 0 && -n "${offline_output//[[:space:]]/}" ]]; then
  check DISK-OFFLINE-ANSWER "${offline_output:0:300}" PASS
else
  check DISK-OFFLINE-ANSWER "status=${offline_status} ${offline_output:0:300}" FAIL
fi

# DISK-NO-REMOTE-PROVIDER: no provider is configured, and nothing depends on one.
if timeout 60 ai settings show 2>&1 | grep -qiE 'provider.*(none|not configured)|LOCAL'; then
  check DISK-NO-REMOTE-PROVIDER "no remote provider configured" PASS
else
  check DISK-NO-REMOTE-PROVIDER "$(timeout 60 ai settings show 2>&1 | head -c 200)" FAIL
fi

for iface in $(ls /sys/class/net | grep -v '^lo$'); do
  sudo -n ip link set "${iface}" up 2>/dev/null || true
done

echo "SENTIA_CHECKS_COMPLETE"
