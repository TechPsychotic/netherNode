//! Everything the window shows: top bar, instance grid, details and console, the play bar,
//! and the forms and Mods screen that replace the grid when open.

use iced::widget::image::FilterMethod;
use iced::widget::{
    button, checkbox, column, container, image, mouse_area, pick_list, progress_bar, row,
    scrollable, shader, space, stack, text, text_input,
};
use iced::{Alignment, Color, Element, Font, Length};
use nethernode_core::crash::CrashInfo;
use nethernode_core::instance::Instance;
use nethernode_core::loader::LoaderKind;

use crate::background::Portal;
use crate::theme;
use crate::{
    App, EditForm, FolderKind, FriendsView, Load, LoaderPick, Message, ModsTab, ModsView,
    NewInstanceForm, NewServerForm, PacksView, Screen, ServerField, ServersView, SettingsForm,
    VersionFilter,
};

/// Obsidian block size in the background, logical px. Content sits inside this frame.
const BLOCK: f32 = 40.0;
/// Lines actually laid out in the console; laying out thousands of lines every frame is slow.
const LOG_VISIBLE: usize = 400;

/// Bundled fonts (see `fonts` in main.rs), so text never depends on what the OS has.
pub const UI_FONT: Font = Font::with_name("Minecraft");
const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..UI_FONT
};
const MONO: Font = Font::with_name("JetBrains Mono");

fn loader_color(kind: Option<LoaderKind>) -> Color {
    match kind {
        None => theme::SUCCESS,
        Some(LoaderKind::Fabric) => Color::from_rgb8(0xe8, 0xc9, 0x9b),
        Some(LoaderKind::Quilt) => Color::from_rgb8(0xc0, 0x96, 0xff),
        Some(LoaderKind::Forge) => Color::from_rgb8(0xf0, 0x94, 0x5c),
        Some(LoaderKind::NeoForge) => Color::from_rgb8(0xff, 0xae, 0x6b),
        Some(LoaderKind::OptiFine) => Color::from_rgb8(0xff, 0x7a, 0x8a),
    }
}

/// Shorten a long name to `max` characters with an ellipsis, so it can't push buttons
/// off screen.
pub(crate) fn clip(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_string()
    } else {
        let mut short: String = name.chars().take(max.saturating_sub(1)).collect();
        short.push('…');
        short
    }
}

fn loader_label(instance: &Instance) -> String {
    match &instance.loader {
        None => "Vanilla".into(),
        Some(l) if l.kind == LoaderKind::OptiFine => "OptiFine".into(),
        Some(l) => format!("{} {}", l.kind, l.version),
    }
}

fn chip<'a>(label: impl text::IntoFragment<'a>, color: Color) -> Element<'a, Message> {
    container(text(label).size(12))
        .padding([3, 10])
        .style(theme::chip(color))
        .into()
}

fn small_button<'a>(label: impl text::IntoFragment<'a>) -> button::Button<'a, Message> {
    button(text(label).size(14))
        .padding([8, 14])
        .style(theme::glass_button)
}

fn field<'a>(label: &'a str, input: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![text(label).size(13).color(theme::MUTED), input.into()]
        .spacing(6)
        .into()
}

