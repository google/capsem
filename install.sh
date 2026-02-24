#!/bin/bash
set -euo pipefail

APP_NAME="Capsem"
INSTALL_DIR="/Applications"

echo "Building $APP_NAME..."
make release-sign

echo "Stopping running $APP_NAME..."
pkill -x "$APP_NAME" 2>/dev/null && sleep 1 || true

echo "Installing to $INSTALL_DIR..."
rm -rf "$INSTALL_DIR/$APP_NAME.app"
cp -R "target/release/bundle/macos/$APP_NAME.app" "$INSTALL_DIR/"

echo "Launching $APP_NAME..."
open "$INSTALL_DIR/$APP_NAME.app"
