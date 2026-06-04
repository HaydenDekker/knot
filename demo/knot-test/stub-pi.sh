#!/usr/bin/env bash
# stub-pi.sh — mimics `pi -p` for demo/testing.
#
# Parses --model, --system-prompt, and @<file> arguments.
# If model contains "nonexistent", exits with code 1.
# Otherwise reads @<file> content and stdin, echoes structured output.
set -euo pipefail

SYSTEM_PROMPT=""
MODEL=""
FILE_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        -p)
            shift
            ;;
        --model)
            MODEL="$2"
            shift 2
            ;;
        --system-prompt)
            SYSTEM_PROMPT="$2"
            shift 2
            ;;
        --no-session|--no-tools)
            shift
            ;;
        --tool)
            shift 2
            ;;
        @*)
            FILE_ARGS+=("$1")
            shift
            ;;
        *)
            shift
            ;;
    esac
done

# Simulate error for nonexistent models
if echo "$MODEL" | grep -q "nonexistent"; then
    echo "Error: model '$MODEL' not found" >&2
    exit 1
fi

# Read stdin (the prompt sent by SubprocessAgentRunner)
STDIN_CONTENT=$(cat)

# Output what we received so integration tests can verify
{
    echo "=== SYSTEM PROMPT ==="
    echo "$SYSTEM_PROMPT"
    echo "=== MODEL ==="
    echo "$MODEL"
    echo "=== STRAND FILES ==="
    for f in "${FILE_ARGS[@]}"; do
        filepath="${f#@}"
        if [ -f "$filepath" ]; then
            echo "FILE: $filepath"
            cat "$filepath"
        fi
    done
    echo "=== STDIN ==="
    echo "$STDIN_CONTENT"
}
