#!/bin/sh
# Deterministic Docker CLI test double. Never executes candidate code.
case "$1" in
    info) printf '%s\n' '29.9.2' ;;
    image) printf 'sha256:%064d\n' 1 ;;
    create)
        printf '%s\n' "$@" > create-args
        printf '%s\n' "$3" > owner
        if [ -f create-id ]; then
            while IFS= read -r line; do printf '%s\n' "$line"; done < create-id
        else
            printf '%064d\n' 2
        fi
        if [ -f create-code ]; then
            IFS= read -r result < create-code
            exit "$result"
        fi
        ;;
    start)
        while IFS= read -r line; do printf '%s\n' "$line"; done < output
        IFS= read -r result < exit-code
        exit "$result"
        ;;
    inspect)
        case "$3" in
            *'.Config.Labels'*)
                IFS= read -r owner < owner
                printf '%064d %s\n' 2 "$owner"
                ;;
            *) while IFS= read -r line; do printf '%s\n' "$line"; done < state ;;
        esac
        ;;
    rm)
        printf '%s\n' "$@" >> removed
        if [ -f rm-code ]; then
            IFS= read -r result < rm-code
            exit "$result"
        fi
        printf '%064d\n' 2
        ;;
    *) exit 125 ;;
esac
