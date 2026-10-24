# NEXUS Shell Integration & OS Right-Click Menu

NEXUS provides seamless operating system integration so you can compress and decompress files and folders directly from your file manager with a single right-click, just like 7-Zip or WinRAR.

---

## 🪟 Windows (File Explorer Integration)

### Quick 1-Click Install
Open PowerShell (no admin rights required) and run:
```powershell
.\scripts\windows\install.ps1
```

### What This Does:
1. Installs `nexus.exe` to `%LOCALAPPDATA%\Programs\NEXUS\nexus.exe`.
2. Adds `NEXUS` to your user `PATH` (usable from any Terminal or Command Prompt).
3. Adds Explorer context menu entries:
   - **Right-click ANY file or folder**: `Compress to .nxar (NEXUS)`
   - **Right-click ANY `.nxar` archive**:
     - `Extract Here (NEXUS)`
     - `Extract to folder (NEXUS)`
   - **Double-click ANY `.nxar` file**: Automatically decompresses the archive!

### Uninstallation:
```powershell
.\scripts\windows\uninstall.ps1
```

---

## 🐧 Linux / Ubuntu (Nautilus & GNOME Files Integration)

### Quick Install
Open a terminal and run:
```bash
chmod +x ./scripts/linux/install.sh
./scripts/linux/install.sh
```

### What This Does:
1. Installs `nexus` to `~/.local/bin/nexus`.
2. Registers the `application/x-nexus-archive` MIME type for `.nxar` and `.nxs`.
3. Installs Nautilus right-click context menu scripts:
   - **Right-click file/folder -> Scripts -> `Compress with NEXUS (.nxar)`**
   - **Right-click archive -> Scripts -> `Extract with NEXUS`**
4. Registers `nexus.desktop` for GUI file associations.

### Uninstallation:
```bash
chmod +x ./scripts/linux/uninstall.sh
./scripts/linux/uninstall.sh
```
