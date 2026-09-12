#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
SCRIPT="$ROOT_DIR/packaging/live/files/usr/libexec/sentia-live/run-calamares-root"
WORK_DIR="$ROOT_DIR/.build/tests/run-calamares-root"

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/success" "$WORK_DIR/failure"

run_case() {
    case_name="$1"
    command_exit="$2"

    case_dir="$WORK_DIR/$case_name"
    fstab="$case_dir/fstab"
    backup="$case_dir/fstab.backup"
    command_path="$case_dir/fake-calamares.sh"

    printf 'UUID=1234 / ext4 defaults 0 1\n' > "$fstab"

    cat > "$command_path" <<'CMD'
#!/bin/sh
set -eu
exit "$SENTIA_FAKE_CALAMARES_EXIT"
CMD
    chmod 0755 "$command_path"

    set +e
    SENTIA_FAKE_CALAMARES_EXIT="$command_exit" \
    SENTIA_LIVE_FSTAB_PATH="$fstab" \
    SENTIA_LIVE_FSTAB_BACKUP_PATH="$backup" \
    SENTIA_LIVE_CALAMARES_COMMAND="$command_path" \
    "$SCRIPT"
    status=$?
    set -e

    if [ "$status" -ne "$command_exit" ]; then
        echo "unexpected exit code for $case_name: got $status, expected $command_exit" >&2
        exit 1
    fi

    if [ ! -f "$fstab" ]; then
        echo "fstab was not restored for $case_name" >&2
        exit 1
    fi
    if [ -f "$backup" ]; then
        echo "backup file still exists for $case_name" >&2
        exit 1
    fi
    grep -q '^UUID=1234 / ext4 defaults 0 1$' "$fstab"
}

run_case success 0
run_case failure 42

rm -rf "$WORK_DIR"

echo "run-calamares-root behavior checks passed"
