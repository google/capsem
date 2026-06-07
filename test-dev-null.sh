#!/bin/sh
mkdir -p /newroot/dev
mount -t devtmpfs devtmpfs /newroot/dev
mount --move /dev /newroot/dev
ls -l /dev/null 2>&1 || echo "As expected, /dev/null is missing"
