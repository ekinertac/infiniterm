# The common commands. `make` lists them.
#
# The app never runs on the real data dir from here: DATA is a scratch
# directory holding a copy of the Tauri app's canvas, so nothing saves over
# the shipping app's file and nothing steals its socket (see HANDOVER.md,
# "Verifying without being at the Mac"). `make run DATA=...` moves it.

DATA ?= /tmp/infiniterm-dev
# Where the scratch app reads settings.json and keybindings.json. An instance
# with its own data dir starts clean by default; `make run` is the exception and
# reads your real settings; run-clean points it at an empty directory.
CONFIG_DIR ?= $(HOME)/.config/infiniterm
CLEAN_CONFIG = /tmp/infiniterm-dev-config
PROFILE ?= debug
APP = target/bundle/infiniterm.app
REAL = $(HOME)/Library/Application\ Support/dev.ekinertac.infiniterm/workspace.json
CRATES = -p infiniterm-core -p infiniterm-ui -p infiniterm-term

.PHONY: help build release install dist publish bundle run run-fresh run-clean stop log test check fmt clippy drive drive-drag drive-panel cast cast-seed shot clean

help:
	@echo "make run        build, bundle and launch on a copy of the real canvas ($(DATA))"
	@echo "make run-fresh  the same on an empty canvas"
	@echo "make run-clean  an empty canvas AND empty settings, as a new user sees the app"
	@echo "make stop       quit the running native app"
	@echo "make log        follow the app's log (run.log)"
	@echo "make test       cargo test, the whole workspace"
	@echo "make check      fmt + clippy -D warnings + test"
	@echo "make fmt        cargo fmt on the three port crates (never the cli and hook copies)"
	@echo "make drive      the scripted GUI run with screenshots (say so first if Ekin is on the Mac)"
	@echo "make drive-drag / drive-panel   the other scenarios"
	@echo "make cast-seed  build the screencast's demo repo and canvas (off camera)"
	@echo "make cast       the screencast take, on the seeded canvas"
	@echo "make shot       screenshot the running window to /tmp/infiniterm-shot.png"
	@echo "make release    optimised build and bundle, signed with the Developer ID"
	@echo "make install    make release, then into /Applications (the running app keeps its cards: iftd holds them)"
	@echo "make dist       notarized, stapled DMG + update zip + latest.json in target/dist"
	@echo "make publish    put target/dist on ekinertac/infiniterm's releases (NOTES=..., BRIDGE=1)"

build:
	cargo build $(if $(filter release,$(PROFILE)),--release,) -p infiniterm-ui

bundle: build
	tools/bundle.sh $(PROFILE)
	tools/sign.sh

release:
	$(MAKE) bundle PROFILE=release

install: release
	ditto $(APP) /Applications/infiniterm.app

# The build that leaves this Mac: notarized, stapled DMG + update zip +
# latest.json in target/dist (tools/dist.sh), then onto the public
# releases repo (tools/publish.sh; NOTES="..." for release notes).
dist:
	tools/dist.sh

publish:
	tools/publish.sh

run: bundle stop
	@mkdir -p $(DATA) $(CONFIG_DIR)
	@[ -f $(DATA)/workspace.json ] || cp $(REAL) $(DATA)/workspace.json 2>/dev/null || true
	@: > run.log
	open --stderr "$(PWD)/run.log" --stdout "$(PWD)/run.log" --env INFINITERM_DATA_DIR=$(DATA) $(if $(CONFIG_DIR),--env INFINITERM_CONFIG_DIR=$(CONFIG_DIR)) --env INFINITERM_KEYLOG=1 $(APP)
	@echo "running on $(DATA); make log to follow"

run-fresh: stop
	rm -rf $(DATA)
	$(MAKE) run DATA=$(DATA)

run-clean: stop
	rm -rf $(DATA) $(CLEAN_CONFIG)
	$(MAKE) run DATA=$(DATA) CONFIG_DIR=$(CLEAN_CONFIG) REAL=/nonexistent

stop:
	-pkill -f "$(CURDIR)/target/bundle/infiniterm.app/Contents/MacOS/infiniterm$$" 2>/dev/null; sleep 0.3

log:
	tail -f run.log

test:
	cargo test

fmt:
	cargo fmt $(CRATES)

clippy:
	cargo clippy --workspace --all-targets --offline -- -D warnings

check: fmt clippy test

drive: bundle
	tools/drive/phase3.sh

drive-drag: bundle
	tools/drive/drag.sh

drive-panel: bundle
	tools/drive/panel.sh

# The screencast needs ift and the hook beside the app: the demo repo's
# hooks point at target/debug, and the take reads card state back with ift.
cast-seed: bundle
	cargo build -p infiniterm-cli -p infiniterm-hook
	caffeinate -disu -t 2400 tools/drive/cast.sh seed

cast: bundle
	cargo build -p infiniterm-cli -p infiniterm-hook
	caffeinate -disu -t 2400 tools/drive/cast.sh take

shot:
	tools/shot.sh infiniterm /tmp/infiniterm-shot.png

clean:
	cargo clean
	rm -rf target/bundle
