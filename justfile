set shell := ["bash", "-euc", "-o", "pipefail"]
set windows-shell := ["bash", "-euc", "-o", "pipefail"]
set quiet

# Homebrew installs rustup as keg-only. Use the system PATH elsewhere.
export PATH := if os() == "macos" { "/opt/homebrew/opt/rustup/bin:" + env_var("PATH") } else { env_var("PATH") }

# --- ANSI Colors ---

blue := '\033[1;34m'
green := '\033[1;32m'
nc := '\033[0m'

# Show available commands
default:
    just --list

# Install locked frontend dependencies
sync:
    printf "\n{{ blue }}=== Installing Dependencies ==={{ nc }}\n"
    pnpm --dir app install --frozen-lockfile

# Format Rust and frontend code
format: sync
    printf "\n{{ blue }}=== Formatting Code ==={{ nc }}\n"
    cargo fmt --all
    pnpm --dir app format

# Check formatting without editing files
fmt-check: sync
    cargo fmt --all --check
    pnpm --dir app format:check

# Run Rust and frontend linters
lint: sync
    printf "\n{{ blue }}=== Running Linters ==={{ nc }}\n"
    cargo clippy --locked --workspace --all-targets -- -D warnings
    pnpm --dir app lint

# Check TypeScript types
typecheck: sync
    printf "\n{{ blue }}=== Running Type Checks ==={{ nc }}\n"
    pnpm --dir app typecheck

# Run Rust workspace and frontend tests
test: sync
    printf "\n{{ blue }}=== Running Tests ==={{ nc }}\n"
    cargo test --locked --workspace
    pnpm --dir app test

# Run the local CI suite on demand
ci: fmt-check lint typecheck test
    printf "\n{{ green }}✔ Local CI pipeline completed successfully.{{ nc }}\n"

alias setup := sync
alias fmt := format
alias check := ci

# Run the desktop app with hot reload
dev: sync
    pnpm --dir app tauri dev

# Build and run the app as a release behaves, without hot reload
dev-optimized: sync
    pnpm --dir app tauri build --no-bundle
    target/release/Overtone

# Build the macOS app bundle (target/release/bundle/macos/)
build: sync
    pnpm --dir app tauri build --bundles app
