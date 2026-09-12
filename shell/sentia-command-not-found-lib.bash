#!/usr/bin/env bash
# shellcheck shell=bash

sentia__default_command_index() {
  printf '%s' "/usr/share/sentia/command-index.tsv"
}

sentia__executable_search_path() {
  if [[ -n "${SENTIA_EXECUTABLE_SEARCH_PATH:-}" ]]; then
    printf '%s' "$SENTIA_EXECUTABLE_SEARCH_PATH"
  else
    printf '%s' "${PATH:-/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin}"
  fi
}

sentia__levenshtein() {
  local left=${1:-}
  local right=${2:-}
  awk -v a="$left" -v b="$right" '
    function min3(x, y, z) {
      m = x
      if (y < m) { m = y }
      if (z < m) { m = z }
      return m
    }
    BEGIN {
      la = length(a)
      lb = length(b)
      for (i = 0; i <= la; i++) { d[i ",0"] = i }
      for (j = 0; j <= lb; j++) { d["0," j] = j }

      for (i = 1; i <= la; i++) {
        ca = substr(a, i, 1)
        for (j = 1; j <= lb; j++) {
          cb = substr(b, j, 1)
          cost = (ca == cb ? 0 : 1)
          above = d[(i - 1) "," j] + 1
          leftv = d[i "," (j - 1)] + 1
          diag = d[(i - 1) "," (j - 1)] + cost
          d[i "," j] = min3(above, leftv, diag)
        }
      }

      print d[la "," lb]
    }
  '
}

sentia__collect_executables() {
  local path_string
  path_string=$(sentia__executable_search_path)
  local dir

  IFS=':' read -r -a sentia_path_items <<<"$path_string"
  for dir in "${sentia_path_items[@]}"; do
    [[ -d "$dir" ]] || continue
    find "$dir" -maxdepth 1 -mindepth 1 -type f -perm -001 -printf '%f\n' 2>/dev/null || true
  done | LC_ALL=C sort -u
}

sentia__best_typo_match() {
  local needle=${1:-}
  local max_distance=${2:-2}
  local candidate
  local best=""
  local best_distance=9999

  while IFS= read -r candidate; do
    [[ -n "$candidate" ]] || continue
    local distance
    distance=$(sentia__levenshtein "$needle" "$candidate")

    if (( distance > max_distance )); then
      continue
    fi

    if (( distance < best_distance )); then
      best="$candidate"
      best_distance=$distance
    elif (( distance == best_distance )) && [[ "$candidate" < "$best" ]]; then
      best="$candidate"
    fi
  done < <(sentia__collect_executables)

  if [[ -n "$best" ]]; then
    printf '%s\n' "$best"
  fi
}

sentia__lookup_packages() {
  local command_name=${1:-}
  local index_path=${SENTIA_COMMAND_INDEX:-$(sentia__default_command_index)}

  [[ -n "$command_name" ]] || return 0
  [[ -r "$index_path" ]] || return 0

  awk -F'\t' -v key="$command_name" '
    $0 ~ /^#/ { next }
    $1 == key && $2 != "" { print $2 }
  ' "$index_path" | LC_ALL=C sort -u
}

sentia__looks_natural_language() {
  local text=${*:-}
  [[ -n "$text" ]] || return 1

  local words
  words=$(wc -w <<<"$text")
  (( words >= 3 )) || return 1

  local lower
  lower=${text,,}

  case " $lower " in
    *"?"*|*" how "*|*" what "*|*" why "*|*" please "*|*" could "*|*" show "*|*" list "*|*" explain "*|*" find "*|*" install "*)
      return 0
      ;;
  esac

  return 1
}

sentia__shell_quote() {
  local input=${1:-}
  local escaped=${input//\'/\'\\\'\'}
  printf "'%s'" "$escaped"
}

sentia_classify_not_found() {
  local raw=${*:-}
  local command_name
  command_name=$(awk '{print $1}' <<<"$raw")

  if [[ -z "$command_name" ]]; then
    printf 'CLASS=UNKNOWN\n'
    printf 'COMMAND=\n'
    return 0
  fi

  local max_distance=${SENTIA_TYPO_MAX_DISTANCE:-2}
  local suggestion
  suggestion=$(sentia__best_typo_match "$command_name" "$max_distance" || true)
  if [[ -n "$suggestion" && "$suggestion" != "$command_name" ]]; then
    printf 'CLASS=LIKELY_TYPO\n'
    printf 'COMMAND=%s\n' "$command_name"
    printf 'SUGGESTION=%s\n' "$suggestion"
    return 0
  fi

  local package_lines
  package_lines=$(sentia__lookup_packages "$command_name" | paste -sd, -)
  if [[ -n "$package_lines" ]]; then
    printf 'CLASS=MISSING_PACKAGE\n'
    printf 'COMMAND=%s\n' "$command_name"
    printf 'PACKAGES=%s\n' "$package_lines"
    return 0
  fi

  if sentia__looks_natural_language "$raw"; then
    printf 'CLASS=NATURAL_LANGUAGE\n'
    printf 'COMMAND=%s\n' "$command_name"
    printf 'AI_HINT=ai -- %s\n' "$(sentia__shell_quote "$raw")"
    return 0
  fi

  printf 'CLASS=UNKNOWN\n'
  printf 'COMMAND=%s\n' "$command_name"
}
