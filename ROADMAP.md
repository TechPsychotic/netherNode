# netherNode: Offline Minecraft Launcher Roadmap

A multi-instance Minecraft launcher with offline accounts, in the spirit of UltimMC (a C++/Qt5 fork of MultiMC).
The launcher never ships Mojang files. Everything is downloaded at runtime from Mojang's own endpoints, as the official launcher does.

---

## 0. Pick a stack (decide on day 1)

| Option | Good for | Notes |
|---|---|---|
| **Rust + Tauri** (UI in Svelte/React) | Small binaries, fast, cross-platform | `piston-mc` crate is a reference. LiquidLauncher is built this way. |
| **Kotlin/Java + JavaFX or Compose** | Same ecosystem as the game | Easy to reuse `UUID.nameUUIDFromBytes`. JVM startup overhead. |
| **C++ + Qt6** | Matches UltimMC/MultiMC/Prism | Most work, but the most code to borrow from. |
| **Python + minecraft-launcher-lib** | Prototype in a weekend | Good for learning the pipeline before you rewrite it. |

**Decision:** Rust. `crates/core` is the launcher library and `crates/cli` the `nethernode` binary. A Tauri GUI will sit on top of `core`.

**Status (2026-10-21):** Phases 1–3 are done. All five test versions (1.8.9, 1.12.2, 1.16.5, 1.20.1, 26.3) launch on Linux x64.

**Platforms:** Linux (any distro), macOS (Intel + Apple Silicon) and Windows (x64 + ARM64).
- The code compile-checks clean for all six targets. CI (`.github/workflows/ci.yml`) builds and tests natively on each OS. Actually playing the game on macOS and Windows still needs a manual test on real hardware.
- On ARM machines, versions whose Java (8 or 16) has no ARM build use Mojang's x86-64 runtime under Rosetta 2 or Windows emulation, with matching x86-64 natives.
- Linux: Minecraft ≤1.12.2 (LWJGL 2) needs the `xrandr` program; the launcher checks for it and prints the install command for each distro.
- Linux ARM64: Mojang publishes no runtimes or older natives for it, so it uses the system Java, and versions before 1.19 lack ARM natives. Fixing this needs LWJGL replacement natives, as Prism does.
- Windows: there is a 32 KB command-line limit. Big modded classpaths may need Java `@argfiles` later.

---

## Phase 1: Core launch pipeline (CLI only, no GUI)

Goal: `nethernode launch 1.21.1 --user Steve` starts the game.

1. **Version manifest**
   - GET `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json`
   - It lists `latest.release`, `latest.snapshot` and `versions[]` (`id`, `type`, `url`, `sha1`).
   - Cache it and refresh it on startup.
2. **Version JSON**
   - Fetch `versions[i].url` and check its `sha1`.
   - Important keys: `downloads.client`, `libraries`, `assetIndex`, `arguments` (1.13+) or `minecraftArguments` (pre-1.13), `mainClass`, `javaVersion`, `logging`, `inheritsFrom` (used by mod loaders).
3. **Rule evaluation**
   - Libraries and arguments carry `rules[]` with `action: allow|disallow`, an `os` (`name`, `arch`, `version` regex) and `features` (`is_demo_user`, `has_custom_resolution`, `is_quick_play_*`...).
   - Write a single `evaluate_rules()` and test it well. Most "game won't start" bugs come from here.
4. **Libraries**
   - For each allowed library, download `downloads.artifact` to `libraries/<path>` and verify its SHA1.
   - Old-style natives (≤1.18): `natives.{linux,windows,osx}` + `classifiers`. Extract them to `natives/<version>/` and honour `extract.exclude` (META-INF).
   - New-style natives (1.19+): native libraries are normal entries with a `:natives-linux` style name and OS rules. LWJGL extracts them itself, but still set `-Djava.library.path`.
   - Library `name` follows Maven coordinates: `group:artifact:version[:classifier]` → `group/path/artifact/version/artifact-version[-classifier].jar`. You need this for mod loader libraries that only have `name` + `url`.
