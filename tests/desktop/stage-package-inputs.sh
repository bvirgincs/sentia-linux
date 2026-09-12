#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"

stage_manifest() {
    manifest_path="$1"
    stage_root="$2"

    rm -rf "$stage_root"
    mkdir -p "$stage_root"

    while IFS= read -r entry || [ -n "$entry" ]; do
        entry="${entry%%#*}"
        entry="$(printf '%s' "$entry" | sed -E 's/^[[:space:]]+|[[:space:]]+$//g')"
        [ -z "$entry" ] && continue

        case "$entry" in
            /*|*..*)
                echo "invalid manifest entry: $entry" >&2
                exit 1
                ;;
        esac

        src="$ROOT_DIR/$entry"
        dest="$stage_root/$entry"

        if [ ! -e "$src" ]; then
            echo "manifest entry missing: $entry" >&2
            exit 1
        fi

        if [ -d "$src" ]; then
            mkdir -p "$dest"
            cp -a "$src/." "$dest/"
        else
            mkdir -p "$(dirname "$dest")"
            cp -a "$src" "$dest"
        fi
    done < "$manifest_path"
}

DESKTOP_STAGE="$ROOT_DIR/packaging/desktop/debian/staged-config"
CALAMARES_STAGE="$ROOT_DIR/packaging/calamares/debian/staged-config"

stage_manifest \
    "$ROOT_DIR/packaging/desktop/debian/source-inputs.manifest" \
    "$DESKTOP_STAGE"
stage_manifest \
    "$ROOT_DIR/packaging/calamares/debian/source-inputs.manifest" \
    "$CALAMARES_STAGE"

# Flatten staged paths into package-local install trees expected by debian/rules.
mkdir -p "$DESKTOP_STAGE"
cp -a "$DESKTOP_STAGE/config/desktop/." "$DESKTOP_STAGE/"
rm -rf "$DESKTOP_STAGE/config"

mkdir -p "$CALAMARES_STAGE"
cp -a "$CALAMARES_STAGE/config/calamares/." "$CALAMARES_STAGE/"
rm -rf "$CALAMARES_STAGE/config"
