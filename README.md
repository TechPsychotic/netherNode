<p align="center">
  <img src="assets/icons/png/128.png" width="96" alt="netherNode icon">
</p>

<h1 align="center">netherNode</h1>

<p align="center">
  An offline Minecraft launcher for Windows, macOS and Linux, with Fabric, Forge, NeoForge, Quilt, OptiFine, and Modrinth mods and modpacks.
</p>

<p align="center">
  <img src="docs/screenshots/home.png" alt="netherNode home screen: instance cards, details panel and play bar over an animated nether portal">
</p>

## Features

- **Offline accounts.** Play with any username. No Microsoft or Mojang login, now or later.
- **Every Minecraft version.** From old alphas to the newest snapshot, straight from Mojang's version list. The right Java is downloaded automatically.
- **Mod loaders in one click.** Fabric, Quilt, Forge (including pre-1.13), NeoForge, and OptiFine from a jar you download.
- **Mods from Modrinth.** Search only shows mods compatible with the instance. Dependencies install automatically, and mods can be turned on and off or added by drag & drop.
- **Modpacks.** Install from Modrinth or import `.mrpack` files. Each pack becomes its own instance.
- **Light on low-end PCs.** Game memory is sized to your RAM and to what's free right now. Low-RAM machines get lighter settings and a lighter launcher.
- **Helpful crash messages.** netherNode reads crash reports and Java errors, and tells you the likely fix.
- **A launcher that looks the part.** Obsidian and violet glass over a live nether portal that pauses while you play. It uses your own game files for textures and never ships Mojang's.

## Screenshots

| | |
|---|---|
| ![New instance with loader picker](docs/screenshots/new.png) | ![Browse Modrinth mods](docs/screenshots/browse.png) |
| **New instance:** any version, any loader | **Mods:** search and install from Modrinth |
| ![Installed mods](docs/screenshots/mods.png) | ![Modpacks](docs/screenshots/modpacks.png) |
| **Installed mods:** turn on/off, remove, drop jars in | **Modpacks:** one click, or import `.mrpack` |
| ![Instance settings](docs/screenshots/edit.png) | ![Game running](docs/screenshots/playing.png) |
| **Settings:** loader, window size, memory, Java | **Playing:** live game log, one-click stop |

## Download

Get the latest build from [**Releases**](../../releases).

| Platform | File |
|---|---|
| Windows 10/11 (x64, runs on ARM too) | `netherNode-…-windows-x64.zip` |
| macOS 11+ (Intel and Apple Silicon) | `netherNode-…-macos-universal.dmg` |
| Linux x64 | `netherNode-…-linux-x64.tar.gz` |

The builds aren't code-signed yet:

- **Windows:** if SmartScreen appears, click *More info*, then *Run anyway*.
- **macOS:** right-click the app, choose *Open*, then *Open* again. If macOS says it's damaged, run `xattr -dr com.apple.quarantine /Applications/netherNode.app`.
- **Linux:** run `./install.sh` to add netherNode to your app menu. Minecraft 1.12.2 and older also need the `xrandr` program.

## Getting started

1. Add an offline account: any name of 3–16 letters, digits or `_`.
2. Click **+ New instance**, pick a Minecraft version and, if you want mods, a loader.
3. Double-click the instance (or press **PLAY**). The first launch downloads the game and Java, which takes a few minutes.
4. For mods, open the instance's **Mods** screen. For ready-made packs, use **Modpacks** in the top bar.

## Command line

The `nethernode` binary does everything the app does:

```sh
nethernode account add Steve
nethernode instance create survival 1.21.1
nethernode instance create modded 1.21.1 --loader fabric --resolution 1920x1080
nethernode mods add modded sodium          # plus required dependencies
nethernode modpack install fabulously-optimized
nethernode launch survival
```

Run `nethernode --help` for all commands.

## Where things are stored

| OS | Folder |
|---|---|
| Windows | `%APPDATA%\nethernode\data` |
| macOS | `~/Library/Application Support/nethernode` |
| Linux | `~/.local/share/nethernode` |

Set `NETHERNODE_HOME` to use another folder. Game files, libraries and Java runtimes are shared between instances. Each instance has its own `.minecraft` folder for worlds, mods and settings.

## Building from source

Requires [Rust](https://rustup.rs) (stable).

```sh
cargo build --release
target/release/nethernode-gui     # the app
target/release/nethernode         # the CLI
```

The workspace has three crates:

- `crates/core`: the launcher library (versions, downloads, Java, loaders, mods, modpacks, crash analysis)
- `crates/cli`: the `nethernode` command
- `crates/gui`: the `nethernode-gui` app, built with [iced](https://iced.rs)

Pushing a `v*` tag builds Windows, macOS and Linux releases through GitHub Actions.

## FAQ

**Can I join online-mode servers?** No. Offline accounts work in singleplayer, LAN, and servers running in offline mode.

**Why can't netherNode download OptiFine for me?** OptiFine's author doesn't allow automated downloads. netherNode opens the official download page and installs the jar you choose. Fabric with Sodium and Iris is a one-click alternative.

**Does it include Minecraft's textures?** No. The launcher reads textures from the game files it downloads from Mojang for you, and uses its own artwork until then.

## Roadmap

See [ROADMAP.md](ROADMAP.md) for what's done and what's next.
