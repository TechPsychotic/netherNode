netherNode: offline Minecraft launcher
=====================================

nethernode-gui   the launcher app
nethernode       the command-line version (run `nethernode --help`)

Your instances, accounts and downloads are stored in:
  Windows  %APPDATA%\nethernode\data
  macOS    ~/Library/Application Support/nethernode
  Linux    ~/.local/share/nethernode
(set NETHERNODE_HOME to use another folder)

First start
-----------
1. Add an offline account (any name, 3-16 letters, digits or _).
2. Click "New instance", pick a Minecraft version (and optionally a mod loader).
3. Double-click the instance or press PLAY. The first launch downloads the
   game and the right Java, which takes a few minutes.

These builds are not code-signed yet
------------------------------------
Windows: SmartScreen may say "Windows protected your PC".
         Click "More info", then "Run anyway".
macOS:   macOS may say the app "can't be opened" or "is damaged".
         Right-click netherNode.app, choose Open, then Open again; or run
           xattr -dr com.apple.quarantine /path/to/netherNode.app
Linux:   Run ./install.sh to add netherNode to your app menu
         (./install.sh --uninstall removes it). Minecraft 1.12.2 and older
         also need the `xrandr` program installed.
