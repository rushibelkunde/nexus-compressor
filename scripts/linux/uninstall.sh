#!/usr/bin/env bash
# ==============================================================================
# NEXUS Linux / Ubuntu Uninstaller
# ==============================================================================

set -e

# Colors
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${CYAN}==========================================================${NC}"
echo -e "${CYAN}       NEXUS Archiver - Linux / Ubuntu Uninstaller        ${NC}"
echo -e "${CYAN}==========================================================${NC}"

if [ "$(id -u)" -eq 0 ]; then
    INSTALL_BIN="/usr/local/bin"
    INSTALL_APPS="/usr/share/applications"
    INSTALL_ICONS="/usr/share/icons/hicolor/256x256/apps"
    INSTALL_MIME="/usr/share/mime/packages"
else
    INSTALL_BIN="$HOME/.local/bin"
    INSTALL_APPS="$HOME/.local/share/applications"
    INSTALL_ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
    INSTALL_MIME="$HOME/.local/share/mime/packages"
fi

echo -e "${YELLOW}[*] Removing binaries and desktop files...${NC}"

rm -f "$INSTALL_BIN/nexus"
rm -f "$INSTALL_BIN/nexus-gui"
rm -f "$INSTALL_APPS/nexus.desktop"
rm -f "$INSTALL_ICONS/nexus.png"
rm -f "$INSTALL_MIME/nexus.xml"

# Remove Nautilus scripts
NAUTILUS_SCRIPTS="$HOME/.local/share/nautilus/scripts"
rm -f "$NAUTILUS_SCRIPTS/Compress with NEXUS"
rm -f "$NAUTILUS_SCRIPTS/Extract with NEXUS"
rm -f "$NAUTILUS_SCRIPTS/Extract Here (NEXUS)"

# Update databases
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$INSTALL_APPS" 2>/dev/null || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
    update-mime-database "${INSTALL_MIME%/packages}" 2>/dev/null || true
fi

echo -e "${GREEN}[SUCCESS] NEXUS has been completely uninstalled from Linux.${NC}"
