#!/usr/bin/env bash
# scripts/profile.sh — reproducible workload × size profiling matrix (E1)
#
# Usage:
#   ./scripts/profile.sh [--tool samply|flamegraph|criterion|all]
#                        [--workload <name>|all]
#                        [--size <N>|all]
#                        [--dry-run]
#
# Defaults: --tool criterion --workload all --size all
#
# Workload × size matrix:
#   ring_dynamics      : 16 64 256 1024
#   neural_population  : 100 500 2000
#   celegans           : fixed
#   knowledge_graph    : fixed
#   stress_emergence   : 100 1000 10000

set -euo pipefail

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PERF_DIR="${PROJECT_ROOT}/docs/perf"

# Workload definitions: "workload:size1,size2,..." or "workload:fixed"
WORKLOAD_DEFS=(
    "ring_dynamics:16,64,256,1024"
    "neural_population:100,500,2000"
    "celegans:fixed"
    "knowledge_graph:fixed"
    "stress_emergence:100,1000,10000"
)

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
TOOL="criterion"
WORKLOAD_FILTER="all"
SIZE_FILTER="all"
DRY_RUN=false

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
usage() {
    grep '^#' "$0" | grep -v '#!/' | sed 's/^# \{0,2\}//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tool)
            TOOL="${2:?--tool requires an argument}"
            shift 2
            ;;
        --workload)
            WORKLOAD_FILTER="${2:?--workload requires an argument}"
            shift 2
            ;;
        --size)
            SIZE_FILTER="${2:?--size requires an argument}"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --help|-h)
            usage
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage
            ;;
    esac
done

# Validate --tool
case "$TOOL" in
    samply|flamegraph|criterion|all) ;;
    *)
        echo "Error: --tool must be samply, flamegraph, criterion, or all" >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# Tool availability checks
# ---------------------------------------------------------------------------
check_tool() {
    local name="$1"
    local install_hint="$2"
    if ! command -v "$name" &>/dev/null; then
        echo "Warning: '$name' not found. Install with: ${install_hint}"
        return 1
    fi
    return 0
}

check_cargo_subcommand() {
    local sub="$1"
    local install_hint="$2"
    if ! cargo "$sub" --help &>/dev/null 2>&1; then
        echo "Warning: 'cargo ${sub}' not found. Install with: ${install_hint}"
        return 1
    fi
    return 0
}

tools_ok=true

if [[ "$TOOL" == "samply" || "$TOOL" == "all" ]]; then
    check_tool samply "cargo install samply" || tools_ok=false
fi

if [[ "$TOOL" == "flamegraph" || "$TOOL" == "all" ]]; then
    check_cargo_subcommand flamegraph "cargo install flamegraph" || tools_ok=false
fi

if [[ "$TOOL" == "criterion" || "$TOOL" == "all" ]]; then
    check_cargo_subcommand criterion "cargo install cargo-criterion" || tools_ok=false
fi

if [[ "$tools_ok" == "false" && "$DRY_RUN" == "false" ]]; then
    echo ""
    echo "One or more required tools are missing. Install them and retry, or use --dry-run to preview commands."
    echo ""
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
run_or_print() {
    # run_or_print <description> <cmd> [args...]
    local desc="$1"; shift
    echo "[profile] ${desc}"
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "  DRY-RUN: $*"
    else
        "$@"
    fi
}

elapsed_seconds() {
    # elapsed_seconds <start_epoch_ns>  (use $SECONDS for portability)
    echo $(( SECONDS - $1 ))
}

workload_sizes() {
    # workload_sizes <workload_name> → prints space-separated sizes, or "fixed"
    local wl="$1"
    for def in "${WORKLOAD_DEFS[@]}"; do
        local name="${def%%:*}"
        local sizes="${def##*:}"
        if [[ "$name" == "$wl" ]]; then
            if [[ "$sizes" == "fixed" ]]; then
                echo "fixed"
            else
                echo "${sizes//,/ }"
            fi
            return 0
        fi
    done
    echo ""   # unknown workload
}

all_workloads() {
    for def in "${WORKLOAD_DEFS[@]}"; do
        echo "${def%%:*}"
    done
}

# Build effective workload list
if [[ "$WORKLOAD_FILTER" == "all" ]]; then
    WORKLOADS=( $(all_workloads) )
else
    WORKLOADS=( "$WORKLOAD_FILTER" )
fi

# ---------------------------------------------------------------------------
# docs/perf directory
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" == "false" ]]; then
    mkdir -p "${PERF_DIR}"
fi

# ---------------------------------------------------------------------------
# Run helpers per tool
# ---------------------------------------------------------------------------

