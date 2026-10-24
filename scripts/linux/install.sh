#!/usr/bin/env bash
# ==============================================================================
# NEXUS Linux / Ubuntu Installer
# Installs nexus CLI, nexus-gui, desktop entry, MIME types, and Nautilus integration
# Works both as root (system-wide) or regular user (~/.local)
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${CYAN}==========================================================${NC}"
echo -e "${CYAN}        NEXUS Archiver - Linux / Ubuntu Installer         ${NC}"
echo -e "${CYAN}==========================================================${NC}"

# Detect if running as root or regular user
if [ "$(id -u)" -eq 0 ]; then
    INSTALL_BIN="/usr/local/bin"
    INSTALL_APPS="/usr/share/applications"
    INSTALL_ICONS="/usr/share/icons/hicolor/256x256/apps"
    INSTALL_MIME="/usr/share/mime/packages"
    echo -e "[*] Running as root: Installing system-wide..."
else
    INSTALL_BIN="$HOME/.local/bin"
    INSTALL_APPS="$HOME/.local/share/applications"
    INSTALL_ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
    INSTALL_MIME="$HOME/.local/share/mime/packages"
    echo -e "[*] Running as user: Installing to $HOME/.local..."
fi

mkdir -p "$INSTALL_BIN" "$INSTALL_APPS" "$INSTALL_ICONS" "$INSTALL_MIME"

# 1. Locate binaries
BIN_CLI=""
BIN_GUI=""

for loc in "$SCRIPT_DIR/nexus" "$SCRIPT_DIR/target/release/nexus" "$SCRIPT_DIR/../target/release/nexus" "$SCRIPT_DIR/../../target/release/nexus"; do
    if [ -f "$loc" ]; then
        BIN_CLI="$loc"
        break
    fi
done

for loc in "$SCRIPT_DIR/nexus-gui" "$SCRIPT_DIR/target/release/nexus-gui" "$SCRIPT_DIR/../target/release/nexus-gui" "$SCRIPT_DIR/../../target/release/nexus-gui"; do
    if [ -f "$loc" ]; then
        BIN_GUI="$loc"
        break
    fi
done

if [ -z "$BIN_CLI" ] || [ -z "$BIN_GUI" ]; then
    echo -e "${YELLOW}[*] Binaries not found in script folder. Attempting cargo build --release...${NC}"
    if command -v cargo >/dev/null 2>&1; then
        cargo build --release --bin nexus --bin nexus-gui
        BIN_CLI="$SCRIPT_DIR/../../target/release/nexus"
        BIN_GUI="$SCRIPT_DIR/../../target/release/nexus-gui"
    else
        echo -e "\033[0;31m[ERROR] nexus or nexus-gui binaries not found and cargo is not installed.\033[0m"
        exit 1
    fi
fi

# 2. Install Binaries
echo -e "${GREEN}[+] Installing nexus CLI to $INSTALL_BIN/nexus...${NC}"
install -m 755 "$BIN_CLI" "$INSTALL_BIN/nexus"

echo -e "${GREEN}[+] Installing nexus-gui to $INSTALL_BIN/nexus-gui...${NC}"
install -m 755 "$BIN_GUI" "$INSTALL_BIN/nexus-gui"

# 3. Install Icon
ICON_SRC=""
for loc in "$SCRIPT_DIR/nexus_logo.png" "$SCRIPT_DIR/nexus.png" "$SCRIPT_DIR/assets/nexus_logo.png" "$SCRIPT_DIR/../../assets/nexus_logo.png"; do
    if [ -f "$loc" ]; then
        ICON_SRC="$loc"
        break
    fi
done

if [ -n "$ICON_SRC" ]; then
    echo -e "${GREEN}[+] Installing application icon...${NC}"
    install -m 644 "$ICON_SRC" "$INSTALL_ICONS/nexus.png"
fi

