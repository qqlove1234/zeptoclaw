#!/bin/sh
set -e

# Fix permissions on data directory if running as root
if [ "$(id -u)" = '0' ]; then
    # Ensure data directory exists and has correct ownership
    mkdir -p /data
    chown -R zeptoclaw:zeptoclaw /data

    # Drop to zeptoclaw user and execute command
    exec gosu zeptoclaw "$@"
fi

exec "$@"