fn short_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        let body = match &self.screen {
            Screen::Home => row![self.instances_panel(), self.details_panel()]
                .spacing(16)
                .height(Length::Fill)
                .into(),
            Screen::NewInstance(form) => self.dialog(self.new_instance_form(form)),
            Screen::EditInstance(form) => self.dialog(self.edit_form(form)),
            Screen::Accounts { name } => self.dialog(self.accounts_form(name)),
            Screen::Mods(view) => self.mods_screen(view),
            Screen::Modpacks(view) => self.modpacks_screen(view),
            Screen::Settings(form) => self.dialog(self.settings_form(form)),
            Screen::Servers(view) => self.servers_screen(view),
            Screen::NewServer(form) => self.dialog(self.new_server_form(form)),
            Screen::JoinFriend { code, error } => {
                self.dialog(join_friend_form(code, error.as_deref()))
            }
            Screen::Friends(view) => self.dialog(self.friends_screen(view)),
            Screen::ServerSettings(form) => self.dialog(server_settings(
                form,
                self.servers.iter().find(|s| s.name == form.server),
                self.server_runs.contains_key(&form.server),
            )),
        };
        let mut content = column![self.top_bar()].spacing(14);
        if let Some((is_error, msg)) = &self.notice {
            content = content.push(
                container(
                    row![
                        text(msg).width(Length::Fill),
                        button(text("×").size(16))
                            .style(theme::link)
                            .on_press(Message::DismissNotice),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding([10, 16])
                .style(theme::notice(*is_error)),
            );
        }
        content = content
            .extend(self.share_banners())
            .push(body)
            .push(self.play_bar());

        if self.gpu {
            let portal = Portal {
                textures: self.textures.clone(),
                animate: self.animating(),
                block: BLOCK,
            };
            let inside = container(content)
                .padding(BLOCK + 12.0)
                .width(Length::Fill)
                .height(Length::Fill);
            // The gradient underneath shows if the GPU path ever falls back to the CPU.
            let backdrop = container(space())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::static_backdrop);
            stack![
                backdrop,
                shader(portal).width(Length::Fill).height(Length::Fill),
                inside
            ]
            .into()
        } else {
            container(content)
                .padding(18)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::static_backdrop)
                .into()
        }
    }

    fn top_bar(&self) -> Element<'_, Message> {
        let gb = self.memory.total_mb as f64 / 1024.0;
        let mut right = row![chip(format!("{gb:.0} GB RAM"), theme::VIOLET_BRIGHT)]
            .spacing(8)
            .align_y(Alignment::Center);
        right = right
            .push(small_button("Home").on_press(Message::ShowHome))
            .push(small_button("Servers").on_press(Message::ShowServers))
            .push(small_button(self.friends_label()).on_press(Message::ShowFriends))
            .push(small_button("Join friend").on_press(Message::ShowJoinFriend))
            .push(small_button("Modpacks").on_press(Message::ShowModpacks))
            .push(small_button("Accounts").on_press(Message::ShowAccounts))
            .push(small_button("Settings").on_press(Message::ShowSettings));
        container(
            row![
                image(self.logo.clone())
                    .width(36)
                    .height(36)
                    .filter_method(FilterMethod::Nearest),
                column![
                    text("netherNode")
                        .size(22)
                        .font(BOLD)
                        .color(theme::VIOLET_BRIGHT),
                    text("just another minecraft launcher")
                        .size(12)
                        .color(theme::MUTED),
                ]
                .spacing(0),
                space::horizontal(),
                right,
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([10, 16])
        .style(theme::glass)
        .into()
    }

    fn instance_icon(&self, instance: &Instance, size: f32) -> Element<'_, Message> {
        let name = crate::textures::icon_name(instance.loader.as_ref().map(|l| l.kind));
        match self.textures.as_ref().and_then(|t| t.icons.get(name)) {
            Some(handle) => image(handle.clone())
                .width(size)
                .height(size)
                .filter_method(FilterMethod::Nearest)
                .into(),
            None => image(crate::textures::fallback_icon(name))
                .width(size)
                .height(size)
                .filter_method(FilterMethod::Nearest)
                .into(),
        }
    }

    fn instances_panel(&self) -> Element<'_, Message> {
        let mut cards = row![].spacing(12);
        // Installs in progress come first so they are visible without scrolling.
        for job in self.pack_jobs.values() {
            let (detail, done, total) = match &job.stage {
                Some((stage, done, total)) => (stage.clone(), *done, *total),
                None => ("Starting…".into(), 0, 1),
            };
            cards = cards.push(
                container(
                    column![
                        text("Installing").size(11).color(theme::WARNING),
                        text(&job.label)
                            .size(14)
                            .font(BOLD)
                            .width(Length::Fill)
                            .center(),
                        text(detail)
                            .size(11)
                            .color(theme::MUTED)
                            .width(Length::Fill)
                            .center(),
                        progress_bar(0.0..=total.max(1) as f32, done as f32)
                            .girth(6)
                            .style(theme::progress),
                    ]
                    .spacing(8)
                    .align_x(Alignment::Center),
                )
                .width(148)
                .height(168)
                .padding(12)
                .style(|_| theme::card(false, true)),
            );
        }
        for instance in &self.instances {
            let selected = self.selected.as_ref() == Some(&instance.name);
            let hovered = self.hovered.as_ref() == Some(&instance.name);
            let status: Element<'_, Message> = if self.runs.contains_key(&instance.name) {
                text("● Running").size(11).color(theme::SUCCESS).into()
            } else if self.jobs.contains_key(&instance.name) {
                text("Installing…").size(11).color(theme::WARNING).into()
            } else {
                text(instance.minecraft_version())
                    .size(11)
                    .color(theme::MUTED)
                    .into()
            };
            let card = container(
                column![
                    self.instance_icon(instance, 52.0),
                    text(clip(&instance.name, 15))
                        .size(14)
                        .font(BOLD)
                        .width(Length::Fill)
                        .center(),
                    status,
                    chip(
                        instance
                            .loader
                            .as_ref()
                            .map_or("Vanilla", |l| l.kind.name()),
                        loader_color(instance.loader.as_ref().map(|l| l.kind)),
                    ),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .width(148)
            .height(168)
            .padding(12)
            .style(move |_| theme::card(selected, hovered));
            cards = cards.push(
                mouse_area(card)
                    .on_press(Message::Select(instance.name.clone()))
                    .on_double_click(Message::Play(instance.name.clone()))
                    .on_enter(Message::Hover(Some(instance.name.clone())))
                    .on_exit(Message::Hover(None))
                    .interaction(iced::mouse::Interaction::Pointer),
            );
        }
        let new_card = button(
            container(
                column![
                    text("+").size(40).color(theme::VIOLET_BRIGHT),
                    text("New instance").size(13)
                ]
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        )
        .width(148)
        .height(168)
        .style(theme::glass_button)
        .on_press(Message::ShowNewInstance);
        cards = cards.push(new_card);

        container(
            column![
                row![
                    text("Instances").size(18).font(BOLD),
                    chip(self.instances.len().to_string(), theme::VIOLET_BRIGHT),
                    space::horizontal(),
                    text("Double-click to play").size(12).color(theme::MUTED),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                scrollable(cards.wrap().vertical_spacing(12))
                    .style(theme::scroll)
                    .height(Length::Fill),
            ]
            .spacing(14),
        )
        .padding(18)
        .width(Length::FillPortion(5))
        .height(Length::Fill)
        .style(theme::glass)
        .into()
    }

    fn details_panel(&self) -> Element<'_, Message> {
        let Some(instance) = self.selected.as_ref().and_then(|s| self.instance(s)) else {
            return container(
                column![
                    text("Welcome to netherNode").size(24).font(BOLD),
                    text("Create an instance: pick a Minecraft version, add Fabric, Forge or another loader if you like, and press Play.")
                        .color(theme::MUTED)
                        .center(),
                    button(text("+  Create your first instance").size(16))
                        .padding([12, 22])
                        .style(theme::play)
                        .on_press(Message::ShowNewInstance),
                ]
                .spacing(16)
                .align_x(Alignment::Center)
                .max_width(420),
            )
            .center(Length::Fill)
            .width(Length::FillPortion(7))
            .style(theme::glass)
            .into();
        };

        let name = instance.name.clone();
        let busy = self.runs.contains_key(&name) || self.jobs.contains_key(&name);
        let memory = match (
            self.runs.get(&name).and_then(|r| r.heap.clone()),
            instance.max_memory_mb,
        ) {
            (Some(heap), _) => format!("Memory {heap}"),
            (None, Some(mb)) => format!("Memory up to {mb} MB"),
            (None, None) => "Memory: automatic".into(),
        };
        let deleting = self.confirm_delete.as_ref() == Some(&name);
        let header =
            row![
                self.instance_icon(instance, 64.0),
                column![
                    text(clip(&instance.name, 26)).size(24).font(BOLD),
                    row![
                        chip(
                            format!("Minecraft {}", instance.minecraft_version()),
                            theme::VIOLET_BRIGHT
                        ),
                        chip(
                            loader_label(instance),
                            loader_color(instance.loader.as_ref().map(|l| l.kind))
                        ),
                    ]
                    .extend(instance.modpack.as_ref().map(|p| chip(
                        format!("Modpack: {} {}", p.name, p.version),
                        theme::MAGENTA
                    )),)
                    .spacing(6),
                    text(memory).size(12).color(theme::MUTED),
                ]
                .spacing(6)
                .width(Length::Fill),
            ]
            .spacing(16)
            .align_y(Alignment::Center);

        // Always shown, so it's easy to find; it works once the world is open to LAN.
        // Clickable while the game runs: the world is found in the log or, failing that,
        // from the game's LAN announcement when clicked.
        let running = self.runs.get(&name).is_some_and(|r| r.heap.is_some());
        let shared = self
            .shares
            .iter()
            .any(|s| !s.is_server && s.instance == name);
        let invite: Option<Element<'_, Message>> = (!shared).then(|| {
            button(text("Invite friends").size(14).font(BOLD))
                .padding([8, 16])
                .style(theme::play)
                .on_press_maybe(running.then(|| Message::InviteFriends(name.clone())))
                .into()
        });
        // While the game runs, the console area already explains Open to LAN.
        let invite_hint = (!self.runs.contains_key(&name) && !shared).then(|| {
            text("To invite friends: play, load a world, then Esc → Open to LAN. Friends use Join friend.")
                .size(12)
                .color(theme::MUTED)
        });
        let actions = row![
            small_button("Mods").on_press(Message::ShowMods(name.clone())),
            small_button("Edit").on_press_maybe((!busy).then(|| Message::ShowEdit(name.clone()))),
            small_button("Folder").on_press(Message::OpenFolder(instance.game_dir(&self.paths))),
        ]
        .extend(invite)
        .push(space::horizontal())
        .push(
            button(text(if deleting { "Really delete?" } else { "Delete" }).size(14))
                .padding([8, 14])
                .style(theme::danger_button)
                .on_press_maybe((!busy).then(|| Message::Delete(name.clone()))),
        )
        .spacing(8);

        let mut body = column![header, actions].push(invite_hint).spacing(14);
        if self
            .runs
            .get(&name)
            .is_some_and(|r| r.heap.is_some() && r.lan_port.is_none())
        {
            body = body.push(
                text("Playing with friends? In the game press Esc → Open to LAN → Start LAN World, then click Invite friends here.")
                    .size(12)
                    .color(theme::MUTED),
            );
        }
        let stage = self
            .jobs
            .get(&name)
            .map(|j| (j.label.clone(), j.stage.clone()))
            .or_else(|| {
                self.runs
                    .get(&name)
                    .and_then(|r| r.stage.clone())
                    .map(|s| ("Preparing".to_string(), Some(s)))
            });
        if let Some((label, stage)) = stage {
            let (detail, done, total) = match stage {
                Some((stage, done, total)) => {
                    (format!("{label}: {stage} {done}/{total}"), done, total)
                }
                None => (format!("{label}…"), 0, 1),
            };
            body = body.push(
                column![
                    text(detail).size(13).color(theme::MUTED),
                    progress_bar(0.0..=total.max(1) as f32, done as f32)
                        .girth(8)
                        .style(theme::progress),
                ]
                .spacing(6),
            );
        }
        if let Some(crash) = self.crashes.get(&name) {
            body = body.push(view_crash(&name, crash));
        }

        let console: Element<'_, Message> = match self.logs.get(&name).filter(|l| !l.is_empty()) {
            None => container(
                column![
                    text("Game output will appear here.").color(theme::MUTED),
                    text("Tip: double-click an instance card to play it right away.")
                        .size(12)
                        .color(theme::MUTED),
                ]
                .spacing(4)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .style(theme::console)
            .into(),
            Some(lines) => {
                let shown = lines.len().min(LOG_VISIBLE);
                let column = column(
                    lines
                        .iter()
                        .skip(lines.len() - shown)
                        .map(|line| text(line).size(12).font(MONO).into()),
                )
                .spacing(2);
                container(
                    scrollable(container(column).padding(12).width(Length::Fill))
                        .anchor_bottom()
                        .style(theme::scroll)
                        .height(Length::Fill),
                )
                .style(theme::console)
                .height(Length::Fill)
                .into()
            }
        };
        body = body.push(console);

        container(body)
            .padding(18)
            .width(Length::FillPortion(7))
            .height(Length::Fill)
            .style(theme::glass)
            .into()
    }

    fn play_bar(&self) -> Element<'_, Message> {
        let names: Vec<String> = self
            .accounts
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect();
        let account = column![
            text("PLAYING AS").size(11).color(theme::MUTED),
            row![
                pick_list(
                    names,
                    self.accounts.default.clone(),
                    Message::SetDefaultAccount
                )
                .placeholder("Add an account")
                .style(theme::picker)
                .width(190),
                small_button("Manage").on_press(Message::ShowAccounts),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(4);

        let selected = self.selected.as_ref().and_then(|s| self.instance(s));
        let names: Vec<String> = self.instances.iter().map(|i| i.name.clone()).collect();
        let picker = pick_list(names, self.selected.clone(), Message::Select)
            .placeholder("Choose an instance")
            .style(theme::picker)
            .width(220);
        let detail = match selected {
            Some(i) => format!("Minecraft {} · {}", i.minecraft_version(), loader_label(i)),
            None => "Create an instance to play".into(),
        };
        // Same shape as "Playing as": label above, controls in one row.
        let target: Element<'_, Message> = column![
            text("INSTANCE").size(11).color(theme::MUTED),
            row![picker, text(detail).size(12).color(theme::MUTED)]
                .spacing(10)
                .align_y(Alignment::Center),
        ]
        .spacing(4)
        .into();

        let big = |label: &str| {
            text(label.to_string())
                .size(20)
                .font(BOLD)
                .width(Length::Fill)
                .center()
        };
        let action = match selected {
            Some(i) if self.jobs.contains_key(&i.name) => {
                button(big("Installing…")).style(theme::play)
            }
            Some(i) => match self.runs.get(&i.name) {
                Some(r) if r.stop.is_none() => button(big("Stopping…")).style(theme::stop),
                Some(r) if r.heap.is_none() => button(big("Preparing…"))
                    .style(theme::stop)
                    .on_press(Message::Stop(i.name.clone())),
                Some(_) => button(big("■  Stop"))
                    .style(theme::stop)
                    .on_press(Message::Stop(i.name.clone())),
                None => button(big("▶  PLAY")).style(theme::play).on_press_maybe(
                    (!self.accounts.accounts.is_empty()).then(|| Message::Play(i.name.clone())),
                ),
            },
            None => button(big("▶  PLAY")).style(theme::play),
        };

        container(
            row![
                account,
                target,
                space::horizontal(),
                action.width(240).padding([16, 20])
            ]
            .spacing(28)
            .align_y(Alignment::Center),
        )
        .padding([12, 18])
        .style(theme::glass_dense)
        .into()
    }

    /// A centered form card over the portal.
    fn dialog<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        let card = container(content)
            .padding(26)
            .max_width(680)
            .style(theme::glass_dense);
        scrollable(container(card).center_x(Length::Fill))
            .style(theme::scroll)
            .height(Length::Fill)
            .into()
    }

    fn loader_picker<'a>(
        &'a self,
        pick: &'a LoaderPick,
        minecraft: Option<&'a str>,
    ) -> Element<'a, Message> {
        let mc = minecraft.unwrap_or("this version");
        // Loaders with no release for this Minecraft version are greyed out.
        let unavailable: Vec<LoaderKind> = LoaderKind::ALL
            .into_iter()
            .filter(|k| minecraft.is_some_and(|mc| self.loader_unavailable(mc, *k)))
            .collect();
        let options = [None]
            .into_iter()
            .chain(LoaderKind::ALL.into_iter().map(Some))
            .map(|kind| {
                let label = kind.map_or("Vanilla", |k| k.name());
                let disabled = kind.is_some_and(|k| unavailable.contains(&k)) && pick.kind != kind;
                button(text(label).size(14))
                    .padding([8, 14])
                    .style(theme::segment(pick.kind == kind))
                    .on_press_maybe((!disabled).then_some(Message::PickLoader(kind)))
                    .into()
            });
        let segments = row(options).spacing(6).wrap().vertical_spacing(6);

        let about = match pick.kind {
            None => "Plain Minecraft, no mods.",
            Some(LoaderKind::Fabric) => {
                "Lightweight and fast to update. Home of Sodium, Iris and most performance mods."
            }
            Some(LoaderKind::Quilt) => "A Fabric fork that also runs most Fabric mods.",
            Some(LoaderKind::Forge) => {
                "The classic modding platform with the biggest library of large mods."
            }
            Some(LoaderKind::NeoForge) => {
                "The modern Forge fork, used by most new Forge-style mods from 1.20.2 on."
            }
            Some(LoaderKind::OptiFine) => "Better graphics, performance and shaders.",
        };
        let mut section =
            column![text("Mod loader").size(13).color(theme::MUTED), segments].spacing(8);
        if !unavailable.is_empty() {
            let names: Vec<&str> = unavailable.iter().map(|k| k.name()).collect();
            section = section.push(
                text(format!(
                    "Not released yet for Minecraft {mc}: {}",
                    names.join(", ")
                ))
                .size(12)
                .color(theme::WARNING),
            );
        }
        section = section.push(text(about).size(12).color(theme::MUTED));

        match pick.kind {
            None => {}
            Some(LoaderKind::OptiFine) => {
                let choice = row![
                    button(text("One-click alternative (recommended)").size(13))
                        .padding([6, 12])
                        .style(theme::segment(pick.optifine_alt))
                        .on_press(Message::OptiFineAlternative(true)),
                    button(text("Use a real OptiFine jar").size(13))
                        .padding([6, 12])
                        .style(theme::segment(!pick.optifine_alt))
                        .on_press(Message::OptiFineAlternative(false)),
                ]
                .spacing(6);
                let detail: Element<'a, Message> = if pick.optifine_alt {
                    let features = nethernode_core::mods::OPTIFINE_ALTERNATIVE
                        .iter()
                        .map(|(_, what)| chip(*what, theme::VIOLET_BRIGHT));
                    let mut col = column![
                        text("Installs Fabric plus open-source mods that do what OptiFine does. Nothing to download yourself:")
                            .size(13),
                        row(features).spacing(6).wrap().vertical_spacing(6),
                    ]
                    .spacing(10);
                    if minecraft.is_some_and(|mc| self.loader_unavailable(mc, LoaderKind::Fabric)) {
                        col = col.push(
                            text(format!("Fabric isn't released for Minecraft {mc} yet, so this can't be installed."))
                                .size(12)
                                .color(theme::WARNING),
                        );
                    }
                    col.into()
                } else {
                    let chosen = pick
                        .optifine_jar
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned());
                    column![
                        text("OptiFine's author doesn't allow other apps to download it, so it takes two steps:").size(13),
                        row![
                            text(format!("1. Download OptiFine for Minecraft {mc}")).size(13).width(Length::Fill),
                            small_button("Open optifine.net").on_press(Message::OpenUrl("https://optifine.net/downloads".into())),
                        ]
                        .align_y(Alignment::Center),
                        row![
                            text(match &chosen {
                                Some(name) => format!("2. Chosen: {name}"),
                                None => "2. Choose the downloaded .jar".into(),
                            })
                            .size(13)
                            .width(Length::Fill),
                            small_button("Browse…").on_press(Message::BrowseOptiFine),
                        ]
                        .align_y(Alignment::Center),
                    ]
                    .spacing(10)
                    .into()
                };
                section = section.push(
                    container(column![choice, detail].spacing(12))
                        .padding(14)
                        .style(theme::row_item),
                );
            }
            Some(kind) => {
                let versions: Element<'a, Message> = match &pick.versions {
                    None | Some(Load::Loading) => text(format!("Loading {kind} versions…"))
                        .size(13)
                        .color(theme::MUTED)
                        .into(),
                    Some(Load::Failed(e)) => text(format!("Couldn't load versions: {e}"))
                        .size(13)
                        .color(theme::DANGER)
                        .into(),
                    Some(Load::Ready(list)) if list.is_empty() => text(format!(
                        "{kind} isn't available for Minecraft {}.",
                        minecraft.unwrap_or("this version")
                    ))
                    .size(13)
                    .color(theme::WARNING)
                    .into(),
                    Some(Load::Ready(list)) => {
                        let names: Vec<String> = list
                            .iter()
                            .take(60)
                            .map(|v| {
                                if v.stable {
                                    format!("{} (recommended)", v.version)
                                } else {
                                    v.version.clone()
                                }
                            })
                            .collect();
                        let selected = pick.version.as_ref().and_then(|v| {
                            names
                                .iter()
                                .find(|n| n.split(' ').next() == Some(v.as_str()))
                                .cloned()
                        });
                        pick_list(names, selected, |label: String| {
                            Message::PickLoaderVersion(
                                label.split(' ').next().unwrap_or_default().to_string(),
                            )
                        })
                        .style(theme::picker)
                        .width(Length::Fill)
                        .into()
                    }
                };
                section = section.push(field("Loader version", versions));
            }
        }
        section.into()
    }

    fn new_instance_form<'a>(&'a self, form: &'a NewInstanceForm) -> Element<'a, Message> {
        // Mojang's full list (refreshed when this form opens), not just downloaded versions.
        let choices = self.version_choices(form.filter);
        let selected = form
            .version
            .as_ref()
            .and_then(|v| choices.iter().find(|c| &c.id == v).cloned());
        let filters = [
            (VersionFilter::Releases, "Releases"),
            (VersionFilter::Snapshots, "+ Snapshots"),
            (VersionFilter::All, "All (incl. alpha/beta)"),
        ]
        .into_iter()
        .map(|(filter, label)| {
            button(text(label).size(13))
                .padding([6, 12])
                .style(theme::segment(form.filter == filter))
                .on_press(Message::NewFilter(filter))
                .into()
        });
        let status = if self.refreshing_versions {
            "Checking Mojang for new versions…".to_string()
        } else {
            let downloaded = choices.iter().filter(|c| c.downloaded).count();
            format!(
                "{} versions available · {downloaded} already downloaded (✓)",
                choices.len()
            )
        };
        let dropdown: Element<'a, Message> = if self.versions.is_some() {
            pick_list(choices, selected, |c: crate::VersionChoice| {
                Message::NewVersion(c.id)
            })
            .placeholder("Choose a Minecraft version")
            .style(theme::picker)
            .padding(10)
            .width(Length::Fill)
            .into()
        } else {
            text("Loading versions from Mojang…")
                .color(theme::MUTED)
                .into()
        };
        let versions: Element<'a, Message> = column![
            row![
                dropdown,
                button(text("Refresh").size(13))
                    .padding([10, 14])
                    .style(theme::glass_button)
                    .on_press_maybe(
                        (!self.refreshing_versions).then_some(Message::RefreshVersions)
                    ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row(filters).spacing(6),
            text(status).size(12).color(theme::MUTED),
        ]
        .spacing(8)
        .into();
        let can_create = form.version.is_some();
        column![
            // Actions sit in the header so they're visible without scrolling.
            row![
                text("New instance").size(26).font(BOLD),
                space::horizontal(),
                small_button("Cancel").on_press(Message::ShowHome),
                button(text("Create").size(15).font(BOLD))
                    .padding([10, 26])
                    .style(theme::play)
                    .on_press_maybe(can_create.then_some(Message::CreateInstance)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            field(
                "Name",
                text_input(form.version.as_deref().unwrap_or("My world"), &form.name)
                    .on_input(Message::NewName)
                    .on_submit_maybe(can_create.then_some(Message::CreateInstance))
                    .style(theme::input)
                    .padding(10),
            ),
            field("Minecraft version", versions),
            self.loader_picker(&form.loader, form.version.as_deref()),
            new_worlds_section(form),
            checkbox(form.low_end)
                .label("Low-end defaults (render distance 8, 60 FPS cap)")
                .on_toggle(Message::NewLowEnd),
        ]
        .spacing(18)
        .into()
    }

    fn servers_screen<'a>(&'a self, view: &'a ServersView) -> Element<'a, Message> {
        use nethernode_core::server::ServerKind;
        let kind_color = |k: ServerKind| match k {
            ServerKind::Vanilla => theme::SUCCESS,
            ServerKind::Fabric => Color::from_rgb8(0xe8, 0xc9, 0x9b),
            ServerKind::Paper => Color::from_rgb8(0x7f, 0xb6, 0xff),
            ServerKind::Forge => Color::from_rgb8(0xe0, 0x8a, 0x5a),
            ServerKind::NeoForge => Color::from_rgb8(0xf0, 0xa0, 0x50),
        };
        let status = |name: &str| -> (&'static str, Color) {
            if self.server_jobs.contains_key(name) {
                return ("Downloading", theme::WARNING);
            }
            match self.server_runs.get(name) {
                Some(r) if r.stop.is_none() => ("Stopping", theme::WARNING),
                Some(r) if r.ready => ("Running", theme::SUCCESS),
                Some(_) => ("Starting", theme::WARNING),
                None => ("Stopped", theme::MUTED),
            }
        };

        let mut list = column![].spacing(8);
        for srv in &self.servers {
            let selected = view.selected.as_ref() == Some(&srv.name);
            let (label, color) = status(&srv.name);
            list = list.push(
                button(
                    column![
                        row![
                            text(clip(&srv.name, 22))
                                .size(15)
                                .font(BOLD)
                                .width(Length::Fill),
                            text(label).size(11).color(color)
                        ]
                        .align_y(Alignment::Center),
                        row![
                            chip(srv.kind.name(), kind_color(srv.kind)),
                            text(format!("Minecraft {}", srv.minecraft))
                                .size(12)
                                .color(theme::MUTED)
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    ]
                    .spacing(6),
                )
                .width(Length::Fill)
                .padding([10, 12])
                .style(move |t, st| {
                    let mut style = theme::glass_button(t, st);
                    if selected {
                        style.border.color = theme::VIOLET_BRIGHT;
                    }
                    style
                })
                .on_press(Message::SelectServer(srv.name.clone())),
            );
        }
        let left = container(
            column![
                row![
                    text("Servers").size(18).font(BOLD),
                    space::horizontal(),
                    button(text("+ New server").size(13).font(BOLD))
                        .padding([6, 12])
                        .style(theme::play)
                        .on_press(Message::ShowNewServer),
                ]
                .align_y(Alignment::Center),
                text("Hosted on this PC. Friends join with Invite friends (no port forwarding).")
                    .size(12)
                    .color(theme::MUTED),
                scrollable(list).style(theme::scroll).height(Length::Fill),
                button(text("← Back to instances").size(13))
                    .style(theme::link)
                    .on_press(Message::ShowHome),
            ]
            .spacing(12),
        )
        .padding(18)
        .width(Length::FillPortion(4))
        .height(Length::Fill)
        .style(theme::glass);

        let Some(srv) = view
            .selected
            .as_ref()
            .and_then(|n| self.servers.iter().find(|s| &s.name == n))
        else {
            let empty = container(
                column![
                    text("Host your own server").size(22).font(BOLD),
                    text("Pick a Minecraft version and Vanilla, Fabric (mods) or Paper (plugins). It runs on this PC, and friends can join any time it's on, even when you're not playing.")
                        .color(theme::MUTED)
                        .center(),
                    button(text("+ New server").size(16).font(BOLD))
                        .padding([12, 22])
                        .style(theme::play)
                        .on_press(Message::ShowNewServer),
                ]
                .spacing(14)
                .align_x(Alignment::Center)
                .max_width(460),
            )
            .center(Length::Fill)
            .width(Length::FillPortion(7))
            .style(theme::glass);
            return row![left, empty].spacing(16).height(Length::Fill).into();
        };

        let name = srv.name.clone();
        let run = self.server_runs.get(&name);
        let ready = run.is_some_and(|r| r.ready);
        let installing = self.server_jobs.contains_key(&name);
        let eula = nethernode_core::server::eula_accepted(&self.paths, srv);
        let (label, color) = status(&name);
        let header = column![
            text(clip(&srv.name, 26)).size(24).font(BOLD),
            row![
                chip(srv.kind.name(), kind_color(srv.kind)),
                chip(format!("Minecraft {}", srv.minecraft), theme::VIOLET_BRIGHT),
                chip(format!("Port {}", srv.port), theme::MUTED),
                chip(label, color),
            ]
            .spacing(6),
        ]
        .spacing(6);

        let main_button = match run {
            Some(r) if r.stop.is_none() => button(text("Stopping…").size(15).font(BOLD))
                .padding([10, 22])
                .style(theme::stop),
            Some(_) => button(text("■  Stop").size(15).font(BOLD))
                .padding([10, 22])
                .style(theme::stop)
                .on_press(Message::StopServer(name.clone())),
            None => button(text("▶  Start").size(15).font(BOLD))
                .padding([10, 22])
                .style(theme::play)
                .on_press_maybe((eula && !installing).then(|| Message::StartServer(name.clone()))),
        };
        let deleting = view.confirm_delete;
        let actions = row![
            main_button,
            small_button("Invite friends").on_press_maybe(
                (ready
                    && !self
                        .shares
                        .iter()
                        .any(|s| s.is_server && s.instance == name))
                .then(|| Message::InviteServer(name.clone()))
            ),
            small_button("Settings").on_press(Message::EditServerSettings),
            small_button("Folder").on_press(Message::OpenFolder(srv.dir(&self.paths))),
            button(text(if deleting { "Really delete?" } else { "Delete" }).size(14))
                .padding([8, 14])
                .style(theme::danger_button)
                .on_press_maybe(run.is_none().then(|| Message::DeleteServer(name.clone()))),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .wrap()
        .vertical_spacing(8);

        let mut body = column![header, actions].spacing(14);
        if !eula {
            body = body.push(
                container(
                    row![
                        text("To start this server you need to accept the Minecraft EULA.")
                            .size(13)
                            .width(Length::Fill),
                        small_button("Read it")
                            .on_press(Message::OpenUrl(nethernode_core::server::EULA_URL.into())),
                        button(text("I accept the EULA").size(13).font(BOLD))
                            .padding([8, 14])
                            .style(theme::play)
                            .on_press(Message::AcceptEula(name.clone())),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([10, 14])
                .style(theme::notice(false)),
            );
        }
        let progress = self
            .server_jobs
            .get(&name)
            .map(|j| (j.label.clone(), j.stage.clone()))
            .or_else(|| {
                run.and_then(|r| r.stage.clone())
                    .map(|s| ("Preparing Java".to_string(), Some(s)))
            });
        if let Some((label, stage)) = progress {
            let (detail, done, total) = match stage {
                Some((stage, done, total)) => {
                    (format!("{label}: {stage} {done}/{total}"), done, total)
                }
                None => (format!("{label}…"), 0, 1),
            };
            body = body.push(
                column![
                    text(detail).size(13).color(theme::MUTED),
                    progress_bar(0.0..=total.max(1) as f32, done as f32)
                        .girth(8)
                        .style(theme::progress),
                ]
                .spacing(6),
            );
        }
        if ready {
            body = body.push(
                text(format!(
                    "To play on it yourself: in Minecraft, Multiplayer → \"{} (this PC)\" (added to your Minecraft {} instances), or Direct Connect to localhost:{}.",
                    clip(&srv.name, 24),
                    srv.minecraft,
                    srv.port
                ))
                .size(12)
                .color(theme::MUTED),
            );
        }

        let console: Element<'_, Message> = match run.map(|r| &r.log) {
            Some(lines) if !lines.is_empty() => {
                let shown = lines.len().min(LOG_VISIBLE);
                let column = column(
                    lines
                        .iter()
                        .skip(lines.len() - shown)
                        .map(|l| text(l).size(12).font(MONO).into()),
                )
                .spacing(2);
                container(
                    scrollable(container(column).padding(12).width(Length::Fill))
                        .anchor_bottom()
                        .style(theme::scroll)
                        .height(Length::Fill),
                )
                .style(theme::console)
                .height(Length::Fill)
                .into()
            }
            _ => container(
                column![
                    text("The server console appears here when it runs.").color(theme::MUTED),
                    row![
                        small_button("Import world folder…")
                            .on_press(Message::ServerImportWorld(name.clone(), false)),
                        small_button("Import world .zip…")
                            .on_press(Message::ServerImportWorld(name.clone(), true)),
                    ]
                    .spacing(8),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .style(theme::console)
            .into(),
        };
        body = body.push(console);
        if run.is_some() {
            body = body.push(
                row![
                    text_input(
                        "Server command, e.g. op Steve, time set day, say hi",
                        &view.command
                    )
                    .on_input(Message::ServerCommandInput)
                    .on_submit(Message::SendServerCommand)
                    .style(theme::input)
                    .font(MONO)
                    .padding(8),
                    small_button("Send").on_press(Message::SendServerCommand),
                ]
                .spacing(8),
            );
        }
        let right = container(body)
            .padding(18)
            .width(Length::FillPortion(7))
            .height(Length::Fill)
            .style(theme::glass);
        row![left, right].spacing(16).height(Length::Fill).into()
    }

    fn new_server_form<'a>(&'a self, form: &'a NewServerForm) -> Element<'a, Message> {
        use nethernode_core::server::ServerKind;
        let choices = self.version_choices(VersionFilter::Releases);
        let selected = form
            .version
            .as_ref()
            .and_then(|v| choices.iter().find(|c| &c.id == v).cloned());
        let mc = form.version.clone().unwrap_or_default();
        let unavailable = |k: ServerKind| {
            matches!(
                self.server_support.get(&mc).and_then(|m| m.get(&k)),
                Some(Load::Ready(false))
            )
        };
        let kinds = ServerKind::ALL.into_iter().map(|k| {
            let disabled = unavailable(k) && form.kind != k;
            button(text(k.name()).size(14))
                .padding([8, 14])
                .style(theme::segment(form.kind == k))
                .on_press_maybe((!disabled).then_some(Message::NewServerKind(k)))
                .into()
        });
        let about = match form.kind {
            ServerKind::Vanilla => "The official server, exactly like the game.",
            ServerKind::Fabric => "Fabric mods on the server (friends may need the same mods).",
            ServerKind::Paper => {
                "Faster, and supports plugins (Bukkit/Paper). Vanilla clients join as normal."
            }
            ServerKind::Forge => {
                "Forge mods on the server. Friends get the same mods through Join friend."
            }
            ServerKind::NeoForge => {
                "NeoForge mods on the server. Friends get the same mods through Join friend."
            }
        };
        let missing: Vec<&str> = ServerKind::ALL
            .into_iter()
            .filter(|k| unavailable(*k))
            .map(|k| k.name())
            .collect();
        let mut kind_section = column![
            text("Server type").size(13).color(theme::MUTED),
            row(kinds).spacing(6).wrap().vertical_spacing(6)
        ]
        .spacing(8);
        if !missing.is_empty() {
            kind_section = kind_section.push(
                text(format!(
                    "Not released yet for Minecraft {mc}: {}",
                    missing.join(", ")
                ))
                .size(12)
                .color(theme::WARNING),
            );
        }
        kind_section = kind_section.push(text(about).size(12).color(theme::MUTED));
        let can_create = form.version.is_some() && form.eula && !unavailable(form.kind);
        // "Based on an instance": same version, loader and mods (Quilt has no servers).
        const NOTHING: &str = "Nothing: choose the version and type below";
        let bases: Vec<String> = std::iter::once(NOTHING.to_string())
            .chain(
                self.instances
                    .iter()
                    .filter(|i| ServerKind::for_loader(i.loader.as_ref().map(|l| l.kind)).is_some())
                    .map(|i| i.name.clone()),
            )
            .collect();
        let based_on =
            column![
            field(
                "Based on an instance (copies its version, loader and mods)",
                pick_list(
                    bases,
                    Some(form.from_instance.clone().unwrap_or_else(|| NOTHING.to_string())),
                    |choice: String| Message::NewServerFrom((choice != NOTHING).then_some(choice)),
                )
                .style(theme::picker)
                .padding(10)
                .width(Length::Fill),
            ),
            text(if form.from_instance.is_some() {
                "Mods that only work in the game (like Sodium or Iris) are left out of the server."
            } else {
                ""
            })
            .size(12)
            .color(theme::MUTED),
        ]
            .spacing(6);
        let version_field: Element<'a, Message> = if form.from_instance.is_some() {
            text(format!("Minecraft {mc} · {}", form.kind.name()))
                .size(14)
                .into()
        } else {
            column![
                field(
                    "Minecraft version",
                    pick_list(choices, selected, |c: crate::VersionChoice| {
                        Message::NewServerVersion(c.id)
                    })
                    .placeholder("Choose a version")
                    .style(theme::picker)
                    .padding(10)
                    .width(Length::Fill),
                ),
                kind_section,
            ]
            .spacing(18)
            .into()
        };
        column![
            row![
                text("New server").size(26).font(BOLD),
                space::horizontal(),
                small_button("Cancel").on_press(Message::ShowServers),
                button(text("Create").size(15).font(BOLD))
                    .padding([10, 26])
                    .style(theme::play)
                    .on_press_maybe(can_create.then_some(Message::CreateServer)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            field(
                "Name",
                text_input("e.g. Survival with friends", &form.name)
                    .on_input(Message::NewServerName)
                    .style(theme::input)
                    .padding(10),
            ),
            based_on,
            version_field,
            column![
                checkbox(form.eula).label("I accept the Minecraft EULA").on_toggle(Message::NewServerEula),
                row![
                    text("Running Mojang's server software requires it.").size(12).color(theme::MUTED),
                    button(text("Read the EULA").size(12))
                        .style(theme::link)
                        .on_press(Message::OpenUrl(nethernode_core::server::EULA_URL.into())),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            ]
            .spacing(6),
            text("Offline accounts can join (with skins, through netherNode). Friends need your OK when they connect with a code (friends on your list get in straight away), and you can turn on a whitelist in its settings.")
                .size(12)
                .color(theme::MUTED),
        ]
        .spacing(18)
        .into()
    }

    /// "Friends" plus how many are online.
    fn friends_label(&self) -> String {
        let online = self
            .friends
            .as_ref()
            .map_or(0, |f| f.list().iter().filter(|(_, p)| p.online).count());
        if online > 0 {
            format!("Friends ● {online}")
        } else {
            "Friends".into()
        }
    }

    fn friends_screen<'a>(&'a self, view: &'a FriendsView) -> Element<'a, Message> {
        let header = row![
            text("Friends").size(26).font(BOLD).width(Length::Fill),
            small_button("Done").on_press(Message::ShowHome),
        ]
        .align_y(Alignment::Center);
        let ready = self.friends.is_some();
        // Where the friends service stands, in one line.
        let status: Element<'a, Message> = match (&self.friends, &self.friends_error) {
            (Some(_), _) => text("● Connected: friends can see you while netherNode is open.")
                .size(13)
                .color(theme::SUCCESS)
                .into(),
            (None, Some(e)) => text(format!(
                "Couldn't connect to the friends network ({e}). Trying again every 30 seconds; check your internet connection."
            ))
            .size(13)
            .color(theme::WARNING)
            .into(),
            (None, None) => text("Connecting to the friends network… (this takes a few seconds after netherNode starts)")
                .size(13)
                .color(theme::MUTED)
                .into(),
        };
        let how = column![
            text("How it works").size(13).color(theme::MUTED),
            text("1. Click Show my friend code and send your friend the four words (or read them out).").size(13),
            text("2. Your friend types them under Add a friend on their netherNode. You get a request to accept.").size(13),
            text("3. Done: you each see when the other is online, and anything they share (a world opened to LAN or a server) appears here with a Join button. Friends get in without codes or Allow prompts.").size(13),
        ]
        .spacing(4);

        let my_code: Element<'a, Message> = match &view.my_code {
            Some(code) => row![
                column![
                    text("Your friend code (works for about an hour):").size(13),
                    text(code).size(20).font(BOLD).color(theme::VIOLET_BRIGHT),
                ]
                .spacing(4)
                .width(Length::Fill),
                small_button("Copy").on_press(Message::CopyText(code.clone())),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
            None => row![
                text("Give a friend your four-word friend code so they can add you.")
                    .size(13)
                    .width(Length::Fill),
                small_button(if view.busy {
                    "Working…"
                } else {
                    "Show my friend code"
                })
                .on_press_maybe((ready && !view.busy).then_some(Message::MakeFriendCode)),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        };
        let can_add = ready && !view.code.trim().is_empty() && !view.busy;
        let add = row![
            text_input(
                "Their friend code, e.g. maple orbit tunnel frog",
                &view.code
            )
            .on_input(Message::FriendCodeInput)
            .on_submit_maybe(can_add.then_some(Message::AddFriend))
            .style(theme::input)
            .padding(10),
            button(text("Add friend").font(BOLD))
                .padding([10, 18])
                .style(theme::play)
                .on_press_maybe(can_add.then_some(Message::AddFriend)),
        ]
        .spacing(10);

        let mut list = column![].spacing(8);
        let mut entries = self.friends.as_ref().map(|f| f.list()).unwrap_or_default();
        entries.sort_by_key(|(f, p)| (!p.online, f.name.to_lowercase()));
        if entries.is_empty() {
            list = list.push(text("No friends yet.").size(13).color(theme::MUTED));
        }
        for (friend, presence) in entries {
            let (dot, state) = if !friend.confirmed {
                (theme::MUTED, "waiting for them to accept")
            } else if presence.online {
                (theme::SUCCESS, "online")
            } else {
                (theme::MUTED, "offline")
            };
            let mut col = column![
                row![
                    text("●").size(14).color(dot),
                    text(clip(&friend.name, 24)).size(16).font(BOLD),
                    text(state).size(12).color(theme::MUTED),
                    space::horizontal(),
                    small_button("Remove").on_press(Message::RemoveFriend(friend.id.clone())),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
            ]
            .spacing(6);
            for shared in &presence.sharing {
                col = col.push(
                    row![
                        text(format!(
                            "{} {} · {}",
                            if shared.server { "Server" } else { "World" },
                            clip(&shared.title, 24),
                            shared.summary
                        ))
                        .size(13)
                        .width(Length::Fill),
                        button(text("Join").size(14).font(BOLD))
                            .padding([6, 16])
                            .style(theme::play)
                            .on_press(Message::JoinShared(shared.code.clone())),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
            list = list.push(container(col).padding(12).style(theme::row_item));
        }
        column![
            header,
            status,
            how,
            my_code,
            field("Add a friend", add),
            text("Your friends").size(13).color(theme::MUTED),
            list,
        ]
        .spacing(18)
        .into()
    }

    /// Banners for active shares (host), friend requests and a joined friend's world. Shown
    /// on every screen, since requests need an answer wherever the player is.
    fn share_banners(&self) -> Vec<Element<'_, Message>> {
        let mut banners = Vec::new();
        for request in self
            .friends
            .as_ref()
            .map(|f| f.requests())
            .unwrap_or_default()
        {
            banners.push(
                container(
                    row![
                        text(format!(
                            "{} wants to be your friend.",
                            clip(&request.name, 24)
                        ))
                        .size(14)
                        .width(Length::Fill),
                        button(text("Accept").size(14).font(BOLD))
                            .padding([6, 16])
                            .style(theme::play)
                            .on_press(Message::AcceptFriend(request.id.clone())),
                        button(text("Decline").size(14))
                            .padding([6, 14])
                            .style(theme::danger_button)
                            .on_press(Message::DeclineFriend(request.id.clone())),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([10, 16])
                .style(theme::glass_dense)
                .into(),
            );
        }
        for share in &self.shares {
            let key = crate::share_key(share.is_server, &share.instance);
            let mut col = column![].spacing(10);
            let head: Element<'_, Message> = match &share.code {
                None => text(format!(
                    "Getting {} ready to share…",
                    clip(&share.instance, 30)
                ))
                .into(),
                Some(code) => row![
                    column![
                        text(format!(
                            "Sharing {} with friends. Send them this code:",
                            clip(&share.instance, 30)
                        ))
                        .size(13),
                        text(code).size(18).font(BOLD).color(theme::VIOLET_BRIGHT),
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                    small_button("Copy code").on_press(Message::CopyText(code.clone())),
                    // The full code, for when the short one can't be looked up.
                    small_button("Copy full code").on_press_maybe(
                        share
                            .full_code
                            .clone()
                            .filter(|full| full != code)
                            .map(Message::CopyText),
                    ),
                    button(text("Stop sharing").size(14))
                        .padding([8, 14])
                        .style(theme::danger_button)
                        .on_press(Message::StopSharing(key.clone())),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            };
            col = col.push(head);
            for (i, (friend, _)) in share.requests.iter().enumerate() {
                col = col.push(
                    container(
                        row![
                            text(format!(
                                "Someone with your code ({friend}) wants to join. Allow?"
                            ))
                            .size(14)
                            .width(Length::Fill),
                            button(text("Allow").size(14).font(BOLD))
                                .padding([6, 16])
                                .style(theme::play)
                                .on_press(Message::AnswerJoin(key.clone(), i, true)),
                            button(text("Deny").size(14))
                                .padding([6, 14])
                                .style(theme::danger_button)
                                .on_press(Message::AnswerJoin(key.clone(), i, false)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .padding([8, 12])
                    .style(theme::notice(false)),
                );
            }
            if !share.friends.is_empty() {
                col = col.push(
                    text(format!("In your world: {}", share.friends.join(", ")))
                        .size(12)
                        .color(theme::MUTED),
                );
            }
            banners.push(
                container(col)
                    .padding([12, 16])
                    .style(theme::glass_dense)
                    .into(),
            );
        }
        if let Some(session) = &self.join_session {
            use nethernode_core::share::JoinEvent;
            let (msg, port) = match &session.status {
                JoinEvent::Connecting => ("Connecting to your friend…".to_string(), None),
                JoinEvent::WaitingForApproval => {
                    ("Waiting for your friend to let you in…".to_string(), None)
                }
                JoinEvent::Ready {
                    port,
                    title,
                    blocked,
                } => (
                    format!(
                        "Connected to {}! In Minecraft: Multiplayer → it's under LAN, or Direct Connect to localhost:{port}{}",
                        clip(title, 30),
                        if blocked.is_empty() {
                            String::new()
                        } else {
                            format!(
                                ". Couldn't open {} here (another program uses it)",
                                blocked
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    ),
                    Some(*port),
                ),
                JoinEvent::Mods(..) => (String::new(), None),
            };
            let mut row = row![text(msg).size(13).width(Length::Fill)]
                .spacing(8)
                .align_y(Alignment::Center);
            if let Some(port) = port {
                row = row.push(
                    small_button("Copy address")
                        .on_press(Message::CopyText(format!("localhost:{port}"))),
                );
            }
            row = row.push(
                button(text("Leave").size(14))
                    .padding([8, 14])
                    .style(theme::danger_button)
                    .on_press(Message::LeaveFriend),
            );
            let mut col = column![row].spacing(10);
            // A modded world: offer the same mods, or the instance that has them.
            if let Some((manifest, _)) = &session.mods {
                let mods_row = match &session.matching {
                    Some(instance) if self.jobs.contains_key(instance) => row![
                        text(format!(
                            "Getting your friend's mods into {}…",
                            clip(instance, 24)
                        ))
                        .size(13)
                        .width(Length::Fill),
                    ],
                    Some(instance) => row![
                        text(format!(
                            "Your friend plays with mods ({}). {} has them.",
                            manifest.summary(),
                            clip(instance, 24)
                        ))
                        .size(13)
                        .width(Length::Fill),
                        button(text("▶  Play it").size(14).font(BOLD))
                            .padding([8, 16])
                            .style(theme::play)
                            .on_press(Message::PlayOnFriend(instance.clone())),
                    ],
                    None => row![
                        text(format!(
                            "Your friend plays with mods ({}). You need the same ones to join.",
                            manifest.summary()
                        ))
                        .size(13)
                        .width(Length::Fill),
                        button(text("Get the same mods").size(14).font(BOLD))
                            .padding([8, 16])
                            .style(theme::play)
                            .on_press(Message::GetFriendMods),
                    ],
                };
                col = col.push(mods_row.spacing(8).align_y(Alignment::Center));
            }
            banners.push(
                container(col)
                    .padding([12, 16])
                    .style(theme::glass_dense)
                    .into(),
            );
        }
        banners
    }

    fn edit_worlds_section<'a>(&'a self, form: &'a EditForm) -> Element<'a, Message> {
        let Some(instance) = self.instance(&form.original) else {
            return space().into();
        };
        let worlds = nethernode_core::worlds::list(&self.paths, instance);
        let list: Element<'a, Message> = if worlds.is_empty() {
            text("No worlds yet.").size(12).color(theme::MUTED).into()
        } else {
            row(worlds
                .into_iter()
                .map(|w| chip(w.name, theme::VIOLET_BRIGHT)))
            .spacing(6)
            .wrap()
            .vertical_spacing(6)
            .into()
        };
        column![
            text("Worlds").size(13).color(theme::MUTED),
            list,
            row![
                small_button("Import world folder…").on_press(Message::EditImportWorlds(false)),
                small_button("Import world .zip…").on_press(Message::EditImportWorlds(true)),
                small_button("Open saves folder").on_press(Message::OpenFolder(
                    nethernode_core::worlds::saves_dir(&self.paths, instance)
                )),
            ]
            .spacing(8)
            .wrap()
            .vertical_spacing(8),
        ]
        .spacing(8)
        .into()
    }

    fn settings_form<'a>(&'a self, form: &'a SettingsForm) -> Element<'a, Message> {
        use nethernode_core::settings::Renderer;
        let folder = |label: &'a str,
                      path: String,
                      which: FolderKind,
                      hint: &'a str|
         -> Element<'a, Message> {
            column![
                text(label).size(13).color(theme::MUTED),
                row![
                    container(text(path).size(13).font(MONO))
                        .padding([9, 12])
                        .width(Length::Fill)
                        .style(theme::row_item),
                    small_button("Change…")
                        .on_press_maybe((!form.busy).then_some(Message::SettingsPick(which))),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                text(hint).size(12).color(theme::MUTED),
            ]
            .spacing(6)
            .into()
        };
        let instances_path = form
            .instances_dir
            .clone()
            .unwrap_or_else(|| form.data_dir.join("instances"))
            .display()
            .to_string();
        let mut instances = column![folder(
            "Instances and worlds",
            instances_path,
            FolderKind::Instances,
            "Each instance's worlds, mods and settings.",
        )]
        .spacing(6);
        if form.instances_dir.is_some() {
            instances = instances.push(
                button(text("Keep them inside the game files folder").size(12))
                    .style(theme::link)
                    .on_press_maybe((!form.busy).then_some(Message::SettingsResetInstances)),
            );
        }
        let servers_path = form
            .servers_dir
            .clone()
            .unwrap_or_else(|| form.data_dir.join("servers"))
            .display()
            .to_string();
        let mut servers = column![folder(
            "Servers",
            servers_path,
            FolderKind::Servers,
            "Servers hosted on this PC, with their worlds.",
        )]
        .spacing(6);
        if form.servers_dir.is_some() {
            servers = servers.push(
                button(text("Keep them inside the game files folder").size(12))
                    .style(theme::link)
                    .on_press_maybe((!form.busy).then_some(Message::SettingsResetServers)),
            );
        }
        let renderers = [
            (Renderer::Auto, "Automatic"),
            (Renderer::Gpu, "GPU (animated portal)"),
            (Renderer::Cpu, "CPU (lightest)"),
        ]
        .into_iter()
        .map(|(r, label)| {
            button(text(label).size(13))
                .padding([6, 12])
                .style(theme::segment(form.renderer == r))
                .on_press(Message::SettingsRenderer(r))
                .into()
        });
        let save_label = if form.busy { "Moving files…" } else { "Save" };
        column![
            row![
                text("Settings").size(26).font(BOLD),
                space::horizontal(),
                small_button("Cancel").on_press_maybe((!form.busy).then_some(Message::ShowHome)),
                button(text(save_label).size(15).font(BOLD))
                    .padding([10, 26])
                    .style(theme::play)
                    .on_press_maybe((!form.busy).then_some(Message::SaveSettings)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            folder(
                "Game files (Minecraft downloads)",
                form.data_dir.display().to_string(),
                FolderKind::GameFiles,
                "Versions, libraries, assets and Java runtimes, shared by all instances.",
            ),
            instances,
            servers,
            checkbox(form.move_files)
                .label("Move my existing files to the new folders")
                .on_toggle_maybe((!form.busy).then_some(Message::SettingsMoveFiles)),
            column![
                text("Launcher graphics").size(13).color(theme::MUTED),
                row(renderers).spacing(6).wrap().vertical_spacing(6),
                text(format!(
                    "Automatic uses the GPU unless this computer has 4 GB of RAM or less (it has {} GB). Takes effect after a restart.",
                    self.memory.total_mb / 1024
                ))
                .size(12)
                .color(theme::MUTED),
                checkbox(form.animated_background)
                    .label("Animated nether portal background")
                    .on_toggle(Message::SettingsAnimation),
            ]
            .spacing(8),
            column![
                text("In game").size(13).color(theme::MUTED),
                checkbox(form.skins)
                    .label("Custom skins (yours and your friends')")
                    .on_toggle(Message::SettingsSkins),
                text("Choose skins under Accounts. Uses authlib-injector, an open-source helper downloaded once from its official site. Nothing logs in anywhere.")
                    .size(12)
                    .color(theme::MUTED),
            ]
            .spacing(8),
            row![
                small_button("Open game files folder").on_press(Message::OpenFolder(self.paths.root.clone())),
                small_button("Open instances folder").on_press(Message::OpenFolder(self.paths.instances())),
            ]
            .spacing(8),
        ]
        .spacing(20)
        .into()
    }

    fn window_section<'a>(&'a self, form: &'a EditForm) -> Element<'a, Message> {
        const PRESETS: [(u32, u32, &str); 4] = [
            (854, 480, "Small (Minecraft default)"),
            (1280, 720, "HD 1280×720"),
            (1600, 900, "1600×900"),
            (1920, 1080, "Full HD 1920×1080"),
        ];
        let current = (
            form.width.parse::<u32>().ok(),
            form.height.parse::<u32>().ok(),
        );
        let presets = PRESETS.into_iter().map(|(w, h, label)| {
            button(text(label).size(13))
                .padding([6, 12])
                .style(theme::segment(current == (Some(w), Some(h))))
                .on_press(Message::EditWindow(w.to_string(), h.to_string()))
                .into()
        });
        let height = form.height.clone();
        let width = form.width.clone();
        column![
            text("Game window").size(13).color(theme::MUTED),
            row(presets).spacing(6).wrap().vertical_spacing(6),
            row![
                text_input("Width", &form.width)
                    .on_input(move |w| Message::EditWindow(w, height.clone()))
                    .style(theme::input)
                    .padding(8)
                    .width(110),
                text("×").color(theme::MUTED),
                text_input("Height", &form.height)
                    .on_input(move |h| Message::EditWindow(width.clone(), h))
                    .style(theme::input)
                    .padding(8)
                    .width(110),
                space::horizontal().width(12),
                checkbox(form.fullscreen)
                    .label("Start in fullscreen")
                    .on_toggle(Message::EditFullscreen),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text("In game, F11 switches between window and fullscreen.")
                .size(12)
                .color(theme::MUTED),
        ]
        .spacing(8)
        .into()
    }

    fn edit_form<'a>(&'a self, form: &'a EditForm) -> Element<'a, Message> {
        let minecraft = self.instance(&form.original).map(|i| i.minecraft_version());
        let note = format!(
            "This computer has {} MB of RAM. Automatic uses about a quarter of it ({} MB) and shrinks if less is free.",
            self.memory.total_mb,
            self.memory.recommended_max_heap_mb()
        );
        let mut memory = column![
            checkbox(form.auto_memory)
                .label("Automatic memory")
                .on_toggle(Message::EditAutoMemory),
            text(note).size(12).color(theme::MUTED),
        ]
        .spacing(8);
        if !form.auto_memory {
            memory = memory.push(field(
                "Maximum memory (MB)",
                text_input("2048", &form.max_memory)
                    .on_input(Message::EditMaxMemory)
                    .style(theme::input)
                    .padding(10),
            ));
        }
        column![
            row![
                text(format!("Edit {}", clip(&form.original, 22)))
                    .size(26)
                    .font(BOLD)
                    .width(Length::Fill),
                small_button("Cancel").on_press(Message::ShowHome),
                button(text("Save").size(15).font(BOLD))
                    .padding([10, 26])
                    .style(theme::play)
                    .on_press(Message::SaveEdit),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            field(
                "Name",
                text_input("Instance name", &form.name)
                    .on_input(Message::EditName)
                    .style(theme::input)
                    .padding(10),
            ),
            self.loader_picker(&form.loader, minecraft),
            self.window_section(form),
            self.edit_worlds_section(form),
            memory,
            field(
                "Java executable (leave empty to pick automatically)",
                text_input("Automatic", &form.java_path)
                    .on_input(Message::EditJava)
                    .style(theme::input)
                    .padding(10),
            ),
            field(
                "Extra JVM arguments",
                text_input("e.g. -XX:+UseZGC", &form.jvm_args)
                    .on_input(Message::EditJvmArgs)
                    .style(theme::input)
                    .padding(10),
            ),
        ]
        .spacing(18)
        .into()
    }

    fn accounts_form<'a>(&'a self, name: &'a str) -> Element<'a, Message> {
        let mut list = column![].spacing(8);
        for account in &self.accounts.accounts {
            use nethernode_core::skins::Model;
            let is_default = self.accounts.default.as_ref() == Some(&account.name);
            let skin = self.skins.get(&account.name);
            let face: Element<'_, Message> = match skin.and_then(|s| s.face.clone()) {
                Some(face) => image(face)
                    .width(40)
                    .height(40)
                    .filter_method(image::FilterMethod::Nearest)
                    .into(),
                None => container(text("?").size(18).color(theme::MUTED))
                    .center(40)
                    .style(theme::row_item)
                    .into(),
            };
            let name = account.name.clone();
            let skin_controls: Element<'_, Message> = match skin {
                Some(skin) => row![
                    small_button("Change skin…").on_press(Message::PickSkin(name.clone(), false)),
                    button(text("Classic").size(13))
                        .padding([6, 12])
                        .style(theme::segment(skin.model == Model::Classic))
                        .on_press(Message::SetSkinModel(name.clone(), Model::Classic)),
                    button(text("Slim").size(13))
                        .padding([6, 12])
                        .style(theme::segment(skin.model == Model::Slim))
                        .on_press(Message::SetSkinModel(name.clone(), Model::Slim)),
                    small_button(if skin.cape {
                        "Change cape…"
                    } else {
                        "Add cape…"
                    })
                    .on_press(Message::PickSkin(name.clone(), true)),
                    small_button("Default skin").on_press(Message::RemoveSkin(name.clone())),
                ]
                .spacing(6)
                .wrap()
                .vertical_spacing(6)
                .into(),
                None => row![
                    small_button("Choose skin…").on_press(Message::PickSkin(name.clone(), false))
                ]
                .into(),
            };
            list = list.push(
                container(
                    row![
                        face,
                        column![
                            text(&account.name).size(16).font(BOLD),
                            text(&account.uuid).size(11).font(MONO).color(theme::MUTED),
                            skin_controls,
                        ]
                        .spacing(6)
                        .width(Length::Fill),
                        if is_default {
                            chip("Default", theme::SUCCESS)
                        } else {
                            small_button("Make default")
                                .on_press(Message::SetDefaultAccount(account.name.clone()))
                                .into()
                        },
                        button(text("Remove").size(14))
                            .padding([8, 14])
                            .style(theme::danger_button)
                            .on_press(Message::RemoveAccount(account.name.clone())),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding(12)
                .style(theme::row_item),
            );
        }
        let can_add = !name.trim().is_empty();
        column![
            text("Accounts").size(26).font(BOLD),
            text("Offline accounts only: no login and no internet check. Your name and UUID are what offline servers and LAN worlds see. Skins show in game for you and for friends who join with netherNode.")
                .size(13)
                .color(theme::MUTED),
            list,
            row![
                text_input("Username (3–16 letters, digits or _)", name)
                    .on_input(Message::AccountName)
                    .on_submit_maybe(can_add.then_some(Message::AddAccount))
                    .style(theme::input)
                    .padding(10),
                button(text("Add").font(BOLD)).padding([10, 22]).style(theme::play).on_press_maybe(can_add.then_some(Message::AddAccount)),
            ]
            .spacing(10),
            row![
                space::horizontal(),
                small_button("Done").on_press_maybe((!self.accounts.accounts.is_empty()).then_some(Message::ShowHome)),
            ],
        ]
        .spacing(18)
        .into()
    }

    fn mods_screen<'a>(&'a self, view: &'a ModsView) -> Element<'a, Message> {
        let Some(instance) = self.instance(&view.instance) else {
            return text("Instance not found").into();
        };
        let kind = instance.loader.as_ref().map(|l| l.kind);
        let header = row![
            button(text("← Back").size(14))
                .style(theme::link)
                .on_press(Message::ShowHome),
            text("Mods").size(24).font(BOLD),
            chip(&instance.name, theme::VIOLET_BRIGHT),
            chip(loader_label(instance), loader_color(kind)),
            space::horizontal(),
            small_button("Open folder")
                .on_press(Message::OpenFolder(instance.mods_dir(&self.paths))),
            small_button("Add files…").on_press(Message::AddModFiles),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let supports_mods = kind.is_some_and(|k| !k.mod_loaders().is_empty());
        let content: Element<'a, Message> = if !supports_mods {
            let why = if kind == Some(LoaderKind::OptiFine) {
                "OptiFine instances can't load other mods. Switch to Fabric or Forge to use mods."
            } else {
                "This instance runs plain Minecraft. Mods need a mod loader such as Fabric or Forge."
            };
            container(
                column![
                    text(why).color(theme::MUTED).center(),
                    button(text("Choose a mod loader").font(BOLD))
                        .padding([10, 20])
                        .style(theme::play)
                        .on_press(Message::ShowEdit(instance.name.clone())),
                ]
                .spacing(14)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into()
        } else {
            let tabs = row![
                button(text(format!("Installed ({})", view.installed.len())).size(14))
                    .padding([8, 16])
                    .style(theme::segment(view.tab == ModsTab::Installed))
                    .on_press(Message::ModsTab(ModsTab::Installed)),
                button(text("Browse Modrinth").size(14))
                    .padding([8, 16])
                    .style(theme::segment(view.tab == ModsTab::Browse))
                    .on_press(Message::ModsTab(ModsTab::Browse)),
                space::horizontal(),
                text("Tip: drop .jar files on the window to add them")
                    .size(12)
                    .color(theme::MUTED),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let list = match view.tab {
                ModsTab::Installed => self.installed_mods(view),
                ModsTab::Browse => self.browse_mods(view),
            };
            column![tabs, list].spacing(14).into()
        };

        container(column![header, content].spacing(16))
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::glass)
            .into()
    }

    fn modpacks_screen<'a>(&'a self, view: &'a PacksView) -> Element<'a, Message> {
        let header = row![
            button(text("← Back").size(14))
                .style(theme::link)
                .on_press(Message::ShowHome),
            text("Modpacks").size(24).font(BOLD),
            chip("Modrinth", theme::SUCCESS),
            space::horizontal(),
            button(text("Import .mrpack…").size(14).font(BOLD))
                .padding([8, 16])
                .style(theme::play)
                .on_press(Message::ImportPack),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let search = row![
            text_input(
                "Search modpacks (e.g. fabulously optimized, cobblemon, create)",
                &view.query
            )
            .on_input(Message::PacksQuery)
            .on_submit(Message::PacksSearch)
            .style(theme::input)
            .padding(10),
            button(text("Search").font(BOLD))
                .padding([10, 20])
                .style(theme::play)
                .on_press(Message::PacksSearch),
        ]
        .spacing(10);
        let hint = text(
            "Each modpack becomes a new instance with its own Minecraft version, mod loader, mods and settings. \
             You can also drop a .mrpack file anywhere on the window.",
        )
        .size(12)
        .color(theme::MUTED);

        let results: Element<'a, Message> = match &view.results {
            None | Some(Load::Loading) => container(text("Loading modpacks…").color(theme::MUTED))
                .center(Length::Fill)
                .into(),
            Some(Load::Failed(e)) => {
                container(text(format!("Search failed: {e}")).color(theme::DANGER))
                    .center(Length::Fill)
                    .into()
            }
            Some(Load::Ready(hits)) if hits.is_empty() => {
                container(text("No modpacks found.").color(theme::MUTED))
                    .center(Length::Fill)
                    .into()
            }
            Some(Load::Ready(hits)) => {
                let mut list = column(hits.iter().map(|hit| {
                    let action: Element<'a, Message> = match self.pack_jobs.get(&hit.project_id) {
                        Some(job) => {
                            let (done, total) =
                                job.stage.as_ref().map_or((0, 1), |(_, d, t)| (*d, *t));
                            column![
                                text("Installing…").size(12).color(theme::WARNING),
                                progress_bar(0.0..=total.max(1) as f32, done as f32)
                                    .girth(6)
                                    .style(theme::progress),
                            ]
                            .spacing(4)
                            .width(120)
                            .into()
                        }
                        None => button(text("Install").size(13).font(BOLD))
                            .padding([8, 18])
                            .style(theme::play)
                            .on_press(Message::InstallPack(hit.project_id.clone()))
                            .into(),
                    };
                    container(
                        row![
                            column![
                                row![
                                    text(&hit.title).size(15).font(BOLD),
                                    text(format!("by {}", hit.author))
                                        .size(12)
                                        .color(theme::MUTED),
                                ]
                                .spacing(8)
                                .align_y(Alignment::Center),
                                text(&hit.description).size(13).color(theme::MUTED),
                                text(format!("{} downloads", short_count(hit.downloads)))
                                    .size(11)
                                    .color(theme::VIOLET_BRIGHT),
                            ]
                            .spacing(4)
                            .width(Length::Fill),
                            action,
                        ]
                        .spacing(14)
                        .align_y(Alignment::Center),
                    )
                    .padding([12, 14])
                    .style(theme::row_item)
                    .into()
                }))
                .spacing(8);
                if (hits.len() as u64) < view.total {
                    list = list.push(
                        container(small_button("Load more").on_press(Message::PacksMore))
                            .center_x(Length::Fill),
                    );
                }
                scrollable(list)
                    .style(theme::scroll)
                    .height(Length::Fill)
                    .into()
            }
        };
        container(column![header, search, hint, results].spacing(14))
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::glass)
            .into()
    }

    fn installed_mods<'a>(&'a self, view: &'a ModsView) -> Element<'a, Message> {
        if view.installed.is_empty() {
            return container(
                column![
                    text("No mods yet.").size(16),
                    text("Find some in Browse Modrinth, or drop .jar files onto this window.")
                        .color(theme::MUTED),
                    button(text("Browse Modrinth").font(BOLD))
                        .padding([10, 20])
                        .style(theme::play)
                        .on_press(Message::ModsTab(ModsTab::Browse)),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into();
        }
        let rows = view.installed.iter().map(|m| {
            let file = m.file_name.clone();
            let version = m
                .info
                .as_ref()
                .map_or(String::new(), |i| format!("{} · ", i.version_number));
            container(
                row![
                    checkbox(m.enabled).on_toggle(move |on| Message::ToggleMod(file.clone(), on)),
                    column![
                        text(m.display_name())
                            .size(15)
                            .font(BOLD)
                            .color(if m.enabled { theme::TEXT } else { theme::MUTED }),
                        text(format!("{version}{}", m.file_name))
                            .size(12)
                            .color(theme::MUTED),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    button(text("Remove").size(13))
                        .padding([6, 12])
                        .style(theme::danger_button)
                        .on_press(Message::RemoveMod(m.file_name.clone())),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding([10, 14])
            .style(theme::row_item)
            .into()
        });
        scrollable(column(rows).spacing(8))
            .style(theme::scroll)
            .height(Length::Fill)
            .into()
    }

    fn browse_mods<'a>(&'a self, view: &'a ModsView) -> Element<'a, Message> {
        let search = row![
            text_input("Search Modrinth (e.g. sodium, jei, create)", &view.query)
                .on_input(Message::ModsQuery)
                .on_submit(Message::ModsSearch)
                .style(theme::input)
                .padding(10),
            button(text("Search").font(BOLD))
                .padding([10, 20])
                .style(theme::play)
                .on_press(Message::ModsSearch),
        ]
        .spacing(10);

        let results: Element<'a, Message> = match &view.results {
            None | Some(Load::Loading) => {
                container(text("Searching Modrinth…").color(theme::MUTED))
                    .center(Length::Fill)
                    .into()
            }
            Some(Load::Failed(e)) => {
                container(text(format!("Search failed: {e}")).color(theme::DANGER))
                    .center(Length::Fill)
                    .into()
            }
            Some(Load::Ready(hits)) if hits.is_empty() => container(
                text("No mods found for this loader and Minecraft version.").color(theme::MUTED),
            )
            .center(Length::Fill)
            .into(),
            Some(Load::Ready(hits)) => {
                let installed: std::collections::HashSet<&str> = view
                    .installed
                    .iter()
                    .filter_map(|m| m.info.as_ref())
                    .map(|i| i.project_id.as_str())
                    .collect();
                let mut list = column(hits.iter().map(|hit| {
                    let action: Element<'a, Message> =
                        if installed.contains(hit.project_id.as_str()) {
                            chip("Installed ✓", theme::SUCCESS)
                        } else if view.installing.contains(&hit.project_id) {
                            button(text("Installing…").size(13))
                                .padding([8, 16])
                                .style(theme::play)
                                .into()
                        } else {
                            button(text("Install").size(13).font(BOLD))
                                .padding([8, 18])
                                .style(theme::play)
                                .on_press(Message::InstallMod(hit.project_id.clone()))
                                .into()
                        };
                    container(
                        row![
                            column![
                                row![
                                    text(&hit.title).size(15).font(BOLD),
                                    text(format!("by {}", hit.author))
                                        .size(12)
                                        .color(theme::MUTED),
                                ]
                                .spacing(8)
                                .align_y(Alignment::Center),
                                text(&hit.description).size(13).color(theme::MUTED),
                                text(format!("{} downloads", short_count(hit.downloads)))
                                    .size(11)
                                    .color(theme::VIOLET_BRIGHT),
                            ]
                            .spacing(4)
                            .width(Length::Fill),
                            action,
                        ]
                        .spacing(14)
                        .align_y(Alignment::Center),
                    )
                    .padding([12, 14])
                    .style(theme::row_item)
                    .into()
                }))
                .spacing(8);
                if (hits.len() as u64) < view.total {
                    list = list.push(
                        container(small_button("Load more").on_press(Message::ModsMore))
                            .center_x(Length::Fill),
                    );
                }
                scrollable(list)
                    .style(theme::scroll)
                    .height(Length::Fill)
                    .into()
            }
        };
        column![search, results]
            .spacing(14)
            .height(Length::Fill)
            .into()
    }
}

/// Plain-text crash summary for pasting into a bug report or chat.
pub fn crash_text(instance: &str, crash: &CrashInfo) -> String {
    let mut out = format!(
        "netherNode {}: {instance}\n{}\n",
        env!("CARGO_PKG_VERSION"),
        crash.title
    );
    for (label, value) in [
        ("Cause", crash.cause.clone()),
        ("Exit code", crash.exit_code.map(|c| c.to_string())),
        (
            "Report",
            crash.report.as_ref().map(|p| p.display().to_string()),
        ),
        ("Hint", crash.hint.clone()),
    ] {
        if let Some(value) = value {
            out.push_str(&format!("{label}: {value}\n"));
        }
    }
    out
}

fn view_crash<'a>(name: &str, crash: &'a CrashInfo) -> Element<'a, Message> {
    let mut details =
        column![text(&crash.title).size(17).font(BOLD).color(theme::DANGER)].spacing(8);
    if let Some(cause) = &crash.cause {
        details = details.push(text(cause).size(12).font(MONO).color(theme::MUTED));
    }
    if let Some(hint) = &crash.hint {
        details = details.push(text(hint).size(14));
    }
    let mut actions = row![].spacing(8);
    if let Some(report) = &crash.report {
        actions = actions
            .push(small_button("Open crash report").on_press(Message::OpenFile(report.clone())));
    }
    actions = actions
        .push(small_button("Copy details").on_press(Message::CopyCrash(name.to_string())))
        .push(space::horizontal())
        .push(
            button(text("Dismiss").size(14))
                .style(theme::link)
                .on_press(Message::DismissCrash(name.to_string())),
        );
    container(details.push(actions))
        .padding(16)
        .width(Length::Fill)
        .style(theme::notice(true))
        .into()
}

/// Worlds picked in the New instance form, imported once it's created.
fn new_worlds_section(form: &NewInstanceForm) -> Element<'_, Message> {
    let picked = form.worlds.iter().enumerate().map(|(i, path)| {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        row![
            text(name).size(13).width(Length::Fill),
            button(text("×").size(15))
                .style(theme::link)
                .on_press(Message::RemoveNewWorld(i)),
        ]
        .align_y(Alignment::Center)
        .into()
    });
    column![
        text("Worlds to bring in (optional)")
            .size(13)
            .color(theme::MUTED),
        column(picked).spacing(4),
        row![
            small_button("Add world folder…").on_press(Message::NewAddWorlds(false)),
            small_button("Add world .zip…").on_press(Message::NewAddWorlds(true)),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .into()
}

fn join_friend_form<'a>(code: &'a str, error: Option<&'a str>) -> Element<'a, Message> {
    let can_join = !code.trim().is_empty();
    let mut col = column![
        row![
            text("Join a friend").size(26).font(BOLD),
            space::horizontal(),
            small_button("Cancel").on_press(Message::ShowHome),
            button(text("Join").size(15).font(BOLD))
                .padding([10, 26])
                .style(theme::play)
                .on_press_maybe(can_join.then_some(Message::StartJoin)),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        text("Type the four words your friend got from \"Invite friends\" (or paste their full code). No port forwarding needed: netherNode connects directly when it can, otherwise through an encrypted relay.")
            .size(13)
            .color(theme::MUTED),
        text_input("e.g. maple orbit tunnel frog", code)
            .on_input(Message::JoinCodeInput)
            .on_submit_maybe(can_join.then_some(Message::StartJoin))
            .style(theme::input)
            .padding(10),
    ]
    .spacing(16);
    if let Some(error) = error {
        col = col.push(text(error).size(13).color(theme::DANGER));
    }
    col.push(
        text("Then start any instance with the same Minecraft version (and mods) as your friend, and pick the world under LAN in Multiplayer.")
            .size(12)
            .color(theme::MUTED),
    )
    .into()
}

/// Full-page server settings: netherNode's options for the server, then every
/// server.properties setting by group, with the right control for each.
fn server_settings<'a>(
    form: &'a crate::ServerSettingsForm,
    server: Option<&'a nethernode_core::server::Server>,
    running: bool,
) -> Element<'a, Message> {
    use nethernode_core::serverprops::{self, Group, Kind};
    let save_row = || {
        row![
            space::horizontal(),
            small_button("Cancel").on_press(Message::CloseServerSettings),
            button(text("Save").size(15).font(BOLD))
                .padding([10, 26])
                .style(theme::play)
                .on_press(Message::SaveServerSettings),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
    };
    let help = |t: &'a str| text(t).size(12).color(theme::MUTED);
    let section = |title: &'a str| text(title).size(18).font(BOLD).color(theme::VIOLET_BRIGHT);

    let mut col = column![
        row![
            text(format!("Settings: {}", clip(&form.server, 22)))
                .size(26)
                .font(BOLD)
                .width(Length::Fill),
            small_button("Cancel").on_press(Message::CloseServerSettings),
            button(text("Save").size(15).font(BOLD))
                .padding([10, 26])
                .style(theme::play)
                .on_press(Message::SaveServerSettings),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    ]
    .spacing(16);
    if let Some(server) = server {
        col = col.push(
            text(format!(
                "{} server · Minecraft {} · port {}",
                server.kind.name(),
                server.minecraft,
                server.port
            ))
            .size(13)
            .color(theme::MUTED),
        );
    }
    if running {
        col = col.push(
            text("The server is running: changes apply the next time it starts.")
                .size(13)
                .color(theme::WARNING),
        );
    }

    // netherNode's own options for this server.
    col = col.push(section("This server in netherNode"));
    col = col.push(field(
        "Memory limit in MB (empty = automatic)",
        text_input("automatic", &form.max_memory)
            .on_input(|v| Message::ServerSettingsInput(ServerField::MaxMemory, v))
            .style(theme::input)
            .padding(8),
    ));
    col = col.push(
        column![
            checkbox(form.skins)
                .label("Show players' skins")
                .on_toggle(|on| Message::ServerSettingsToggle(ServerField::Skins, on)),
            help("Players then need netherNode (with skins on) to join. Turn off to let any offline launcher in."),
        ]
        .spacing(4),
    );
    col = col.push(
        column![
            field(
                "Extra ports for friends",
                text_input("e.g. UDP 24454, TCP 8100", &form.extra_ports)
                    .on_input(|v| Message::ServerSettingsInput(ServerField::ExtraPorts, v))
                    .style(theme::input)
                    .padding(8),
            ),
            help("For mods that need their own port. Simple Voice Chat is found automatically."),
        ]
        .spacing(4),
    );
    let mut whitelist = column![
        checkbox(form.whitelist_on)
            .label("Only allow these players (whitelist)")
            .on_toggle(|on| Message::ServerSettingsToggle(ServerField::WhitelistOn, on)),
    ]
    .spacing(6);
    if form.whitelist_on {
        whitelist = whitelist.push(
            text_input("Steve, Alex", &form.whitelist)
                .on_input(|v| Message::ServerSettingsInput(ServerField::Whitelist, v))
                .style(theme::input)
                .padding(8),
        );
    }
    col = col.push(whitelist);

    // One control per known setting.
    let control = |key: &'a str,
                   label: &'a str,
                   about: &'a str,
                   kind: Kind,
                   value: &'a str|
     -> Element<'a, Message> {
        let key_owned = key.to_string();
        match kind {
            Kind::Bool => column![
                checkbox(value == "true").label(label).on_toggle(
                    move |on| Message::ServerProperty(key_owned.clone(), on.to_string())
                ),
                help(about),
            ]
            .spacing(4)
            .into(),
            Kind::Choice(choices) => column![
                text(label).size(13).color(theme::MUTED),
                row(choices.iter().map(|c| {
                    button(text(*c).size(13))
                        .padding([6, 12])
                        .style(theme::segment(value == *c))
                        .on_press(Message::ServerProperty(key.to_string(), c.to_string()))
                        .into()
                }))
                .spacing(6)
                .wrap()
                .vertical_spacing(6),
                help(about),
            ]
            .spacing(6)
            .into(),
            Kind::Number { .. } | Kind::Text => column![
                text(label).size(13).color(theme::MUTED),
                text_input("", value)
                    .on_input(move |v| Message::ServerProperty(key_owned.clone(), v))
                    .style(theme::input)
                    .padding(8),
                help(about),
            ]
            .spacing(4)
            .into(),
        }
    };
    for group in Group::ALL {
        if group == Group::Advanced && !form.show_advanced {
            continue;
        }
        let settings: Vec<_> = serverprops::SETTINGS
            .iter()
            .filter(|s| s.group == group)
            .filter_map(|s| form.values.get(s.key).map(|v| (s, v)))
            .collect();
        if settings.is_empty() {
            continue;
        }
        col = col.push(section(group.title()));
        for (s, value) in settings {
            col = col.push(control(s.key, s.label, s.help, s.kind, value));
        }
    }
    col = col.push(
        button(
            text(if form.show_advanced {
                "▾ Hide advanced settings"
            } else {
                "▸ Show advanced settings (RCON, query, and everything else in server.properties)"
            })
            .size(13),
        )
        .style(theme::link)
        .on_press(Message::ToggleAdvancedServerSettings),
    );
    if form.show_advanced {
        // Settings the server has that netherNode doesn't describe (newer versions, Paper,
        // mods): plain text.
        let others: Vec<_> = form
            .values
            .iter()
            .filter(|(k, _)| serverprops::find(k).is_none())
            .collect();
        if !others.is_empty() {
            col = col.push(section("Other"));
            for (key, value) in others {
                let key_owned = key.clone();
                col = col.push(
                    row![
                        text(key.as_str())
                            .size(12)
                            .font(MONO)
                            .width(Length::FillPortion(2)),
                        text_input("", value)
                            .on_input(move |v| Message::ServerProperty(key_owned.clone(), v))
                            .style(theme::input)
                            .size(13)
                            .padding(6)
                            .width(Length::FillPortion(3)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
        }
    }
    col.push(save_row()).into()
}
