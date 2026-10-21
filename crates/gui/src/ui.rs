//! Everything the window shows: top bar, instance grid, details and console, the play bar,
//! and the forms and Mods screen that replace the grid when open.

use iced::widget::image::FilterMethod;
use iced::widget::{
    button, checkbox, column, combo_box, container, image, mouse_area, pick_list, progress_bar,
    row, scrollable, shader, space, stack, text, text_input,
};
use iced::{Alignment, Color, Element, Font, Length};
use nethernode_core::crash::CrashInfo;
use nethernode_core::instance::Instance;
use nethernode_core::loader::LoaderKind;

use crate::background::Portal;
use crate::theme;
use crate::{App, EditForm, Load, LoaderPick, Message, ModsTab, ModsView, NewInstanceForm, Screen};

/// Obsidian block size in the background, logical px. Content sits inside this frame.
const BLOCK: f32 = 40.0;
/// Lines actually laid out in the console; laying out thousands of lines every frame is slow.
const LOG_VISIBLE: usize = 400;

const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..Font::DEFAULT
};

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

fn small_button(label: &str) -> button::Button<'_, Message> {
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
        };
        let mut content = column![self.top_bar()].spacing(14);
        if let Some((is_error, msg)) = &self.notice {
            content = content.push(
                container(
                    row![
                        text(msg).width(Length::Fill),
                        button(text("✕").size(12))
                            .style(theme::link)
                            .on_press(Message::DismissNotice),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding([10, 16])
                .style(theme::notice(*is_error)),
            );
        }
        content = content.push(body).push(self.play_bar());

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
            stack![
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
        if self.gpu {
            let label = if self.prefs.animated_background {
                "Animation on"
            } else {
                "Animation off"
            };
            right = right.push(small_button(label).on_press(Message::ToggleAnimation));
        }
        right = right
            .push(small_button("Accounts").on_press(Message::ShowAccounts))
            .push(
                small_button("Data folder").on_press(Message::OpenFolder(self.paths.root.clone())),
            );
        container(
            row![
                image(self.logo.clone()).width(36).height(36),
                column![
                    text("netherNode")
                        .size(22)
                        .font(BOLD)
                        .color(theme::VIOLET_BRIGHT),
                    text("Offline Minecraft launcher")
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
            None => {
                let initial = instance
                    .name
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string();
                container(text(initial).size(size * 0.45).font(BOLD))
                    .center(size)
                    .style(theme::chip(loader_color(
                        instance.loader.as_ref().map(|l| l.kind),
                    )))
                    .into()
            }
        }
    }

    fn instances_panel(&self) -> Element<'_, Message> {
        let mut cards = row![].spacing(12);
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
                    text(&instance.name)
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
        let header = row![
            self.instance_icon(instance, 64.0),
            column![
                text(&instance.name).size(24).font(BOLD),
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
                .spacing(6),
                text(memory).size(12).color(theme::MUTED),
            ]
            .spacing(6)
            .width(Length::Fill),
        ]
        .spacing(16)
        .align_y(Alignment::Center);

        let actions = row![
            small_button("Mods").on_press(Message::ShowMods(name.clone())),
            small_button("Edit").on_press_maybe((!busy).then(|| Message::ShowEdit(name.clone()))),
            small_button("Folder").on_press(Message::OpenFolder(instance.game_dir(&self.paths))),
            space::horizontal(),
            button(text(if deleting { "Really delete?" } else { "Delete" }).size(14))
                .padding([8, 14])
                .style(theme::danger_button)
                .on_press_maybe((!busy).then(|| Message::Delete(name.clone()))),
        ]
        .spacing(8);

        let mut body = column![header, actions].spacing(14);
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
                        .map(|line| text(line).size(12).font(Font::MONOSPACE).into()),
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
        let target: Element<'_, Message> = match selected {
            Some(i) => column![
                text("INSTANCE").size(11).color(theme::MUTED),
                text(&i.name).size(17).font(BOLD),
                text(format!(
                    "Minecraft {} · {}",
                    i.minecraft_version(),
                    loader_label(i)
                ))
                .size(12)
                .color(theme::MUTED),
            ]
            .spacing(2)
            .into(),
            None => text("No instance selected").color(theme::MUTED).into(),
        };

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
        let options = [None]
            .into_iter()
            .chain(LoaderKind::ALL.into_iter().map(Some))
            .map(|kind| {
                let label = kind.map_or("Vanilla", |k| k.name());
                button(text(label).size(14))
                    .padding([8, 14])
                    .style(theme::segment(pick.kind == kind))
                    .on_press(Message::PickLoader(kind))
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
            Some(LoaderKind::OptiFine) => {
                "Graphics and performance tweaks with shader support (no other mods)."
            }
        };
        let mut section = column![
            text("Mod loader").size(13).color(theme::MUTED),
            segments,
            text(about).size(12).color(theme::MUTED)
        ]
        .spacing(8);

        match pick.kind {
            None => {}
            Some(LoaderKind::OptiFine) => {
                let mc = minecraft.unwrap_or("this version");
                let chosen = pick
                    .optifine_jar
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned());
                section = section.push(
                    container(
                        column![
                            text("OptiFine's author doesn't allow automatic downloads, so it takes two steps:").size(13),
                            row![
                                text(format!("1. Download OptiFine for Minecraft {mc}")).size(13).width(Length::Fill),
                                small_button("Open optifine.net")
                                    .on_press(Message::OpenUrl("https://optifine.net/downloads".into())),
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
                            text("Prefer one click? Fabric + Sodium + Iris gives similar speed and shaders.")
                                .size(12)
                                .color(theme::MUTED),
                        ]
                        .spacing(10),
                    )
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
        let versions: Element<'a, Message> = if self.versions.is_some() {
            combo_box(
                &form.choices,
                "Search versions…",
                form.version.as_ref(),
                Message::NewVersion,
            )
            .input_style(theme::input)
            .padding(10)
            .into()
        } else {
            text("Loading versions…").color(theme::MUTED).into()
        };
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
            checkbox(form.snapshots)
                .label("Show snapshots")
                .on_toggle(Message::NewSnapshots),
            self.loader_picker(&form.loader, form.version.as_deref()),
            checkbox(form.low_end)
                .label("Low-end graphics defaults (render distance 6, 60 FPS cap, fast graphics)")
                .on_toggle(Message::NewLowEnd),
        ]
        .spacing(18)
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
                text(format!("Edit {}", form.original)).size(26).font(BOLD),
                space::horizontal(),
                small_button("Cancel").on_press(Message::ShowHome),
                button(text("Save").size(15).font(BOLD))
                    .padding([10, 26])
                    .style(theme::play)
                    .on_press(Message::SaveEdit),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            self.loader_picker(&form.loader, minecraft),
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
            let is_default = self.accounts.default.as_ref() == Some(&account.name);
            list = list.push(
                container(
                    row![
                        column![
                            text(&account.name).size(16).font(BOLD),
                            text(&account.uuid)
                                .size(11)
                                .font(Font::MONOSPACE)
                                .color(theme::MUTED),
                        ]
                        .spacing(2)
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
            text("Offline accounts only: no login and no internet check. Your name and UUID are what offline servers and LAN worlds see.")
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
        details = details.push(
            text(cause)
                .size(12)
                .font(Font::MONOSPACE)
                .color(theme::MUTED),
        );
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