5. **Client jar**: `downloads.client.url` → `versions/<id>/<id>.jar`.
6. **Assets**
   - Fetch `assetIndex.url` → `assets/indexes/<id>.json`.
   - Each object is stored at `https://resources.download.minecraft.net/<hash[0:2]>/<hash>` → `assets/objects/<hash[0:2]>/<hash>`.
   - Legacy indexes (`legacy`, `pre-1.6`) with `virtual`/`map_to_resources` need copying into `assets/virtual/legacy` or `resources/`.
7. **Java runtime**
   - Read `javaVersion.component` (`jre-legacy`, `java-runtime-alpha`, `-gamma`, `-delta`...) and `majorVersion`.
   - Either detect a system JVM of the right major version, or download Mojang's runtime from its `all.json` runtime manifest (look up the current URL in the official launcher/piston-meta. The hash path has changed over time).
   - Map OS/arch to keys like `linux`, `windows-x64`, `mac-os-arm64`.
8. **Build the command line**
   - Classpath = all allowed library jars + client jar, separated by `:` (Linux/macOS) or `;` (Windows).
   - Substitute placeholders in `arguments.jvm` and `arguments.game`:
     `${auth_player_name}`, `${auth_uuid}`, `${auth_access_token}`, `${user_type}`, `${version_name}`, `${game_directory}`, `${assets_root}`, `${assets_index_name}`, `${natives_directory}`, `${classpath}`, `${launcher_name}`, `${launcher_version}`, `${version_type}`, `${clientid}`, `${auth_xuid}`, `${resolution_width/height}`, `${quickPlay*}`.
   - Add `-Xmx/-Xms` and the `logging.client.argument` (log4j config) if present.
   - Run `java <jvm args> <mainClass> <game args>`, with cwd = the instance's game directory.
9. **Downloader**: parallel (8–16 workers), resumable, SHA1-verified and retried. Skip files whose hash already matches. This makes later launches instant.

**Milestone:** launches 1.8.9, 1.12.2, 1.16.5, 1.20.x and latest on your OS. These cover the legacy args, old natives, new natives and Java 8/17/21 cases.

---

## Phase 2: Offline accounts

Offline is just "launch with a made-up identity". There is no network auth.

- **UUID:** `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`, which is MD5 forced to version 3. Non-Java languages must reproduce it exactly (MD5 → set `byte[6] = (b & 0x0f) | 0x30`, `byte[8] = (b & 0x3f) | 0x80`). With this, offline servers and LAN worlds give the same player the same UUID. Renaming changes the UUID, so warn users about that.
- **Placeholders:** `auth_player_name=<name>`, `auth_uuid=<uuid without dashes>`, `auth_access_token=0` (any dummy value), `user_type=legacy` (or `msa` on newer versions if a value is required), `auth_xuid=0`, `clientid=0`.
- **Validation:** 3–16 characters, `[A-Za-z0-9_]`.
- **Storage:** `accounts.json` with `{type: "offline", name, uuid}`.
- **Low-memory handling (done):** heap size is automatic (¼ of RAM, 768 MB–3 GB) and shrinks to the memory that is actually free at launch. Mojang's ZGC/AlwaysPreTouch flags are replaced with G1. Machines with 8 GB or less get a lighter `options.txt`. The game is killed when the launcher exits, and an instance can't run twice.
- **Known quirks:** multiplayer to online-mode servers won't work (by design). Some newer versions show a "chat disabled / profile keys" warning. Singleplayer and LAN are fine.

**No Microsoft or Mojang login, ever.** This is a hard project rule. Game files are still downloaded from Mojang's public servers, but no account is ever checked.

**Undecided:**
- **authlib-injector / Yggdrasil (Ely.by, Drasl, LittleSkin):** third-party, not Microsoft/Mojang; gives offline-style players real skins. Add the JVM arg `-javaagent:authlib-injector.jar=<api-root>`, authenticate against that server's `/authserver/authenticate` and use the returned token and UUID. Drasl is self-hostable if you want your own skin server.

---

## Phase 3: Instances

Instances are what make it MultiMC-like.

```
data/
  meta/            cached manifests, version JSONs
  libraries/       shared across instances
  assets/          shared
  runtimes/        downloaded JREs
  instances/<name>/
    instance.json  { mcVersion, loader, loaderVersion, javaPath, memory, jvmArgs, icon }
    .minecraft/    game dir: saves, mods, resourcepacks, config, options.txt
  accounts.json
  settings.json
```

