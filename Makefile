BINARY := target/debug/capsem
ASSETS := $(CURDIR)/assets
ENTITLEMENTS := entitlements.plist
APP_NAME := Capsem
TAURI_KEY := $(HOME)/.tauri/capsem.key

.PHONY: run build sign frontend release release-sign assets-check clean

# --- Development ---
run: sign
	CAPSEM_ASSETS_DIR=$(ASSETS) $(BINARY)

sign: build
	codesign --sign - --entitlements $(ENTITLEMENTS) --force $(BINARY)

build: frontend
	cargo build -p capsem

frontend:
	cd frontend && pnpm build

# --- Release ---
release: assets-check frontend
	cd crates/capsem-app && cargo tauri build

release-sign: release
	codesign --sign - --entitlements $(ENTITLEMENTS) --force --deep \
		"target/release/bundle/macos/$(APP_NAME).app"

assets-check:
	@test -f $(ASSETS)/vmlinuz || (echo "ERROR: assets not built. Run: cd images && python3 build.py" && exit 1)

clean:
	cargo clean
	rm -rf frontend/build frontend/.svelte-kit
