# netherNode Roadmap

An offline-accounts Minecraft launcher for Linux, macOS and Windows, written in Rust:
`crates/core` (library), `crates/cli` (`nethernode`) and `crates/gui` (`nethernode-gui`, iced).
Current release: **1.2.0**.

## Ground rules

- **No Microsoft or Mojang login, ever.** Only offline accounts. Game files still come from Mojang's public servers.
- **No CurseForge.** Modrinth is the only source for mods and modpacks.
- **Never ship Mojang's files or art.** Game jars, assets and textures are downloaded or read at runtime.
- **Cross-platform:** every feature has to work on Linux (any distro), macOS (Intel and Apple Silicon) and Windows (x64 and ARM).
- **Light on low-end machines:** memory is sized to the machine, and the GUI uses the CPU renderer and a static background at 4 GB of RAM or less.
- **License:** GPL-3.0-or-later. Third-party pieces keep their licenses (fonts: OFL and public domain; authlib-injector: AGPL-3.0, downloaded at runtime, never bundled).

## Done

### Playing

| Area | What works | Verified |
|---|---|---|
| Launching | Version manifest, libraries, natives, assets, Mojang Java runtimes (x86-64 under emulation on ARM), launch arguments, offline UUIDs | 1.8.9, 1.12.2, 1.16.5, 1.20.1, 1.21.1, 26.3 on Linux; also played on Windows |
| Windows command lines | Long launches (big modpacks) use a Java argument file (Java 9+) or move the classpath to `CLASSPATH` (Java 8), avoiding the 32,767-character limit | NeoForge 1.21.1 and Forge 1.12.2 through both paths |
| Memory | Heap sized from total and free RAM, G1 flags, low-end `options.txt`, game stopped with the launcher | Freezes from stacked game processes gone |
| Crashes | Crash reports, JVM `hs_err` logs and output tail turned into a cause and a fix; Java version checked before launch; missing-mod hints | Real OOM, bad heap and wrong-Java crashes |
| Window | 1280×720 by default, presets, custom size, fullscreen | Arguments checked for every version style |
| Skins | Skins and capes per account, served by a local skin service through authlib-injector (pinned version, SHA-256 checked); one shared signing key so every netherNode trusts the others' skins; fixed port 47811 so texture URLs work on every PC; `hasJoined` lets LAN worlds and servers log offline players in with their skin | Signed profile loaded; skin downloaded in a world; Forge server join with skins |

### Mods

| Area | What works | Verified |
|---|---|---|
| Loaders | Fabric, Quilt, Forge (also pre-1.13), NeoForge; OptiFine from a jar or the one-click open-source alternative | All launched except real OptiFine |
| Mods | Modrinth search and install with dependencies, enable/disable/remove, add jars by picker or drag & drop, identify jars by hash | Fabric + Sodium + Iris + Mod Menu |
| Missing dependencies | Before a modded launch, every jar's metadata (`fabric.mod.json`, `quilt.mod.json`, `mods.toml`, `neoforge.mods.toml`, bundled jars) is read; required mods that are missing are installed from Modrinth, and a candidate stays only if it really provides the id. `nethernode mods check` | Sodium Extra alone → Sodium installed; no false alarms on Fabulously Optimized, Forge and NeoForge instances |
| Modpacks | `.mrpack` import (picker or drop) and one-click Modrinth install; hidden while building, cleaned up if interrupted | Fabulously Optimized 26.2 and 1.21.1 |

### Together

| Area | What works | Verified |
|---|---|---|
| Invite friends / Join friend | iroh tunnel (QUIC, TLS 1.3, hole punching, relay fallback over HTTPS); four-word codes resolved through iroh's lookup service, full codes still work; host approves each new player; LAN-list announcement on the joiner's side | Direct ≈0 ms, relay-only 162 ms round trip |
| LAN world detection | Port read from the log in every version's wording, or found from Minecraft's own LAN announcement | Unit tests for each wording; announcement test |
| Tunnel v3 | Several shares at once, each with its own code; extra TCP ports; UDP over QUIC datagrams (game port and mod ports, e.g. Simple Voice Chat, found automatically); the host's mod list | Echo servers through the tunnel |
| Mod sync | Joiners of a modded world or server get *Get the same mods*: loader plus every mod, from Modrinth or from the host over the tunnel, SHA-1 checked | 6 mods, one not on Modrinth |
| Friends | Lasting identity per install (`identity.key`); friends add each other with a four-word friend code; presence every 20 s over ALPN `nethernode/friends/1`; friends see what each other shares and join with one click, let in without the Allow prompt | Two data folders on one PC |
| Skins between friends | Joiners send their skins, the host sends everyone's and passes on new ones | Two data folders on one PC |

### Servers on this PC