- Create, clone, rename, delete, import and export (zip) instances.
- Per-instance overrides for Java, memory, resolution and JVM args.
- Sharing `libraries/` and `assets/` saves disk space.

---

## Phase 4: Mod loaders

Every loader produces a version JSON that `inheritsFrom` the vanilla one. Write a **merge function**: take the parent and append child libraries (child wins on the same `group:artifact`), child `mainClass`, concatenate `arguments`.

- **Fabric:** GET `https://meta.fabricmc.net/v2/versions/loader/<mc>/<loader>/profile/json`. Libraries use `name` + `url` (Maven repo), not `downloads`, so resolve them yourself.
- **Quilt:** same idea at `meta.quiltmc.org/v3/...`.
- **NeoForge / Forge (1.13+):** run the official installer jar headlessly (`--installClient <dir>`) into a temp `.minecraft`, or reimplement its `install_profile.json` processors (the hard way). Prism Launcher's meta repo shows how to pre-process these.
- **Legacy Forge (≤1.12):** a version JSON plus libraries and the old `minecraftArguments` style.

---

## Phase 5: GUI

- **Main window:** instance grid with icons, Play button, account switcher, news/log panel.
- **Dialogs:** New Instance (version list filtered by release/snapshot/old, loader picker), Account Manager, Settings, Instance Settings.
- **Live game log console** (stream stdout/stderr) and a **crash detector** that looks for `crash-reports/` and a non-zero exit code.
- **Progress UI** for downloads, with bytes and file counts.

---

## Phase 6: Mod and modpack content

- **Modrinth API** (`api.modrinth.com/v2`): search, versions by game version + loader, dependencies, and the `.mrpack` modpack format (`modrinth.index.json` + overrides). It is open and easy, so start here.
- **CurseForge:** needs an API key (apply for one at console.curseforge.com).
- Mod list per instance with enable/disable (rename to `.jar.disabled`) and update checking.

---

## Phase 7: Polish and distribution

- Self-updater (GitHub Releases + signature check).
- Java auto-detection on Windows (registry), Linux (`/usr/lib/jvm`) and macOS (`/Library/Java`).
- Packaging: AppImage/Flatpak/.deb/AUR, Windows MSI/portable zip, macOS .dmg (notarization costs money).
- CI: GitHub Actions matrix build for Linux, Windows and macOS.
- Portable mode (data dir next to the binary).
- Import from Prism/MultiMC/the official launcher.

---

## Suggested timeline (solo, part-time)

| Weeks | Deliverable |
|---|---|
| 1–2 | Phase 1 CLI launches vanilla across the 5 test versions |
| 3 | Phase 2 offline accounts, Phase 3 instances on disk |
| 4–5 | Phase 4 Fabric + Quilt, then NeoForge |
| 6–8 | Phase 5 GUI |
| 9+ | Phase 6 Modrinth, Phase 7 packaging/updater |

---

## Reference projects to read

- UltimMC: https://github.com/UltimMC/Launcher (C++/Qt, MultiMC fork with "Add Local" offline accounts)
- Prism Launcher: https://github.com/PrismLauncher/PrismLauncher (the best-maintained MultiMC descendant, with clean `minecraft/` launch code)
- HMCL: https://github.com/HMCL-dev/HMCL (Java, offline + authlib-injector)
- LiquidLauncher: Rust/Tauri launcher core
- minecraft-launcher-lib (Python): https://minecraft-launcher-lib.readthedocs.io
- piston-mc (Rust crate): https://docs.rs/piston-mc
- wiki.vg mirror, *Game files* and *Launching the game*: https://wikivg.booky.dev/Game_files
- Minecraft Wiki, version_manifest.json: https://minecraft.fandom.com/wiki/Version_manifest.json
- Fabric Meta: https://github.com/FabricMC/fabric-meta
- Drasl (self-hosted Yggdrasil / authlib-injector server): https://git.plan9.rocks/unmojang/drasl

## Legal note

Offline mode is a built-in Minecraft feature. Keep the launcher clean by never redistributing Mojang jars or assets (download them from Mojang at runtime) and by not bypassing anything on online-mode servers. netherNode is offline-only by design and never asks for a Microsoft login.