run_criterion() {
    local start_all=$SECONDS
    echo ""
    echo "=== criterion: saving baseline 'phase1' ==="
    local cmd=(
        cargo criterion
        --bench engine
        --save-baseline phase1
    )
    run_or_print "criterion bench (engine)" \
        "${cmd[@]}"

    if [[ "$DRY_RUN" == "false" ]]; then
        local elapsed=$(( SECONDS - start_all ))
        echo "[profile] criterion finished in ${elapsed}s"
        # Copy criterion output into docs/perf if present
        local baseline_src="${PROJECT_ROOT}/target/criterion"
        if [[ -d "$baseline_src" ]]; then
            cp -r "$baseline_src" "${PERF_DIR}/criterion_baseline" 2>/dev/null || true
            echo "[profile] criterion baseline copied → ${PERF_DIR}/criterion_baseline/"
        fi
    fi
}

run_samply() {
    local workload="$1"
    local size="$2"       # may be empty for fixed workloads
    local tag
    if [[ -n "$size" ]]; then
        tag="${workload}_n${size}"
    else
        tag="${workload}"
    fi

    local out_dir="${PERF_DIR}/samply/${tag}"
    local args=()
    if [[ -n "$size" ]]; then
        args=( -- --size "$size" )
    fi

    if [[ "$DRY_RUN" == "false" ]]; then
        mkdir -p "$out_dir"
    fi

    # samply writes a profile to a default location; redirect stdout/stderr log
    run_or_print "samply: ${tag}" \
        samply record \
            --output "${out_dir}/${tag}.json" \
            cargo run --example "$workload" --release \
            "${args[@]}"
}

run_flamegraph() {
    local workload="$1"
    local size="$2"
    local tag
    if [[ -n "$size" ]]; then
        tag="${workload}_n${size}"
    else
        tag="${workload}"
    fi

    local out_dir="${PERF_DIR}/flamegraph"
    local svg="${out_dir}/${tag}.svg"
    local args=()
    if [[ -n "$size" ]]; then
        args=( -- --size "$size" )
    fi

    if [[ "$DRY_RUN" == "false" ]]; then
        mkdir -p "$out_dir"
    fi

    run_or_print "flamegraph: ${tag}" \
        cargo flamegraph \
            --example "$workload" \
            --output "$svg" \
            "${args[@]}"
}

# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------
OVERALL_START=$SECONDS
TOTAL_RUNS=0
FAILED_RUNS=()

echo ""
echo "=== profile.sh ==="
echo "tool=${TOOL}  workload=${WORKLOAD_FILTER}  size=${SIZE_FILTER}  dry-run=${DRY_RUN}"
echo "results → ${PERF_DIR}"
echo ""

# criterion is workload-agnostic (runs all benches at once)
if [[ "$TOOL" == "criterion" || "$TOOL" == "all" ]]; then
    run_start=$SECONDS
    run_criterion
    TOTAL_RUNS=$(( TOTAL_RUNS + 1 ))
fi

# samply / flamegraph iterate over workload × size
if [[ "$TOOL" == "samply" || "$TOOL" == "flamegraph" || "$TOOL" == "all" ]]; then
    for wl in "${WORKLOADS[@]}"; do
        raw_sizes="$(workload_sizes "$wl")"

        if [[ -z "$raw_sizes" ]]; then
            echo "Warning: unknown workload '${wl}', skipping." >&2
            continue
        fi

        if [[ "$raw_sizes" == "fixed" ]]; then
            size_list=( "" )   # single run, no --size arg
        else
            size_list=( $raw_sizes )
        fi

        for sz in "${size_list[@]}"; do
            # Apply --size filter
            if [[ "$SIZE_FILTER" != "all" && -n "$sz" && "$sz" != "$SIZE_FILTER" ]]; then
                continue
            fi

            run_start=$SECONDS

            set +e
            if [[ "$TOOL" == "samply" || "$TOOL" == "all" ]]; then
                run_samply "$wl" "$sz"
                rc=$?
                [[ $rc -ne 0 ]] && FAILED_RUNS+=( "samply:${wl}:${sz}" )
            fi

            if [[ "$TOOL" == "flamegraph" || "$TOOL" == "all" ]]; then
                run_flamegraph "$wl" "$sz"
                rc=$?
                [[ $rc -ne 0 ]] && FAILED_RUNS+=( "flamegraph:${wl}:${sz}" )
            fi
            set -e

            TOTAL_RUNS=$(( TOTAL_RUNS + 1 ))
            if [[ "$DRY_RUN" == "false" ]]; then
                elapsed=$(( SECONDS - run_start ))
                echo "[profile] done in ${elapsed}s"
            fi
        done
    done
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
OVERALL_ELAPSED=$(( SECONDS - OVERALL_START ))
echo ""
echo "=== Summary ==="
echo "Total runs  : ${TOTAL_RUNS}"
echo "Elapsed     : ${OVERALL_ELAPSED}s"
echo "Output dir  : ${PERF_DIR}"

if [[ ${#FAILED_RUNS[@]} -gt 0 ]]; then
    echo "Failed runs :"
    for f in "${FAILED_RUNS[@]}"; do
        echo "  - $f"
    done
    exit 1
else
    echo "All runs completed successfully."
fi
