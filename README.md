<p align="center">
  <img src="assets/icons/png/128.png" width="96" alt="netherNode icon">
</p>

<h1 align="center">netherNode</h1>

<p align="center">
  An offline Minecraft launcher for Windows, macOS and Linux: every version, every major mod loader, Modrinth mods and modpacks, your own servers, and playing with friends over the internet without port forwarding.
</p>

<p align="center">
  <img src="docs/screenshots/home.png" alt="netherNode home screen: instance cards, details panel and play bar over an animated nether portal">
</p>

## Features

**Play**
- **Offline accounts.** Play with any username. No Microsoft or Mojang login.
- **Every Minecraft version**, from old alphas to today's snapshot, straight from Mojang's list. The right Java is downloaded for you.
- **Fits your PC.** Game memory is sized to your RAM and to what's free right now. The game opens at 1280×720 or any size you choose, and low-RAM machines get lighter settings.
- **Helpful crash messages.** netherNode reads crash reports and Java errors and tells you the likely fix.

**Mod**
- **Mod loaders in one click:** Fabric, Quilt, Forge (including pre-1.13) and NeoForge. Loaders that haven't released for a version yet are greyed out.
- **OptiFine, solved.** A one-click open-source alternative (Sodium, Iris shaders, connected textures, dynamic lights, zoom and more), or a real OptiFine jar.
- **Mods from Modrinth**, filtered to what works with the instance, with dependencies installed automatically. Turn mods on and off, or drop `.jar` files onto the window.
- **Modpacks** from Modrinth in one click, or import `.mrpack` files.

**Together**
- **Play with friends over the internet.** Open your world to LAN, click *Invite friends*, and send the code. Your friend clicks *Join friend* and finds your world in Minecraft's LAN list. No port forwarding, works behind CGNAT, end-to-end encrypted, and you approve every friend.
- **Host a server on your PC:** Vanilla, Fabric (mods) or Paper (plugins), with a live console, settings and a whitelist. Friends can join any time it's running, even when you're not playing.
- **Desktop notifications** when a friend wants in or your server is ready, so you notice even in fullscreen.

**Your stuff**
- **Bring your worlds:** import world folders or `.zip` backups into instances and servers.
- **Your folders:** keep game files, instances and servers on any drive (Settings).
- **A launcher that looks the part:** obsidian and violet glass over a live nether portal that pauses while you play. Textures come from your own game files; netherNode never ships Mojang's.

## Screenshots

| | |
|---|---|
| ![New instance with loader picker](docs/screenshots/new.png) | ![Browse Modrinth mods](docs/screenshots/browse.png) |
| **New instance:** any version, any loader, OptiFine alternative | **Mods:** search and install from Modrinth |
| ![Hosting a server](docs/screenshots/servers.png) | ![Modpacks](docs/screenshots/modpacks.png) |
| **Servers:** host on your PC and share with friends | **Modpacks:** one click, or import `.mrpack` |
| ![Instance settings](docs/screenshots/edit.png) | ![Settings](docs/screenshots/settings.png) |
| **Instance settings:** loader, window size, worlds, memory | **Settings:** folders, graphics |
| ![Game running](docs/screenshots/playing.png) | ![Installed mods](docs/screenshots/mods.png) |
| **Playing:** live game log, one-click stop | **Installed mods:** on/off, remove, drop jars in |

## Download

Get the latest version from [**Releases**](../../releases).

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

## Playing with friends

**Your world, while you play**
1. Start your game and load a world. Press **Esc → Open to LAN → Start LAN World**.
2. In netherNode, click **Invite friends** and send the code it shows (**Copy code**).
3. Your friend clicks **Join friend**, pastes the code, and you click **Allow** when asked. You also get a desktop notification.
4. Your friend opens **Multiplayer**; your world is in the LAN list. They can also Direct Connect to the `localhost` address netherNode shows them.

**A server, even when you're not playing**
1. **Servers → + New server**: name, Minecraft version, Vanilla / Fabric / Paper. Accept the Minecraft EULA.
2. **Start** it. When it's ready, **Invite friends** works just like above, and **Play on it** puts you in too.

Both use the same Minecraft version on both sides; with mods, friends need the same mods.

**How it works:** netherNode uses [iroh](https://iroh.computer). Both computers connect outwards and punch a direct path through NAT when they can; otherwise traffic goes through a relay over HTTPS that only sees encrypted data. The code contains a random secret, you approve each friend, and only the Minecraft port is reachable.

## Command line

The `nethernode` binary does everything the app does:

```sh
nethernode account add Steve
nethernode instance create survival 1.21.1
nethernode instance create modded 1.21.1 --loader fabric --resolution 1920x1080
nethernode mods add modded sodium                 # plus required dependencies
nethernode mods optifine-alternative modded
nethernode modpack install fabulously-optimized
nethernode launch survival

nethernode server create smp 1.21.1 --kind paper --accept-eula
nethernode server start smp --share               # prints a join code for friends
nethernode share join <code>                       # join a friend's world or server
```

Run `nethernode --help` for all commands.

## Where things are stored

| OS | Default folder |
|---|---|
| Windows | `%APPDATA%\nethernode\data` |
| macOS | `~/Library/Application Support/nethernode` |
| Linux | `~/.local/share/nethernode` |

Game files, instances and servers can each be moved in **Settings**. Game files, libraries and Java are shared between instances, and each instance has its own `.minecraft` folder for worlds, mods and settings. Set `NETHERNODE_HOME` to use a completely separate folder (a portable install).

## Building from source

Requires [Rust](https://rustup.rs) (stable).

```sh
cargo build --release
target/release/nethernode-gui     # the app
target/release/nethernode         # the CLI
```

The workspace has three crates:

- `crates/core`: the launcher library (versions, downloads, Java, loaders, mods, modpacks, servers, sharing, crash analysis)
- `crates/cli`: the `nethernode` command
- `crates/gui`: the `nethernode-gui` app, built with [iced](https://iced.rs)

Pushing a `v*` tag builds Windows, macOS and Linux releases through GitHub Actions.

## FAQ

**Can I join online-mode servers?** No. Offline accounts work in singleplayer, LAN, servers in offline mode (like the ones netherNode hosts), and friends' worlds through *Join friend*.

**Is a server in offline mode safe?** It can't check names, so anyone who gets in could use any name. netherNode only lets in friends you approve, and you can turn on a whitelist in the server's settings.

**What about OptiFine?** Pick *OptiFine* when creating an instance. The default, *One-click alternative*, installs open-source mods that do what OptiFine does: Sodium, Iris (OptiFine shader packs work), Continuity, Entity Texture/Model Features, dynamic lights, zoom and more. OptiFine itself can't be downloaded by other apps (its author doesn't allow it), so the real thing needs a jar from optifine.net, which netherNode then installs for you.

**Does it include Minecraft's textures?** No. The launcher reads textures from the game files it downloads from Mojang for you, and uses its own artwork until then.

## Credits

The interface uses the [Inter](https://rsms.me/inter/) and [JetBrains Mono](https://www.jetbrains.com/lp/mono/) fonts, both under the SIL Open Font License (see `assets/fonts/`). Networking for *Invite friends* is built on [iroh](https://iroh.computer).

## Roadmap

See [ROADMAP.md](ROADMAP.md) for what's done and what's next.
