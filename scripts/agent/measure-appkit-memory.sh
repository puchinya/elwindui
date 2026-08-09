#!/usr/bin/env bash
# Reproducibly records the AppKit base-memory A-D comparison for Issue #60.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

readonly sample_count=5
readonly settle_ms=5000
readonly report_path="docs/status/appkit_memory_baseline.md"
readonly baseline_binary="target/release/examples/appkit-memory-baseline"
readonly graphics_demo_binary="target/release/graphics-demo"

declare -a sample_cases=()
declare -a sample_runs=()
declare -a sample_json=()

json_number() {
    local json="$1"
    local key="$2"
    if [[ "$json" =~ \"${key}\":([0-9]+) ]]; then
        printf '%s\n' "${BASH_REMATCH[1]}"
        return 0
    fi
    printf 'missing numeric JSON field %s\n' "$key" >&2
    return 1
}

format_mib() {
    awk -v bytes="$1" 'BEGIN { printf "%.2f", bytes / 1048576 }'
}

case_values() {
    local wanted_case="$1"
    local field="$2"
    local index
    for ((index = 0; index < ${#sample_cases[@]}; index++)); do
        if [[ "${sample_cases[index]}" == "$wanted_case" ]]; then
            json_number "${sample_json[index]}" "$field"
        fi
    done
}

median() {
    sort -n | sed -n '3p'
}

range() {
    sort -n | awk 'NR == 1 { min = $1 } { max = $1 } END { printf "%s–%s", min, max }'
}

run_case() {
    local case_name="$1"
    local run_number="$2"
    local command=()
    case "$case_name" in
        A|B|C) command=("$baseline_binary" "$case_name") ;;
        D) command=("$graphics_demo_binary") ;;
        *) printf 'unknown case: %s\n' "$case_name" >&2; return 2 ;;
    esac

    local output
    output="$(
        ELWINDUI_APPKIT_MEMORY_REPORT_AFTER_MS="$settle_ms" \
        ELWINDUI_APPKIT_MEMORY_EXIT_AFTER_REPORT=1 \
        "${command[@]}" 2>&1
    )"
    local line
    line="$(printf '%s\n' "$output" | sed -n 's/^elwindui-appkit-memory //p' | tail -n 1)"
    if [[ -z "$line" ]]; then
        printf 'no AppKit memory report for case %s run %s\n%s\n' "$case_name" "$run_number" "$output" >&2
        return 1
    fi
    json_number "$line" physical_footprint_bytes >/dev/null
    json_number "$line" resident_bytes >/dev/null
    sample_cases+=("$case_name")
    sample_runs+=("$run_number")
    sample_json+=("$line")
}

cargo build --release -p elwindui-backend-appkit --example appkit-memory-baseline --features render-stats
cargo build --release -p graphics-demo --features render-stats

for ((run = 1; run <= sample_count; run++)); do
    for case_name in A B C D; do
        run_case "$case_name" "$run"
    done
done

{
    printf '# AppKit 基礎メモリ Baseline\n\n'
    printf 'Issue #60 の Task 1–2 用の実測記録。`scripts/agent/measure-appkit-memory.sh` が上書き生成する。\n\n'
    printf '## 測定条件\n\n'
    printf -- '- Date: %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf -- '- Commit: `%s`\n' "$(git rev-parse HEAD)"
    printf -- '- macOS: %s\n' "$(sw_vers -productVersion)"
    printf -- '- Architecture: %s\n' "$(uname -m)"
    printf -- '- CPU: %s\n' "$(sysctl -n machdep.cpu.brand_string)"
    printf -- '- Physical memory: %s bytes\n' "$(sysctl -n hw.memsize)"
    printf -- '- Build: `cargo build --release`, `render-stats` enabled\n'
    printf -- '- Runs: %s per case, separate process, fixed 800x600 window, no interaction, %sms stabilization\n\n' "$sample_count" "$settle_ms"
    printf '## ケース\n\n'
    printf '| Case | Configuration |\n|---|---|\n'
    printf '| A | NSApplication + NSWindow + empty NSView |\n'
    printf '| B | NSApplication + NSWindow + empty TreeHostView |\n'
    printf '| C | B + TreeHostView.wantsLayer = true |\n'
    printf '| D | graphics-demo initial Fills tab |\n\n'
    printf '## 全サンプル\n\n'
    printf '| Case | Run | Physical Footprint (MiB) | RSS (MiB) | TreeHosts attached/hidden | NSViews | Layer-backed NSViews | CALayers |\n'
    printf '|---|---:|---:|---:|---:|---:|---:|---:|\n'
    for ((index = 0; index < ${#sample_cases[@]}; index++)); do
        json="${sample_json[index]}"
        footprint="$(json_number "$json" physical_footprint_bytes)"
        resident="$(json_number "$json" resident_bytes)"
        attached="$(json_number "$json" attached_tree_host_count)"
        hidden="$(json_number "$json" hidden_tree_host_count)"
        views="$(json_number "$json" native_nsview_count)"
        layer_views="$(json_number "$json" layer_backed_nsview_count)"
        layers="$(json_number "$json" live_calayer_count)"
        printf '| %s | %s | %s | %s | %s/%s | %s | %s | %s |\n' \
            "${sample_cases[index]}" "${sample_runs[index]}" "$(format_mib "$footprint")" \
            "$(format_mib "$resident")" "$attached" "$hidden" "$views" "$layer_views" "$layers"
    done
    printf '\n## 中央値と差分\n\n'
    printf '| Case | Physical Footprint median (MiB) | range (bytes) | RSS median (MiB) |\n'
    printf '|---|---:|---:|---:|\n'
    for case_name in A B C D; do
        footprint_values="$(case_values "$case_name" physical_footprint_bytes)"
        resident_values="$(case_values "$case_name" resident_bytes)"
        footprint_median="$(printf '%s\n' "$footprint_values" | median)"
        resident_median="$(printf '%s\n' "$resident_values" | median)"
        printf '| %s | %s | %s | %s |\n' "$case_name" "$(format_mib "$footprint_median")" \
            "$(printf '%s\n' "$footprint_values" | range)" "$(format_mib "$resident_median")"
    done
    a_median="$(case_values A physical_footprint_bytes | median)"
    b_median="$(case_values B physical_footprint_bytes | median)"
    c_median="$(case_values C physical_footprint_bytes | median)"
    d_median="$(case_values D physical_footprint_bytes | median)"
    printf '\n- B - A: %s MiB\n' "$(awk -v b="$b_median" -v a="$a_median" 'BEGIN { printf "%.2f", (b - a) / 1048576 }')"
    printf -- '- C - B: %s MiB\n' "$(awk -v c="$c_median" -v b="$b_median" 'BEGIN { printf "%.2f", (c - b) / 1048576 }')"
    printf -- '- D - C: %s MiB\n' "$(awk -v d="$d_median" -v c="$c_median" 'BEGIN { printf "%.2f", (d - c) / 1048576 }')"
    printf '\n## Raw JSON\n\n```json\n'
    for ((index = 0; index < ${#sample_cases[@]}; index++)); do
        printf '{"case":"%s","run":%s,"snapshot":%s}\n' \
            "${sample_cases[index]}" "${sample_runs[index]}" "${sample_json[index]}"
    done
    printf '```\n\n'
    printf '## 観察と次の判断\n\n'
    printf 'この Issue は計測基盤と baseline のみを提供する。次の最適化は C-B の実測差とばらつきをレビューしてから別 Issue で決定する。\n'
} > "$report_path"

printf 'Wrote %s\n' "$report_path"
