#!/usr/bin/env bash
# Reproducibly measures the staged AppKit UI-construction profiles for Issue #60.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

readonly sample_count=5
readonly settle_ms=5000
readonly baseline_binary="target/release/examples/appkit-memory-baseline"
readonly graphics_demo_binary="target/release/graphics-demo"
readonly raw_root=".agent-state/issues/60/ui-construction-$(date -u '+%Y%m%dT%H%M%SZ')"
readonly report_path="$raw_root/report.md"

declare -a sample_cases=()
declare -a sample_runs=()
declare -a sample_json=()
declare -a sample_malloc_allocated_kib=()
declare -a sample_malloc_frag_kib=()
declare -a sample_malloc_small_dirty_kib=()
declare -a sample_core_animation_dirty_kib=()
declare -a sample_vm_allocate_dirty_kib=()
active_pid=""

cleanup_active_process() {
    if [[ -n "$active_pid" ]]; then
        kill -TERM "$active_pid" 2>/dev/null || true
        wait "$active_pid" 2>/dev/null || true
    fi
}
trap cleanup_active_process EXIT INT TERM

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

format_kib() {
    awk -v kib="$1" 'BEGIN { printf "%.1f", kib }'
}

signed_mib_delta() {
    awk -v after="$1" -v before="$2" 'BEGIN { printf "%+.2f", (after - before) / 1048576 }'
}

signed_kib_delta() {
    awk -v after="$1" -v before="$2" 'BEGIN { printf "%+.1f", after - before }'
}

median() {
    sort -n | sed -n '3p'
}

range() {
    sort -n | awk 'NR == 1 { min = $1 } { max = $1 } END { printf "%s–%s", min, max }'
}

range_mib() {
    sort -n | awk 'NR == 1 { min = $1 } { max = $1 } END { printf "%.2f–%.2f", min / 1048576, max / 1048576 }'
}

range_kib() {
    sort -n | awk 'NR == 1 { min = $1 } { max = $1 } END { printf "%.1f–%.1f", min, max }'
}

unit_to_kib() {
    awk -v value="$1" '
        function number(text) { sub(/[KMG]$/, "", text); return text + 0 }
        BEGIN {
            suffix = substr(value, length(value), 1)
            multiplier = suffix == "G" ? 1048576 : suffix == "M" ? 1024 : 1
            printf "%.3f\n", number(value) * multiplier
        }
    '
}

vmmap_row_dirty_kib() {
    local summary_path="$1"
    local region="$2"
    local raw
    raw="$(awk -v region="$region" '$1 == region { print $4; exit }' "$summary_path")"
    if [[ -z "$raw" ]]; then
        printf '0\n'
    else
        unit_to_kib "$raw"
    fi
}

vmmap_zone_field_kib() {
    local summary_path="$1"
    local field="$2"
    awk -v field="$field" '
        function kib(value, suffix, multiplier) {
            suffix = substr(value, length(value), 1)
            multiplier = suffix == "G" ? 1048576 : suffix == "M" ? 1024 : 1
            sub(/[KMG]$/, "", value)
            return value * multiplier
        }
        /^MALLOC ZONE[[:space:]]/ { in_zone_table = 1; next }
        in_zone_table && /^===/ { next }
        in_zone_table && NF >= 8 && $2 ~ /^[0-9]/ { total += kib($field); next }
        in_zone_table && NF < 8 { in_zone_table = 0 }
        END { printf "%.3f\n", total }
    ' "$summary_path"
}

