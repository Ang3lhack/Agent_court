#!/bin/sh
# Controlled test process; never invokes a real model or Codex executable.
printf '%s\n' "$@" > agent-args.txt
while [ "$#" -gt 0 ]; do
    if [ "$1" = '--output-last-message' ]; then
        shift
        printf '%s' 'controlled test response' > "$1"
    fi
    shift
done
printf '%s\n' '{"type":"test_event"}'
