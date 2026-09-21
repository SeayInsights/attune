#!/usr/bin/env bash
#
# Fails if a desktop-toolkit crate resolves into the Linux dependency graph.
#
# Attune ships to Windows 11 only, but the workspace is still checked on
# Linux and macOS by the inherited build.yml, and that is what keeps upstream
# merges honest. The failure mode pinned here: a crate added for the Windows
# UI that nobody gated, dragging a system library into the Linux graph that
# the workflow does not install.
#
# It has happened. attune-app took tao and wry unconditionally; they are the
# only edges into GTK3 and WebKitGTK, and the Linux job died at gdk-sys for
# as long as they were ungated. `cargo check` cannot catch it on Windows
# because the graph is what is wrong, not the code -- and a bare `cargo tree`
# only prints the graph, exiting 0 either way. This asserts on it.
#
# Run it by hand the same way CI does:  bash ci/check-linux-graph.sh

set -euo pipefail

TARGET=x86_64-unknown-linux-gnu
PATTERN='(gtk|gdk|webkit2gtk|soup3)[a-z0-9-]* v'

# stdout only: cargo writes Downloaded/Updating progress to stderr, and those
# lines name the crates too, so including them would match on every fresh run.
graph=$(cargo tree --target "$TARGET" --workspace --edges normal 2>/dev/null)

if hits=$(echo "$graph" | grep -E "$PATTERN"); then
  echo "::error::A desktop-toolkit crate is in the $TARGET dependency graph."
  echo "$hits"
  echo
  echo "Gate it on cfg(windows) in its crate's Cargo.toml, the way attune-app"
  echo "gates tao and wry. If gating would cascade too far -- as it would for"
  echo "cpal, which attune-analysis uses everywhere -- install the system"
  echo "library in build.yml instead and say whose dependency it is, the way"
  echo "libasound2-dev is handled."
  exit 1
fi

echo "Clean: no gtk/gdk/webkit2gtk/soup3 crate resolves for $TARGET."
