**netherNode 1.2.3**: offline Minecraft launcher for Windows, macOS and Linux.

## What's new

- **Joining a friend's server:** *Play it*, Direct Connect and the copied address use `127.0.0.1` instead of `localhost`, so the game can't pick an address the tunnel isn't on ("Connection refused: getsockopt").
- **Same address every time:** when the host's port is free on your PC, the tunnel uses that same port number, so a saved Multiplayer entry keeps working next time.
- Joining reminds you to keep netherNode open while you play (the tunnel runs inside it).

Everyone playing together needs netherNode 1.2 or newer. Full history: [CHANGELOG.md](https://github.com/TechPsychotic/netherNode/blob/main/CHANGELOG.md).

## Downloads

| Platform | File |
|---|---|
| Windows 10/11 (x64; runs on ARM via emulation) | `netherNode-…-windows-x64.zip` |
| macOS 11+ (Intel and Apple Silicon) | `netherNode-…-macos-universal.dmg` (or `.zip`) |
| Linux x64 | `netherNode-…-linux-x64.tar.gz` |

## Installing

The builds aren't code-signed yet:

- **Windows:** unzip, run `nethernode-gui.exe`. If SmartScreen appears: *More info → Run anyway*.
- **macOS:** open the `.dmg`, drag netherNode to Applications, then right-click it → *Open* → *Open* (first time only). If macOS says it's damaged: `xattr -dr com.apple.quarantine /Applications/netherNode.app`.
- **Linux:** extract, then run `./install.sh` (adds it to the app menu) or `./nethernode-gui` directly.

New here? See the [website](https://techpsychotic.github.io/netherNode/) and the [README](https://github.com/TechPsychotic/netherNode#readme).
