#!/bin/sh
# Controlled test process. All response data stays in separate test-owned files.
while IFS= read -r line; do
    printf '%s\n' "$line"
done < stdout
while IFS= read -r line; do
    printf '%s\n' "$line" >&2
done < stderr
IFS= read -r status < status
exit "$status"