case_values() {
    local wanted_case="$1"
    local source="$2"
    local field="$3"
    local index
    for ((index = 0; index < ${#sample_cases[@]}; index++)); do
        if [[ "${sample_cases[index]}" != "$wanted_case" ]]; then
            continue
        fi
        case "$source" in
            snapshot) json_number "${sample_json[index]}" "$field" ;;
            malloc_allocated) printf '%s\n' "${sample_malloc_allocated_kib[index]}" ;;
            malloc_frag) printf '%s\n' "${sample_malloc_frag_kib[index]}" ;;
            malloc_small_dirty) printf '%s\n' "${sample_malloc_small_dirty_kib[index]}" ;;
            core_animation_dirty) printf '%s\n' "${sample_core_animation_dirty_kib[index]}" ;;
            vm_allocate_dirty) printf '%s\n' "${sample_vm_allocate_dirty_kib[index]}" ;;
            *) printf 'unknown sample source: %s\n' "$source" >&2; return 2 ;;
        esac
    done
}

case_median() {
    case_values "$1" "$2" "$3" | median
}

wait_for_report() {
    local output_path="$1"
    local attempt line
    for ((attempt = 0; attempt < 100; attempt++)); do
        line="$(sed -n 's/^elwindui-appkit-memory //p' "$output_path" | tail -n 1)"
        if [[ -n "$line" ]]; then
            printf '%s\n' "$line"
            return 0
        fi
        sleep 0.1
    done
    return 1
}

run_case() {
    local case_name="$1"
    local run_number="$2"
    local output_path="$raw_root/${case_name}-${run_number}.log"
    local summary_path="$raw_root/${case_name}-${run_number}.vmmap-summary.txt"
    local profile=""
    local command=()
    case "$case_name" in
        A) command=("$baseline_binary" A) ;;
        E) command=("$baseline_binary" E) ;;
        F|G|H|I) command=("$graphics_demo_binary"); profile="$case_name" ;;
        J) command=("$graphics_demo_binary") ;;
        *) printf 'unknown case: %s\n' "$case_name" >&2; return 2 ;;
    esac

    if [[ -n "$profile" ]]; then
        ELWINDUI_GRAPHICS_DEMO_MEMORY_CASE="$profile" \
            ELWINDUI_APPKIT_MEMORY_REPORT_AFTER_MS="$settle_ms" \
            "${command[@]}" >"$output_path" 2>&1 &
    else
        ELWINDUI_APPKIT_MEMORY_REPORT_AFTER_MS="$settle_ms" \
            "${command[@]}" >"$output_path" 2>&1 &
    fi
    active_pid="$!"

    local line
    if ! line="$(wait_for_report "$output_path")"; then
        printf 'no AppKit memory report for case %s run %s\n' "$case_name" "$run_number" >&2
        sed -n '1,200p' "$output_path" >&2
        return 1
    fi
    json_number "$line" physical_footprint_bytes >/dev/null
    json_number "$line" resident_bytes >/dev/null
    vmmap -summary "$active_pid" >"$summary_path"

    local malloc_allocated malloc_frag malloc_small_dirty core_animation_dirty vm_allocate_dirty
    malloc_allocated="$(vmmap_zone_field_kib "$summary_path" 7)"
    malloc_frag="$(vmmap_zone_field_kib "$summary_path" 8)"
    malloc_small_dirty="$(vmmap_row_dirty_kib "$summary_path" MALLOC_SMALL)"
    core_animation_dirty="$(vmmap_row_dirty_kib "$summary_path" CoreAnimation)"
    vm_allocate_dirty="$(vmmap_row_dirty_kib "$summary_path" VM_ALLOCATE)"

    kill -TERM "$active_pid"
    wait "$active_pid" || true
    active_pid=""

    sample_cases+=("$case_name")
    sample_runs+=("$run_number")
    sample_json+=("$line")
    sample_malloc_allocated_kib+=("$malloc_allocated")
    sample_malloc_frag_kib+=("$malloc_frag")
    sample_malloc_small_dirty_kib+=("$malloc_small_dirty")
    sample_core_animation_dirty_kib+=("$core_animation_dirty")
    sample_vm_allocate_dirty_kib+=("$vm_allocate_dirty")
}

mkdir -p "$raw_root"
cargo build --release -p elwindui-backend-appkit --example appkit-memory-baseline --features render-stats
cargo build --release -p graphics-demo --features render-stats

