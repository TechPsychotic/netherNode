**Test build. Not code-signed yet**: see "Installing" below.

## Downloads

| Platform | File |
|---|---|
| Windows 10/11 (x64; runs on ARM via emulation) | `netherNode-…-windows-x64.zip` |
| macOS 11+ (Intel and Apple Silicon) | `netherNode-…-macos-universal.dmg` (or `.zip`) |
| Linux x64 | `netherNode-…-linux-x64.tar.gz` |

## Installing

- **Windows:** unzip, run `nethernode-gui.exe`. If SmartScreen appears: *More info → Run anyway*.
- **macOS:** open the `.dmg`, drag netherNode to Applications, then right-click it → *Open* → *Open* (first time only). If macOS says it's damaged: `xattr -dr com.apple.quarantine /Applications/netherNode.app`.
- **Linux:** extract, then run `./install.sh` (adds it to the app menu) or `./nethernode-gui` directly.

## What's in it

- Offline accounts only: no Microsoft or Mojang login.
- Every Minecraft version, with the matching Java downloaded automatically.
- Fabric, Quilt, Forge, NeoForge and OptiFine (OptiFine from a jar you download).
- Mods and modpacks from Modrinth, `.mrpack` import, drag & drop.
- Memory sized to your computer, crash detection with fixes, and a 1280×720 game window by default.

## Please check when testing

- The app starts, and an account and an instance can be created.
- A vanilla version (e.g. 1.21.1) and an older one (e.g. 1.12.2) launch.
- A Fabric instance with Sodium launches.
- Closing the launcher while a game runs stops the game.
- On Apple Silicon: 1.16.5 or older launches (it runs Java under Rosetta 2).
- On Windows: no extra black console windows appear.
