# Jamelade — build and install to a personal (per-user) prefix.
#
# No sudo: this is a one-user, one-machine app (see ARCHITECTURE.md), so everything
# lands under ~/.local, which is already on PATH and XDG_DATA_DIRS. Override
# PREFIX for a system install (make PREFIX=/usr/local install, with sudo).
#
# Jamelade installs a *second* artefact: the Electron
# sidecar that owns DRM playback. It is ~200 MB of Chromium and is fetched by
# npm, never committed. The binary finds it via JAMELADE_SIDECAR, else
# $(DATADIR)/jamelade/sidecar, else ./sidecar for a dev tree.

PREFIX  ?= $(HOME)/.local
BINDIR   = $(PREFIX)/bin
DATADIR  = $(PREFIX)/share
APPID    = io.github.Jamelade.Jamelade
LAUNCHERID = $(APPID).Launcher
SIDECAR  = $(DATADIR)/jamelade/sidecar

ICON_SIZES = 16 24 32 48 64 128 256 512

.PHONY: all build run test check sizes sidecar-check sidecar sidecar-run gapless footprint install install-sidecar \
        dev-install uninstall clean flatpak flatpak-bundle appimage release-check
all: build

build:
	cargo build --release

run:
	cargo run

test: sidecar-check
	cargo test

# The bar from ARCHITECTURE.md. --all-targets so tests are linted too.
check: sizes sidecar-check
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

# A size budget, enforced as a ratchet. First, because it is instant and the
# thing it catches is drift you would otherwise only notice months later.
sizes:
	@./scripts/check-sizes.sh

sidecar-check:
	cd sidecar && npm test
	cd sidecar && node --check main.js && node --check preload.js \
		&& node --check page-hook.js && node --check security.js \
		&& node --check auth-preload.js && node --check session-vault.js \
		&& node --check login-email.js && node --check login-email-assist.js \
		&& node --check persistence.js

# Fetch castLabs Electron. Two steps, both required: `npm install` brings down
# the ~14 MB wrapper, and install.js fetches the ~200 MB Chromium itself.
# castLabs ships no postinstall hook, so skipping the second step leaves you
# with a package that has no binary in it.
# (The Widevine CDM itself arrives later, at first run, via Chromium's
# component updater — that needs network too.)
sidecar:
	cd sidecar && npm install && node node_modules/electron/install.js

# Run the sidecar standalone with its window visible — the isolation step from
# ARCHITECTURE.md. If a track plays here, DRM is fine and the bug is in the Rust side.
sidecar-run: sidecar
	cd sidecar && npm run debug

# Watch the audio stream across a track boundary. Run it in one terminal and
# `RUST_LOG=jamelade=info cargo run` in another — the log says whether Rust
# drove the transition, this says whether the decoder stopped.
gapless:
	./scripts/gapless-check.sh

# What the app costs the machine: memory, CPU and disk. Needs a running
# instance for the first two; `--disk` alone needs nothing.
footprint:
	./scripts/footprint.sh

# A native `flatpak-builder` if there is one, otherwise the Flathub app. They
# are the same tool; the difference is that the flatpak'd one runs sandboxed,
# and on a CI runner that sandbox cannot see runtimes installed into the user
# installation — it fails with `Unable to find sdk org.gnome.Sdk version 49`
# twenty seconds after installing exactly that.
FLATPAK_BUILDER := $(shell command -v flatpak-builder >/dev/null 2>&1 \
	&& echo flatpak-builder || echo flatpak run org.flatpak.Builder)

flatpak:
	$(FLATPAK_BUILDER) --force-clean --user --install \
		--repo=flatpak-repo build-dir packaging/flatpak/io.github.Jamelade.Jamelade.yml

# `--runtime-repo` is the difference between a bundle that installs and one
# that stops with "requires the runtime org.gnome.Platform/x86_64/49 which was
# not found". A .flatpak carries the *app* and never the runtime, so on a
# machine with no Flathub remote there is nothing for it to sit on and flatpak
# has no idea where to look. The URL is recorded inside the bundle, so
# installing it offers to add Flathub and pull the runtime itself.
#
# Found by installing on a clean Ubuntu VM, which is the only place it could
# have been found: every machine that has ever built this already had the
# runtime.
flatpak-bundle: flatpak
	flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
		flatpak-repo Jamelade.flatpak io.github.Jamelade.Jamelade master
	@echo "Jamelade.flatpak — copy it anywhere and: flatpak install ./Jamelade.flatpak"

appimage:
	./packaging/appimage/build.sh

release-check:
	./scripts/release-check.sh

install: build install-sidecar dev-install
	install -Dm755 target/release/jamelade $(BINDIR)/jamelade
	@echo "Installed to $(PREFIX). Launch 'Jamelade' from the app grid, or run 'jamelade'."

