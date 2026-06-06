#!/usr/bin/env bash
set -euo pipefail

# pgShield Agent Installer
# Usage: curl -fsSL https://pgshield.example.com/install.sh | bash
#   or: bash install.sh [--version 0.1.0]

BIN_DIR="${BIN_DIR:-/usr/local/bin}"
CONFIG_DIR="${CONFIG_DIR:-/etc/pgshield}"
DATA_DIR="${DATA_DIR:-/var/lib/pgshield}"
VERSION="${1:-0.1.0}"
REPO="pgshield"

echo "==> Installing pgShield Agent v${VERSION}"

# Create directories
mkdir -p "$BIN_DIR" "$CONFIG_DIR" "$DATA_DIR/backups"

# Download binary (placeholder — replace with actual release URL)
# BINARY_URL="https://github.com/yourorg/pgshield/releases/download/v${VERSION}/pgshield-agent-$(uname -s)-$(uname -m)"
# echo "==> Downloading from $BINARY_URL"
# curl -fsSL "$BINARY_URL" -o "$BIN_DIR/pgshield-agent"
# chmod +x "$BIN_DIR/pgshield-agent"

# Build from source instead
if command -v cargo &>/dev/null; then
    echo "==> Building from source..."
    REPO_DIR=$(mktemp -d)
    git clone --depth 1 --branch "v${VERSION}" "https://github.com/yourorg/pgshield.git" "$REPO_DIR" 2>/dev/null || {
        echo "Git clone failed, using local build"
        if [ -f "Cargo.toml" ]; then
            cargo build --release --package pgshield-agent
            cp target/release/pgshield-agent "$BIN_DIR/"
        else
            echo "No Cargo.toml found. Copy binary manually."
            exit 1
        fi
    }
    cd "$REPO_DIR"
    cargo build --release --package pgshield-agent
    cp target/release/pgshield-agent "$BIN_DIR/"
    rm -rf "$REPO_DIR"
else
    echo "Cargo not found. Please install Rust or place pgshield-agent binary in $BIN_DIR/"
    exit 1
fi

# Copy default config
if [ ! -f "$CONFIG_DIR/agent.yaml" ]; then
    cp config/agent.yaml "$CONFIG_DIR/agent.yaml" 2>/dev/null || cat > "$CONFIG_DIR/agent.yaml" << 'EOF'
server_url: "http://localhost:8080"
listen_addr: "0.0.0.0"
listen_port: 9443
backup_dir: "/var/lib/pgshield/backups"
tls_cert: ""
tls_key: ""
EOF
    echo "==> Created default config at $CONFIG_DIR/agent.yaml"
fi

# Install systemd service
if command -v systemctl &>/dev/null; then
    cat > /etc/systemd/system/pgshield-agent.service << 'UNIT'
[Unit]
Description=pgShield Backup Agent
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/pgshield-agent --config /etc/pgshield/agent.yaml
Restart=always
RestartSec=5
User=root
Group=root
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
UNIT
    systemctl daemon-reload
    echo "==> Installed systemd unit: pgshield-agent.service"
    echo "==> Start with: systemctl enable --now pgshield-agent"
fi

echo "==> Done! pgShield Agent installed."
echo "    Edit config: $CONFIG_DIR/agent.yaml"
echo "    Start: pgshield-agent --config $CONFIG_DIR/agent.yaml"
