# Contributing to netherNode

Thanks for helping. Bug reports, testing on your platform, fixes and features are all welcome.

## Ground rules

These are permanent design decisions; changes that go against them won't be merged:

- **Offline accounts only.** No Microsoft or Mojang login.
- **Modrinth only** for mods and modpacks (no CurseForge).
- **Never ship Mojang's files or art.** Game files and textures are downloaded or read at runtime.
- **Every feature works on Linux, macOS and Windows** (x64 and ARM).
- **Light on low-end machines.**

See [ROADMAP.md](ROADMAP.md) for what's planned.

## Building

Install [Rust](https://rustup.rs) (stable), then:

```sh
cargo build --release
target/release/nethernode-gui     # the app
target/release/nethernode         # the CLI
```

Set `NETHERNODE_HOME` to a scratch folder while developing so you don't touch your real instances.

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI runs clippy and the tests on Linux, Windows and macOS for every pull request.

- Keep pull requests focused: one fix or feature each.
- User-facing text is short and plain; say what happened and what to do.
- Add a line to [CHANGELOG.md](CHANGELOG.md) under an *Unreleased* heading for anything users will notice.

## Reporting bugs

Use the [bug report form](https://github.com/TechPsychotic/netherNode/issues/new/choose). The most useful things to include are your OS, the netherNode version, the Minecraft version and loader, and the log: open the instance's **Folder** and attach `logs/latest.log` (and anything in `crash-reports/`).

Security problems go through [SECURITY.md](SECURITY.md), not public issues.

## Manual test checklist

Before a release, check what you can on each platform:

- [ ] Friends: add each other with friend codes; check online status and *Join* on something the other shares.
- [ ] Join a modded world: *Get the same mods*, then *Play it*.
- [ ] A Forge or NeoForge server made from an instance; two servers running at once with Invite friends on both.
- [ ] Simple Voice Chat between two PCs through Invite friends.
- [ ] A big modpack on Windows (the one that failed before).
- [ ] Skins: choose one under Accounts, play, and check it shows in third person (F5) — and that a friend who joins your LAN world sees it.
- [ ] Invite friends with the four words: the friend types them in Join friend.
- [ ] Windows: all text is readable and nothing turns dark.
- [ ] The nether portal background animates (Settings → Launcher graphics).
- [ ] An OptiFine instance (one-click alternative) launches.
- [ ] Importing a world, and moving the instances folder in Settings.
- [ ] Invite friends: open a world to LAN, invite, and join from another PC or network with the code.
- [ ] Servers: create one, accept the EULA, Start, Invite friends, and join it from Minecraft's Multiplayer list ("*name* (this PC)").
- [ ] A join request shows a desktop notification and flashes the taskbar.
- [ ] The app starts, and an account and an instance can be created.
- [ ] A vanilla version (e.g. 1.21.1) and an older one (e.g. 1.12.2) launch.
- [ ] A Fabric instance with Sodium launches.
- [ ] Closing the launcher while a game runs stops the game.
- [ ] On Apple Silicon: 1.16.5 or older launches (it runs Java under Rosetta 2).
- [ ] On Windows: no extra black console windows appear.

## Website

The site in `site/` is published to GitHub Pages by `.github/workflows/pages.yml`. Preview it with:

```sh
site/build.sh && python3 -m http.server -d _site 8000
```

## License

By contributing you agree that your contributions are licensed under the [GPL-3.0-or-later](LICENSE), like the rest of netherNode.