install-sidecar: sidecar
	install -d $(SIDECAR)
	cp -r sidecar/package.json sidecar/main.js sidecar/preload.js \
		sidecar/page-hook.js sidecar/security.js sidecar/auth-preload.js \
		sidecar/session-vault.js sidecar/login-email.js \
		sidecar/login-email-assist.js sidecar/persistence.js \
		sidecar/node_modules $(SIDECAR)/
	install -d $(DATADIR)/jamelade/companions
	cp data/companions/jambun.png data/companions/jampam.png \
		data/companions/jamjoe.png $(DATADIR)/jamelade/companions/
	install -d $(DATADIR)/jamelade/companions/launcher
	cp data/companions/launcher/*.png $(DATADIR)/jamelade/companions/launcher/
	install -d $(DATADIR)/jamelade/companions/animated
	cp -r data/companions/animated/jambun data/companions/animated/jampam \
		data/companions/animated/jamjoe $(DATADIR)/jamelade/companions/animated/
	install -d $(DATADIR)/jamelade/companions/animated-hq
	cp -r data/companions/animated-hq/jambun data/companions/animated-hq/jampam \
		data/companions/animated-hq/jamjoe $(DATADIR)/jamelade/companions/animated-hq/

# Everything except the binaries: desktop metadata and the icons.
# Not a way to get a dev-mode icon — on Wayland only the fully installed app
# shows one.
dev-install:
	install -Dm644 data/$(APPID).desktop $(DATADIR)/applications/$(APPID).desktop
	install -Dm644 data/$(LAUNCHERID).desktop $(DATADIR)/applications/$(LAUNCHERID).desktop
	install -Dm644 data/$(APPID).metainfo.xml $(DATADIR)/metainfo/$(APPID).metainfo.xml
	install -Dm644 data/icons/hicolor/symbolic/apps/$(APPID)-symbolic.svg \
		$(DATADIR)/icons/hicolor/symbolic/apps/$(APPID)-symbolic.svg
	@for sz in $(ICON_SIZES); do \
		install -Dm644 data/icons/hicolor/$${sz}x$${sz}/apps/$(APPID).png \
			$(DATADIR)/icons/hicolor/$${sz}x$${sz}/apps/$(APPID).png; \
	done
	@for jamkin in jambun jampam jamjoe; do \
		install -Dm644 data/companions/launcher/$$jamkin.png \
			$(DATADIR)/icons/hicolor/512x512/apps/$(APPID).$$jamkin.png; \
	done
	install -d $(DATADIR)/jamelade/companions
	install -m644 data/companions/jambun.png data/companions/jampam.png \
		data/companions/jamjoe.png $(DATADIR)/jamelade/companions/
	install -d $(DATADIR)/jamelade/companions/launcher
	cp data/companions/launcher/*.png $(DATADIR)/jamelade/companions/launcher/
	install -d $(DATADIR)/jamelade/companions/animated
	cp -r data/companions/animated/jambun data/companions/animated/jampam \
		data/companions/animated/jamjoe $(DATADIR)/jamelade/companions/animated/
	install -d $(DATADIR)/jamelade/companions/animated-hq
	cp -r data/companions/animated-hq/jambun data/companions/animated-hq/jampam \
		data/companions/animated-hq/jamjoe $(DATADIR)/jamelade/companions/animated-hq/
	@if [ -f $(DATADIR)/icons/hicolor/index.theme ]; then \
		touch $(DATADIR)/icons/hicolor; \
		gtk-update-icon-cache -q -t -f $(DATADIR)/icons/hicolor; \
	fi
	-update-desktop-database -q $(DATADIR)/applications

uninstall:
	rm -f $(BINDIR)/jamelade
	rm -rf $(DATADIR)/jamelade
	rm -f $(DATADIR)/applications/$(APPID).desktop
	rm -f $(DATADIR)/applications/$(LAUNCHERID).desktop
	rm -f $(DATADIR)/metainfo/$(APPID).metainfo.xml
	rm -f $(DATADIR)/icons/hicolor/symbolic/apps/$(APPID)-symbolic.svg
	@for sz in $(ICON_SIZES); do \
		rm -f $(DATADIR)/icons/hicolor/$${sz}x$${sz}/apps/$(APPID).png; \
	done
	@for jamkin in jambun jampam jamjoe; do \
		rm -f $(DATADIR)/icons/hicolor/512x512/apps/$(APPID).$$jamkin.png; \
	done
	@if [ -f $(DATADIR)/icons/hicolor/index.theme ]; then \
		gtk-update-icon-cache -q -t -f $(DATADIR)/icons/hicolor; \
	fi
	-update-desktop-database -q $(DATADIR)/applications
	@echo "Uninstalled from $(PREFIX)."

clean:
	cargo clean
	rm -rf sidecar/node_modules
