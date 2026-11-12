# Changelog

Every netherNode release. Downloads are on the [Releases page](https://github.com/TechPsychotic/netherNode/releases).

## 1.2.3 (2026-11-05)

- **Joining a friend's server:** *Play it*, Direct Connect and the copied address use `127.0.0.1` instead of `localhost`, so the game can't pick an address the tunnel isn't on ("Connection refused: getsockopt").
- **Same address every time:** when the host's port is free on your PC, the tunnel uses that same port number, so a saved Multiplayer entry keeps working next time.
- Joining reminds you to keep netherNode open while you play (the tunnel runs inside it).

## 1.2.2 (2026-10-26)

- **Joining friends works on Windows and Fedora:** the tunnel accepts IPv6 `localhost`, which those systems use first. Direct Connect, *Play it* and Simple Voice Chat were refused before.
- **Shared worlds show in your friend's LAN list** reliably (announced straight to their game instead of over multicast, which Windows sent out of the wrong adapter).
- **Voice chat:** large voice packets no longer get lost in the tunnel.
- **Modded servers:** missing dependencies are installed before each start; a mod that only works in the game is moved out of the server with a message if the server crashes on it; other start-up crashes (missing mods, busy port, out of memory) are explained.

## 1.2.1 (2026-10-26)

- **Server settings** are a full page: netherNode's options (memory, skins, extra ports, whitelist), then every `server.properties` setting in groups with a plain explanation and the right control (tick box, choices or a checked number). Server-specific and mod settings stay editable under *Advanced*.
- **Friends:** the screen always shows how to add friends, with a connecting/connected status; if it can't connect it says why and retries.

## 1.2.0 (2026-10-26)

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

## 1.1.2 (2026-10-22)

- Instance icons appear as soon as a game starts (they used to show only after closing it), and come from any downloaded version, so old versions no longer leave letter circles.
- Before any game is downloaded, instances get netherNode's own pixel-art icons.

## 1.1.1 (2026-10-22)

- *Invite friends* on an instance now works after **Open to LAN** in every Minecraft version and game language. netherNode finds the world from the game's own LAN announcement when the log doesn't say which port it uses.
- The download now includes the license (GPL-3.0).

## 1.1.0 (2026-10-22)

- **Custom skins and capes** for offline accounts (Accounts → Choose skin…, classic or slim arms). You and friends who join through netherNode see each other's skins in game. Uses authlib-injector, an open-source helper downloaded once from its official site; turn it off in Settings.
- **Four-word invite codes** like `maple-orbit-tunnel-frog`, easy to read out loud. The long code still works (*Copy full code*).
- *Invite friends* is always on the instance; server settings scroll, so every option fits.
- The interface uses a Minecraft-style font.
- Everyone playing together needs 1.1: it can't connect to 1.0.

## 1.0.0 (2026-10-22)

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

## 0.1.x (2026-10-21)

Test builds before the first full release.