# 4. Install Desktop Launcher
DESKTOP_SRC=""
for loc in "$SCRIPT_DIR/nexus.desktop" "$SCRIPT_DIR/scripts/linux/nexus.desktop"; do
    if [ -f "$loc" ]; then
        DESKTOP_SRC="$loc"
        break
    fi
done

if [ -n "$DESKTOP_SRC" ]; then
    echo -e "${GREEN}[+] Installing desktop entry...${NC}"
    install -m 644 "$DESKTOP_SRC" "$INSTALL_APPS/nexus.desktop"
fi

# 5. Install MIME type definition
MIME_SRC=""
for loc in "$SCRIPT_DIR/nexus.xml" "$SCRIPT_DIR/scripts/linux/nexus.xml"; do
    if [ -f "$loc" ]; then
        MIME_SRC="$loc"
        break
    fi
done

if [ -n "$MIME_SRC" ]; then
    echo -e "${GREEN}[+] Installing .nxar MIME type definition...${NC}"
    install -m 644 "$MIME_SRC" "$INSTALL_MIME/nexus.xml"
fi

# 6. Install Nautilus / GNOME Files Context Menu Scripts
NAUTILUS_SCRIPTS="$HOME/.local/share/nautilus/scripts"
if [ -d "$HOME/.local/share/nautilus" ] || [ -n "$GNOME_DESKTOP_SESSION_ID" ] || [ "$XDG_CURRENT_DESKTOP" = "GNOME" ] || [ "$XDG_CURRENT_DESKTOP" = "ubuntu:GNOME" ]; then
    mkdir -p "$NAUTILUS_SCRIPTS"
    
    cat > "$NAUTILUS_SCRIPTS/Compress with NEXUS" << 'EOF'
#!/bin/bash
if [ -n "$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS" ]; then
    FIRST_TARGET=$(echo "$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS" | head -n 1)
    nexus-gui compress "$FIRST_TARGET"
fi
EOF
    chmod +x "$NAUTILUS_SCRIPTS/Compress with NEXUS"

    cat > "$NAUTILUS_SCRIPTS/Extract with NEXUS" << 'EOF'
#!/bin/bash
if [ -n "$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS" ]; then
    FIRST_TARGET=$(echo "$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS" | head -n 1)
    nexus-gui decompress "$FIRST_TARGET"
fi
EOF
    chmod +x "$NAUTILUS_SCRIPTS/Extract with NEXUS"

    cat > "$NAUTILUS_SCRIPTS/Extract Here (NEXUS)" << 'EOF'
#!/bin/bash
if [ -n "$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS" ]; then
    FIRST_TARGET=$(echo "$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS" | head -n 1)
    nexus-gui decompress "$FIRST_TARGET" --auto-start
fi
EOF
    chmod +x "$NAUTILUS_SCRIPTS/Extract Here (NEXUS)"
    echo -e "${GREEN}[+] Installed GNOME / Nautilus right-click context scripts.${NC}"
fi

# 7. Update System Databases
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$INSTALL_APPS" 2>/dev/null || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
    update-mime-database "${INSTALL_MIME%/packages}" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f "${INSTALL_ICONS%/256x256/apps}" 2>/dev/null || true
fi

# 8. Check PATH for regular user
if [ "$(id -u)" -ne 0 ]; then
    if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
        echo -e "${YELLOW}[!] Note: $HOME/.local/bin is not in your current PATH.${NC}"
        echo -e "    Add it by running: echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
    fi
fi

echo -e "${CYAN}==========================================================${NC}"
echo -e "${GREEN} [SUCCESS] NEXUS installed successfully on Linux!        ${NC}"
echo -e " - CLI Engine : $INSTALL_BIN/nexus"
echo -e " - GUI App    : $INSTALL_BIN/nexus-gui"
echo -e " - Launch     : Search 'NEXUS Archiver' in Ubuntu App Menu"
echo -e "${CYAN}==========================================================${NC}"