for ((run = 1; run <= sample_count; run++)); do
    for case_name in A E F G H I J; do
        run_case "$case_name" "$run"
    done
done

{
    printf '# AppKit UI構築段階メモリ計測\n\n'
    printf 'Issue #60の次段階調査。`scripts/agent/measure-appkit-ui-construction-memory.sh`がrelease buildを各case 5回ずつ実行して生成した。\n\n'
    printf '## 測定条件\n\n'
    printf -- '- Date: %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf -- '- Commit: `%s`\n' "$(git rev-parse HEAD)"
    printf -- '- macOS: %s\n' "$(sw_vers -productVersion)"
    printf -- '- Architecture: %s\n' "$(uname -m)"
    printf -- '- Samples: %s separate processes per case, JSON report after %s ms\n' "$sample_count" "$settle_ms"
    printf -- '- Raw process logs and `vmmap -summary` captures: `%s/` (not committed)\n\n' "$raw_root"
    printf 'Case definitions: A empty `NSView`; E empty `TreeHostView`; F `TabView` with 0 tabs; G 1 empty tab; H 7 empty tabs; I current 7-tab graphics-demo UIElement/state tree with every canvas paint callback disabled; J normal graphics-demo with Fills selected and painting enabled.\n\n'
    printf '## 全sample\n\n'
    printf '| Case | Run | Footprint MiB | RSS MiB | MALLOC allocated KiB | FRAG KiB | MALLOC_SMALL dirty KiB | CA dirty KiB | VM_ALLOCATE dirty KiB | NSView | TreeHost | CALayer | NSStackView | NSButton |\n'
    printf '|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n'
    for ((index = 0; index < ${#sample_cases[@]}; index++)); do
        json="${sample_json[index]}"
        printf '| %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s |\n' \
            "${sample_cases[index]}" "${sample_runs[index]}" \
            "$(format_mib "$(json_number "$json" physical_footprint_bytes)")" \
            "$(format_mib "$(json_number "$json" resident_bytes)")" \
            "$(format_kib "${sample_malloc_allocated_kib[index]}")" \
            "$(format_kib "${sample_malloc_frag_kib[index]}")" \
            "$(format_kib "${sample_malloc_small_dirty_kib[index]}")" \
            "$(format_kib "${sample_core_animation_dirty_kib[index]}")" \
            "$(format_kib "${sample_vm_allocate_dirty_kib[index]}")" \
            "$(json_number "$json" native_nsview_count)" \
            "$(json_number "$json" attached_tree_host_count)" \
            "$(json_number "$json" live_calayer_count)" \
            "$(json_number "$json" native_nsstackview_count)" \
            "$(json_number "$json" native_nsbutton_count)"
    done
    printf '\n## 中央値\n\n'
    printf '| Case | Footprint MiB | RSS MiB | MALLOC allocated KiB | FRAG KiB | MALLOC_SMALL dirty KiB | CA dirty KiB | VM_ALLOCATE dirty KiB | NSView | TreeHost | CALayer | NSStackView | NSButton |\n'
    printf '|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n'
    for case_name in A E F G H I J; do
        printf '| %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s | %s |\n' \
            "$case_name" \
            "$(format_mib "$(case_median "$case_name" snapshot physical_footprint_bytes)")" \
            "$(format_mib "$(case_median "$case_name" snapshot resident_bytes)")" \
            "$(format_kib "$(case_median "$case_name" malloc_allocated unused)")" \
            "$(format_kib "$(case_median "$case_name" malloc_frag unused)")" \
            "$(format_kib "$(case_median "$case_name" malloc_small_dirty unused)")" \
            "$(format_kib "$(case_median "$case_name" core_animation_dirty unused)")" \
            "$(format_kib "$(case_median "$case_name" vm_allocate_dirty unused)")" \
            "$(case_median "$case_name" snapshot native_nsview_count)" \
            "$(case_median "$case_name" snapshot attached_tree_host_count)" \
            "$(case_median "$case_name" snapshot live_calayer_count)" \
            "$(case_median "$case_name" snapshot native_nsstackview_count)" \
            "$(case_median "$case_name" snapshot native_nsbutton_count)"
    done
    printf '\n## 範囲\n\n'
    printf '| Case | Footprint MiB | RSS MiB | MALLOC allocated KiB | FRAG KiB | MALLOC_SMALL dirty KiB | CA dirty KiB | VM_ALLOCATE dirty KiB |\n'
    printf '|---|---:|---:|---:|---:|---:|---:|---:|\n'
    for case_name in A E F G H I J; do
        printf '| %s | %s | %s | %s | %s | %s | %s | %s |\n' \
            "$case_name" \
            "$(case_values "$case_name" snapshot physical_footprint_bytes | range_mib)" \
            "$(case_values "$case_name" snapshot resident_bytes | range_mib)" \
            "$(case_values "$case_name" malloc_allocated unused | range_kib)" \
            "$(case_values "$case_name" malloc_frag unused | range_kib)" \
            "$(case_values "$case_name" malloc_small_dirty unused | range_kib)" \
            "$(case_values "$case_name" core_animation_dirty unused | range_kib)" \
            "$(case_values "$case_name" vm_allocate_dirty unused | range_kib)"
    done
    printf '\n## 隣接caseの中央値差分\n\n'
    printf '| Transition | Footprint MiB | MALLOC allocated KiB | FRAG KiB | MALLOC_SMALL dirty KiB | CA dirty KiB | VM_ALLOCATE dirty KiB | NSView | TreeHost | CALayer |\n'
    printf '|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n'
    for transition in 'A E' 'E F' 'F G' 'G H' 'H I' 'I J'; do
        read -r before after <<<"$transition"
        printf '| %s - %s | %s | %s | %s | %s | %s | %s | %s | %s | %s |\n' \
            "$after" "$before" \
            "$(signed_mib_delta "$(case_median "$after" snapshot physical_footprint_bytes)" "$(case_median "$before" snapshot physical_footprint_bytes)")" \
            "$(signed_kib_delta "$(case_median "$after" malloc_allocated unused)" "$(case_median "$before" malloc_allocated unused)")" \
            "$(signed_kib_delta "$(case_median "$after" malloc_frag unused)" "$(case_median "$before" malloc_frag unused)")" \
            "$(signed_kib_delta "$(case_median "$after" malloc_small_dirty unused)" "$(case_median "$before" malloc_small_dirty unused)")" \
            "$(signed_kib_delta "$(case_median "$after" core_animation_dirty unused)" "$(case_median "$before" core_animation_dirty unused)")" \
            "$(signed_kib_delta "$(case_median "$after" vm_allocate_dirty unused)" "$(case_median "$before" vm_allocate_dirty unused)")" \
            "$(signed_kib_delta "$(case_median "$after" snapshot native_nsview_count)" "$(case_median "$before" snapshot native_nsview_count)")" \
            "$(signed_kib_delta "$(case_median "$after" snapshot attached_tree_host_count)" "$(case_median "$before" snapshot attached_tree_host_count)")" \
            "$(signed_kib_delta "$(case_median "$after" snapshot live_calayer_count)" "$(case_median "$before" snapshot live_calayer_count)")"
    done
    printf '\n## 解釈契約\n\n'
    printf -- '- MALLOC `ALLOCATED`はlive allocation、`FRAG SIZE`はallocatorが保持するcapacityであり、特定のElwindUI objectへ帰属させない。\n'
    printf -- '- IはJと同一のUIElement/state構成でpaint callbackのみを無効化するため、H→IをUI tree/layout/state、I→Jをrender tree/CALayer/paintの比較として読む。\n'
    printf -- '- `vmmap`値は各sampleの実プロセスから採取する。Physical FootprintとRSSは同じプロセスのrender-stats JSONから採取する。\n'
} >"$report_path"

printf 'wrote %s\n' "$report_path"
