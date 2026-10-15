#!/bin/sh
set -eu

current_uid=$(id -u)
if [ "$current_uid" -eq 0 ]; then
    # Preserve the host owner; the service group only needs read and traversal.
    chgrp -Rh 10001 /data
    chmod -R g=rX /data

    # Replace the shell so Limen receives Docker's signals directly as PID 1.
    exec setpriv --reuid=10001 --regid=10001 --clear-groups \
        --inh-caps=-all --ambient-caps=-all --no-new-privs \
        /usr/local/bin/limen "$@"
fi

exec /usr/local/bin/limen "$@"
