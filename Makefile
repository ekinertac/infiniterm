# The common commands. `make` lists them.
#
# The app never runs on the real data dir from here: DATA is a scratch
# directory holding a copy of the Tauri app's canvas, so nothing saves over
# the shipping app's file and nothing steals its socket (see HANDOVER.md,
# "Verifying without being at the Mac"). `make run DATA=...` moves it.

DATA ?= /tmp/infiniterm-dev
PROFILE ?= debug
APP = target/bundle/infiniterm.app
REAL = $(HOME)/Library/Application\ Support/dev.ekinertac.infiniterm/workspace.json
CRATES = -p infiniterm-core -p infiniterm-ui -p infiniterm-term

.PHONY: help build release bundle run run-fresh stop log test check fmt clippy drive drive-drag drive-panel shot clean

help:
	@echo "make run        build, bundle and launch on a copy of the real canvas ($(DATA))"
	@echo "make run-fresh  the same on an empty canvas"
	@echo "make stop       quit the running native app"
	@echo "make log        follow the app's log (run.log)"
	@echo "make test       cargo test, the whole workspace"
	@echo "make check      fmt + clippy -D warnings + test"
	@echo "make fmt        cargo fmt on the three port crates (never the cli and hook copies)"
	@echo "make drive      the scripted GUI run with screenshots (say so first if Ekin is on the Mac)"
	@echo "make drive-drag / drive-panel   the other scenarios"
	@echo "make shot       screenshot the running window to /tmp/infiniterm-shot.png"
	@echo "make release    optimised build and bundle"

build:
	cargo build $(if $(filter release,$(PROFILE)),--release,) -p infiniterm-ui

bundle: build
	tools/bundle.sh $(PROFILE)

release:
	$(MAKE) bundle PROFILE=release

run: bundle stop
	@mkdir -p $(DATA)
	@[ -f $(DATA)/workspace.json ] || cp $(REAL) $(DATA)/workspace.json 2>/dev/null || true
	@: > run.log
	open --stderr "$(PWD)/run.log" --stdout "$(PWD)/run.log" --env INFINITERM_DATA_DIR=$(DATA) --env INFINITERM_KEYLOG=1 $(APP)
	@echo "running on $(DATA); make log to follow"

run-fresh: stop
	rm -rf $(DATA)
	$(MAKE) run DATA=$(DATA)

stop:
	-pkill -f "MacOS/infiniterm$$" 2>/dev/null; sleep 0.3

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

shot:
	tools/shot.sh infiniterm /tmp/infiniterm-shot.png

clean:
	cargo clean
	rm -rf target/bundle