| Area | What works | Verified |
|---|---|---|
| Kinds | Vanilla, Fabric, Paper, Forge and NeoForge (official installer with `--installServer`, finished by netherNode's processor runner when a checksum is too strict, as on Fedora's zlib-ng) | Each kind started; Forge 1.20.1 joined by a real client |
| From an instance | Same version, loader and mods; client-only mods left out (Modrinth `server_side: unsupported`, `fabric.mod.json` `environment: client`); configs copied | Forge 1.20.1 instance → server |
| Running | Several at once on their own ports; console with commands; graceful stop; desktop notification when ready | Each server alone; two at once not yet tested |
| Settings | MOTD, players, game mode, difficulty, PvP, whitelist (offline UUIDs), memory, skins, extra ports, and every other `server.properties` entry | Written to `server.properties` (unit tests) |
| Multiplayer list | Each server is added to `servers.dat` of every instance with the same version (and loader), as "*name* (this PC)" | Shown in the game's Multiplayer screen |
| Skins | Online mode against the local skin service when skins are on (players need netherNode); offline mode when off | Forge server join with skins |

### App

| Area | What works |
|---|---|
| GUI | Obsidian/violet glass over a live portal shader, Minecraft-style font, instance cards with block icons from the game (netherNode's own pixel art before any download), play bar, Home/Servers/Friends/Join friend/Modpacks/Accounts/Settings |
| Instances | Create, edit, rename (unique, case-insensitive), delete; long names clipped so buttons stay on screen |
| Settings | Data, instances and servers folders (moved safely, also on Windows), renderer, animated background, skins |
| Packaging | Windows zip, macOS universal `.dmg`/`.zip`, Linux tar.gz with installer; release notes and license in every package; CI on Linux per push and the full matrix on demand |

## What's next

### 1. High priority

1. **Windows mod-specific failures.** Some mods work on Linux but not Windows (reported with a Call of Duty mod). Needs the crash report to find the cause.
2. **Real-world tests between two PCs:** voice chat through the tunnel, joining from the friends list, *Get the same mods* with a large pack over the relay.
3. **Mod updates.** "Updates available" on the Mods screen: newest compatible version per mod, update one or all, keep enabled/disabled state. The same for server mods.
4. **Self-updater.** Check GitHub Releases, verify a signature, replace the binary.
5. **Code signing.** Windows: SignPath (free, needs a public repo), Certum open-source certificate or an OV certificate; or the Microsoft Store (free registration, signs MSIX). macOS: Apple Developer Program for signing and notarization.
6. **Installers:** Windows installer (MSI/NSIS or MSIX for the Store), Linux AppImage/Flatpak.

### 2. Multiplayer

1. Self-hosted relay option (for example a small server in India) to cut relay latency; show "direct" vs "relay" in the banner.
2. Quilt servers (Quilt instances currently can't make a server).
3. Friends: groups, nicknames, "invite to my world" notifications, sharing a server with friends automatically when it starts.
4. An "AuthMe" helper for Paper servers that let in players without netherNode.
5. Plugin browser for Paper (Modrinth plugins) and a mods browser for modded servers.
6. Automatic world backups for servers and instances.

### 3. Instance management

1. Duplicate, and change the Minecraft version of an instance, with a warning about mods.
2. Export an instance as `.mrpack`.
3. Modpack updates that keep worlds and options.
4. Import from Prism/MultiMC, the official launcher and TLauncher.
5. Custom icons, search/filter and groups.
6. Worlds tab: list, back up, restore.

### 4. More from Modrinth

1. Resource packs and shader packs into `resourcepacks/` and `shaderpacks/`, with an "Iris needed" hint.
2. Data packs per world.
3. Mod details panel: icon, gallery, changelog, version picker.

### 5. Polish

1. Logs viewer with search; crash report list.
2. Minecraft news panel.
3. Keyboard navigation, UI scaling, high contrast.
4. Translations, starting with English and Hindi.
5. Remember window size and position, and the selected instance.

### 6. Platform gaps

1. Linux ARM64 before 1.19: replacement LWJGL natives.
2. Optional download mirrors for regions where Mojang's servers are slow.
3. A fully offline test: launch downloaded versions with no internet.

## Known limitations

- iced has no backdrop blur, so the glass effect is a translucent tint.
- The Mods screen needs a mod loader. Vanilla and OptiFine instances can't use it.
- With skins on, servers run in online mode against the local skin service, so players need netherNode (with skins on) to join. Turning skins off for a server lets any offline launcher in.
- Friends need netherNode open (and internet) to show online.
- A required mod that isn't on Modrinth for that version can't be installed automatically.
- Players on different netherNode versions may not be able to play together when the tunnel protocol changes (1.2 can't connect to 1.1).
