**netherNode 1.2**: offline Minecraft launcher for Windows, macOS and Linux. Builds aren't code-signed yet; see "Installing" below.

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

## New in 1.2

- **Friends list:** add friends once with a four-word friend code, see who's online and what they're sharing, and join with one click (no codes or Allow prompts).
- **Modded multiplayer:** joining a modded world or server offers *Get the same mods* (same loader and mods, from Modrinth or straight from the host).
- **Servers:** Forge and NeoForge servers; make a server from an instance (client-only mods are left out); run several at once, each with its own code; every `server.properties` setting; skins on servers.
- **Your servers are in Minecraft's Multiplayer list** ("*name* (this PC)"), replacing *Play on it*.
- **Voice chat and other mod ports:** UDP and extra ports go through the tunnel; Simple Voice Chat is found automatically.
- **Missing mod dependencies:** before a modded game starts, netherNode reads what each mod needs and installs anything missing from Modrinth (also for jars you added yourself).
- **Windows:** big modpacks no longer fail with "The filename or extension is too long".
- **Instances:** rename in Edit (names stay unique); long names no longer push buttons off screen.
- A **Home** button, and the instance's version now sits beside its dropdown.
- Everyone playing together needs 1.2: it can't connect to 1.1.

## Fixed in 1.1.2

- Instance icons appear as soon as a game starts (they used to show only after closing it), and come from any downloaded version, so old versions no longer leave letter circles.
- Before any game is downloaded, instances get netherNode's own pixel-art icons.

## Fixed in 1.1.1

- *Invite friends* on an instance now works after **Open to LAN** in every Minecraft version and game language. netherNode finds the world from the game's own LAN announcement when the log doesn't say which port it uses.
- The download now includes the license (GPL-3.0).

## New in 1.1

- **Custom skins and capes** for offline accounts (Accounts → Choose skin…, classic or slim arms). You and friends who join through netherNode see each other's skins in game. Uses authlib-injector, an open-source helper downloaded once from its official site; turn it off in Settings.
- **Four-word invite codes** like `maple-orbit-tunnel-frog`, easy to read out loud. The long code still works (*Copy full code*).
- *Invite friends* is always on the instance; server settings scroll, so every option fits.
- The interface uses a Minecraft-style font.
- Everyone playing together needs 1.1: it can't connect to 1.0.

## What's in it

- Offline accounts only: no Microsoft or Mojang login.
- Every Minecraft version, with the matching Java downloaded automatically.
- Fabric, Quilt, Forge and NeoForge; loaders not yet released for a version are greyed out.
- OptiFine: a one-click open-source alternative, or a real OptiFine jar.
- Import worlds (folders or .zip), and move the game files or instances folder in Settings.
- Play with friends over the internet: Invite friends / Join friend, no port forwarding.
- Host a server on your PC (Vanilla, Fabric or Paper), shareable with the same Invite friends.
- Desktop notifications for join requests and server status.
- Mods and modpacks from Modrinth, `.mrpack` import, drag & drop.
- Memory sized to your computer, crash detection with fixes, and a 1280×720 game window by default.

## Please check when testing

- Friends: add each other with friend codes; check online status and *Join* on something the other shares.
- Join a modded world: *Get the same mods*, then *Play it*.
- A Forge or NeoForge server made from an instance; two servers running at once with Invite friends on both.
- Simple Voice Chat between two PCs through Invite friends.
- A big modpack on Windows (the one that failed before).
- Skins: choose one under Accounts, play, and check it shows in third person (F5) — and that a friend who joins your LAN world sees it.
- Invite friends with the four words: the friend types them in Join friend.
- Windows: all text is readable and nothing turns dark.
- The nether portal background animates (Settings → Launcher graphics).
- An OptiFine instance (one-click alternative) launches.
- Importing a world, and moving the instances folder in Settings.
- Invite friends: open a world to LAN, invite, and join from another PC or network with the code.
- Servers: create one, accept the EULA, Start, Invite friends, and join with "Play on it".
- A join request shows a desktop notification and flashes the taskbar.
- The app starts, and an account and an instance can be created.
- A vanilla version (e.g. 1.21.1) and an older one (e.g. 1.12.2) launch.
- A Fabric instance with Sodium launches.
- Closing the launcher while a game runs stops the game.
- On Apple Silicon: 1.16.5 or older launches (it runs Java under Rosetta 2).
- On Windows: no extra black console windows appear.
