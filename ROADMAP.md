# netherNode Roadmap

An offline-accounts Minecraft launcher for Linux, macOS and Windows, written in Rust:
`crates/core` (library), `crates/cli` (`nethernode`) and `crates/gui` (`nethernode-gui`, iced).

## Ground rules

- **No Microsoft or Mojang login, ever.** Only offline accounts. Game files still come from Mojang's public servers.
- **No CurseForge.** Modrinth is the only source for mods and modpacks.
- **Never ship Mojang's files or art.** Game jars, assets and textures are downloaded or read at runtime.
- **Cross-platform:** every feature has to work on Linux (any distro), macOS (Intel and Apple Silicon) and Windows (x64 and ARM).
- **Light on low-end machines:** memory is sized to the machine, and the GUI uses the CPU renderer and a static background at 8 GB of RAM or less.

## Done

| Area | What works | Verified |
|---|---|---|
| Launching | Version manifest, libraries, natives, assets, Mojang Java runtimes (x86-64 under emulation on ARM), launch arguments, offline UUIDs | 1.8.9, 1.12.2, 1.16.5, 1.20.1, 26.3 on Linux |
| Memory | Heap sized from total and free RAM, G1 flags, low-end `options.txt`, game stopped with the launcher | Freezes from stacked game processes gone |
| Crashes | Crash reports, JVM `hs_err` logs and output tail turned into a cause and a fix; Java version checked before launch | Real OOM, bad heap and wrong-Java crashes |
| Loaders | Fabric, Quilt, Forge (also pre-1.13), NeoForge; OptiFine from a user-supplied jar | All except OptiFine launched |
| Mods | Modrinth search and install with dependencies, enable/disable/remove, add jars by picker or drag & drop, identify jars by hash | Fabric + Sodium + Iris + Mod Menu |
| Modpacks | `.mrpack` import (picker or drop) and one-click Modrinth install; hidden while building, cleaned up if interrupted | Fabulously Optimized 26.2 and 1.21.1 |
| GUI | Obsidian/violet glass over a live portal shader, instance cards, play bar, full version dropdown, loader picker, Mods/Modpacks/Accounts screens, crash cards | Screenshots of every screen, Linux |
| Window | 1280×720 by default (Minecraft's own is 854×480), presets, custom size, fullscreen | Arguments checked for every version style |
| Packaging | App icons (PNG/ICO/ICNS), Windows exe icon, Linux `.desktop` + installer script, macOS `.app` script, CI workflow | Cross-compiles for 6 targets; CI not yet run |

## Play with friends (done)

Invite friends / Join friend tunnel a LAN world over the internet with iroh: QUIC with TLS 1.3, NAT hole punching, and a relay fallback over HTTPS for CGNAT. The join code carries a random secret, the host approves each friend, and the joiner sees the world in Minecraft's LAN list. Verified on this machine: direct about 0 ms, relay-only 162 ms round trip (n0's public relays are in the US/EU).

Short codes: the host gets four everyday words (for example `maple-orbit-tunnel-frog`) that are easy to read out loud. The full code is published under a key derived from the words on iroh's public lookup service; the friend's app looks it up. Full codes still work.

**Next:** self-hosted relay option (for example a small server in India) to cut relay latency; show "direct" vs "relay" in the banner.

## Skins (done)

Custom skins and capes per account (Accounts screen or `nethernode account skin`). A local skin service plus authlib-injector (open source, downloaded at a pinned version and checked by SHA-256, never bundled) shows them in game, and Invite friends exchanges skins between friends. Nothing logs in anywhere. Verified: the game loads the signed profile, LAN-style logins pass through the local `hasJoined`, and a real client downloaded its custom skin in a world.

All installs sign skin data with one shared key built into the app (Minecraft only shows other players' skins it can verify, and authlib-injector trusts a single key). It's public on purpose: it only vouches for textures served from 127.0.0.1, and the skin service listens on a fixed port (47811) so texture URLs work on every PC.

**Next:** skins on netherNode-hosted servers (they run in offline mode, so players there get default skins).

## Servers on this PC (done)

Vanilla, Fabric and Paper servers with a console, graceful stop (saves the world), settings, an offline-UUID whitelist, world import, "Play on it" (quick-join) and Invite friends through the same tunnel. Offline mode, with an EULA checkbox (never accepted silently). Desktop notifications for join requests (plus a taskbar flash) and server ready. Verified: each kind started, pinged through the tunnel, a real client joined (Vanilla, Paper) and stopped cleanly.

**Next:** Forge/NeoForge servers, mods/plugins browser for servers, automatic world backups, an "AuthMe" helper for Paper, and start-on-launch.

## What's left

### 1. Before the first public release (must have)

1. **Test on real Windows and macOS machines.** Everything compiles for both but has never actually run there. Check:
   - Windows: no stray console windows, the exe icon, the console-close/stop behaviour, paths with spaces or non-English characters, the 32 KB command-line limit with big modpacks (Java `@argfiles`).
   - macOS: the `.app` opens (Gatekeeper), Apple Silicon running Java 8/16 under Rosetta, `-XstartOnFirstThread`.
2. **CI and releases.** The private GitHub repo runs Linux checks on each push and the full matrix on demand. Pushing a `v*` tag builds Windows, macOS (universal) and Linux and publishes a pre-release. Remaining: confirm the first release builds pass everywhere.
3. **Settings screen.** Global defaults: memory, window size, Java path, data folder location, animation, close-launcher-on-play. Today these are per instance or hard-coded.
4. **Mod updates.** "Updates available" on the Mods screen: compare each mod's Modrinth version with the newest compatible one and update one or all, keeping enabled/disabled state.
5. **Self-updater.** Check GitHub Releases, verify a signature, then replace the binary (or point users to their package manager).
6. **Installers and packages:**
   - Windows: installer (MSI or NSIS) plus a portable zip.
   - macOS: `.dmg`. Signing and notarization need an Apple Developer ID (paid); without it users must right-click → Open.
   - Linux: AppImage and Flatpak, with AUR and `.deb` as nice-to-haves.
7. **Test OptiFine with a real jar.** The install path follows HMCL's method but hasn't been tested.

### 2. Instance management (should have)

1. Rename, duplicate and change the Minecraft version of an instance, with a warning about mods.
2. Export an instance as `.mrpack`, the reverse of import.
3. Modpack updates: install a newer pack version over an instance and keep user changes (worlds, options).
4. Import instances from Prism/MultiMC, the official launcher (`.minecraft`) and TLauncher.
5. Custom instance icons, search/filter and groups once lists get long.
6. Worlds tab: list saves, back up and restore, open folder.

### 3. More content from Modrinth (should have)

1. Resource packs and shader packs: search and install into `resourcepacks/` and `shaderpacks/`, with an "Iris needed" hint for shaders.
2. Data packs per world.
3. Mod details panel: icon, gallery, changelog, versions to choose from.

### 4. Polish (nice to have)

1. Logs viewer: `latest.log`, past logs and the crash report list, with search.
2. Minecraft news panel from Mojang's public launcher news feed.
3. Keyboard navigation, UI scaling, high-contrast option.
4. Translations, starting with English and Hindi.
5. Remember window size and position, and the selected instance.
6. Quick join: saved servers per instance (quick play for 1.20+, `--server` for older).

### 5. Platform gaps (nice to have)

1. Linux ARM64: Mojang has no ARM natives before 1.19. This needs replacement LWJGL natives, as Prism does.
2. Download mirrors for regions where Mojang's servers are slow (e.g. BMCLAPI), as an opt-in setting.
3. A fully offline mode test: launch already-downloaded versions with no internet at all.

## Open decisions

- **Skins for offline players.** Without Mojang auth, other players see default skins. Options:
  - (a) Do nothing.
  - (b) Bundle a client-side skin mod, such as CustomSkinLoader, into loader instances.
  - (c) Support authlib-injector with a third-party skin server (Ely.by, self-hosted Drasl). This isn't Microsoft or Mojang, but it is still a login, so it needs your decision.
- **Licence** for the project itself, before publishing (MIT or GPL-3.0 are typical for launchers).
- **Name and branding check.** Make sure "netherNode" and the icon don't clash with existing projects or Mojang's brand guidelines.

## Known limitations (today)

- iced has no backdrop blur, so the glass effect is a translucent tint.
- The Mods screen needs a mod loader. Vanilla and OptiFine instances can't use it.
- Minecraft textures in the UI only appear after the first game version has downloaded. Until then the UI uses its own pixel art.
- The first launch of a large modpack on low-end hardware can take a few minutes. That's the game loading, not the launcher.
