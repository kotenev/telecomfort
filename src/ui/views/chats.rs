//! Чаты как в официальном клиенте: диалоги с аватарами и непрочитанными,
//! пузыри сообщений (исходящие подсвечены), reply/edit/delete, реакции,
//! поиск по истории, композер с черновиками, прикрепление файлов.
//! Медиа-воронка Telecomfort (скачивание/Яндекс/Calibre) сохранена целиком.

use crate::config::{AutodownloadRule, MediaKind};
use crate::ui::app::{format_msg_time, TelecomfortApp};

/// Быстрые реакции палитры (повтор той же сервером убирается — toggle).
const QUICK_REACTIONS: [&str; 4] = ["👍", "❤️", "🔥", "🎉"];

pub fn show(app: &mut TelecomfortApp, ui: &mut egui::Ui) {
    egui::Panel::left("chats_list")
        .resizable(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Диалоги");
                if ui.small_button("↻").on_hover_text("Обновить").clicked() {
                    app.refresh_dialogs();
                }
                if ui
                    .small_button("＋")
                    .on_hover_text("Создать группу/канал")
                    .clicked()
                {
                    app.create_chat_open = true;
                }
            });
            // Глобальный поиск по всем чатам (серверный).
            ui.horizontal(|ui| {
                ui.label("🌍");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut app.gsearch)
                        .hint_text("глобальный поиск…")
                        .desired_width(160.0),
                );
                let go = ui.small_button("🔍").clicked()
                    || (resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if go {
                    app.do_global_search();
                }
            });
            if !app.gsearch_results.is_empty() {
                let count = app.gsearch_results.len();
                ui.label(
                    egui::RichText::new(format!("найдено: {count} (клик — открыть чат)"))
                        .small()
                        .weak(),
                );
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (cid, m) in app.gsearch_results.clone() {
                        let title = app
                            .dialogs
                            .iter()
                            .find(|d| d.id == cid)
                            .map(|d| d.title.clone())
                            .unwrap_or_else(|| format!("chat {cid}"));
                        if ui
                            .selectable_label(
                                false,
                                format!("{}: {}", title, crate::utils::truncate(&m.text, 60)),
                            )
                            .clicked()
                        {
                            app.select_chat(cid);
                        }
                    }
                });
                ui.separator();
            }
            // Папки сервера + Архив (как вкладки в официальном клиенте).
            ui.horizontal_wrapped(|ui| {
                let all_sel = app.folder_sel.is_none();
                if ui.selectable_label(all_sel, "Все").clicked() {
                    app.folder_sel = None;
                }
                for f in app.folders.clone() {
                    let sel = app.folder_sel == Some(f.id);
                    if ui.selectable_label(sel, &f.title).clicked() {
                        app.folder_sel = Some(f.id);
                    }
                }
                let arch_sel = app.folder_sel == Some(-1);
                if ui.selectable_label(arch_sel, "📦 Архив").clicked() {
                    app.folder_sel = Some(-1);
                }
            });
            ui.separator();
            let ctx = ui.ctx().clone();
            egui::ScrollArea::vertical().show(ui, |ui| {
                let folder_sel = app.folder_sel;
                let peer_set: Option<std::collections::HashSet<i64>> = folder_sel
                    .and_then(|fid| {
                        if fid == -1 {
                            None
                        } else {
                            app.folders.iter().find(|f| f.id == fid).map(|f| {
                                f.peer_ids.iter().copied().collect()
                            })
                        }
                    });
                let mut shown = 0;
                for d in app.dialogs.clone() {
                    // Фильтр вида: Все — без архива; Архив — только архив; папка — её пиры.
                    if let Some(fid) = folder_sel {
                        if fid == -1 {
                            if !d.archived {
                                continue;
                            }
                        } else if d.archived {
                            continue;
                        }
                    } else if d.archived {
                        continue;
                    }
                    if let Some(set) = &peer_set {
                        if !set.contains(&d.id) {
                            continue;
                        }
                    }
                    shown += 1;
                    let sel = app.selected_chat == Some(d.id);
                    ui.horizontal(|ui| {
                        let tex = app.ensure_avatar(&ctx, d.id, &d.title);
                        ui.image((tex.id(), egui::Vec2::new(36.0, 36.0)));
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                if ui.selectable_label(sel, &d.title).clicked() {
                                    app.select_chat(d.id);
                                }
                                if d.unread > 0 {
                                    ui.label(
                                        egui::RichText::new(format!("{}", d.unread))
                                            .small()
                                            .strong()
                                            .color(egui::Color32::from_rgb(220, 70, 60)),
                                    );
                                }
                                if d.muted {
                                    ui.label(
                                        egui::RichText::new("🔇").small().weak(),
                                    );
                                }
                                if d.archived {
                                    ui.label(
                                        egui::RichText::new("📦").small().weak(),
                                    );
                                }
                            });
                            if let Some(u) = &d.username {
                                ui.label(
                                    egui::RichText::new(format!("@{u}")).small().weak(),
                                );
                            }
                        });
                    });
                    if d.protected {
                        ui.label(
                            egui::RichText::new("защита от копирования · качаем напрямую")
                                .small()
                                .weak(),
                        );
                    }
                }
                if app.dialogs.is_empty() {
                    ui.label("пока пусто — нажмите ↻");
                } else if shown == 0 {
                    ui.label(
                        egui::RichText::new("в этом виде пусто — смените папку/фильтр")
                            .small()
                            .weak(),
                    );
                }
            });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(chat_id) = app.selected_chat {
            let (title, username, protected, muted, archived) = app
                .dialogs
                .iter()
                .find(|d| d.id == chat_id)
                .map(|d| (d.title.clone(), d.username.clone(), d.protected, d.muted, d.archived))
                .unwrap_or_else(|| (format!("chat {chat_id}"), None, false, false, false));
            // Шапка: аватар + название + username.
            ui.horizontal(|ui| {
                let ctx = ui.ctx().clone();
                let tex = app.ensure_avatar(&ctx, chat_id, &title);
                ui.image((tex.id(), egui::Vec2::new(40.0, 40.0)));
                ui.vertical(|ui| {
                    ui.heading(title.clone());
                    if let Some(u) = username {
                        ui.label(egui::RichText::new(format!("@{u}")).small().weak());
                    }
                });
            });
            ui.label(
                egui::RichText::new(
                    "Файлы/видео/аудио из закрытых групп скачиваются напрямую через MTProto (флаг noforwards лишь прячет кнопки в официальных клиентах).",
                )
                .small()
                .weak(),
            );
            if protected {
                ui.label(
                    egui::RichText::new("🔒 В этом чате запрещено копирование — Telecomfort качает напрямую по file reference.")
                        .small(),
                );
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("⬇⬇ Скачать всё медиа (авто: локально → Яндекс → Calibre)").clicked() {
                    app.download_all_matching(chat_id, title.clone());
                }
                if ui.small_button("↻ Обновить историю").clicked() {
                    app.load_history(chat_id);
                }
                // Управление чатом как в официальном клиенте.
                let mute_label = if muted { "🔈 Разглушить" } else { "🔇 Мьют" };
                if ui.small_button(mute_label).clicked() {
                    app.toggle_mute(chat_id);
                }
                let arch_label = if archived { "📤 Из архива" } else { "📦 В архив" };
                if ui.small_button(arch_label).clicked() {
                    app.toggle_archive(chat_id);
                }
                // Модерация и управление: блок, инвайт, очистка, выход, запланированные.
                if ui.small_button("🚫").on_hover_text("Заблокировать пользователя").clicked() {
                    app.toggle_block(chat_id, false);
                }
                if ui.small_button("✅").on_hover_text("Разблокировать пользователя").clicked() {
                    app.toggle_block(chat_id, true);
                }
                if ui.small_button("🔗").on_hover_text("Пригласительная ссылка").clicked() {
                    app.load_invite_link(chat_id);
                }
                if ui.small_button("🧹").on_hover_text("Очистить историю").clicked() {
                    app.confirm_action = Some(
                        crate::ui::app::ConfirmAction::ClearHistory { chat_id, revoke: false },
                    );
                }
                if ui.small_button("🚪").on_hover_text("Покинуть канал/супергруппу").clicked() {
                    app.confirm_action = Some(
                        crate::ui::app::ConfirmAction::LeaveChannel { chat_id },
                    );
                }
                if ui.small_button("⏰").on_hover_text("Запланированные сообщения").clicked() {
                    app.load_scheduled(chat_id);
                }
                if ui
                    .small_button("+ Правило для этого чата")
                    .on_hover_text("Добавить per-chat правило автозагрузки")
                    .clicked()
                {
                    ensure_rule_for_chat(app, &title, chat_id);
                }
            });
            // Закреплённое сообщение — баннер под шапкой.
            if let Some(pinned) = app.pinned.get(&chat_id).cloned() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "📌 {}",
                            crate::utils::truncate(&pinned.text, 90)
                        ))
                        .small()
                        .strong(),
                    );
                    if ui.small_button("✕").on_hover_text("Открепить").clicked() {
                        app.toggle_pin(chat_id, pinned.id, true);
                    }
                });
            }
            // Топики форума: вкладки под шапкой (пусто — не форум).
            if !app.topics.is_empty() || app.topic_sel.is_some() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("🧵");
                    let all_sel = app.topic_sel.is_none();
                    if ui.selectable_label(all_sel, "Все").clicked() {
                        app.select_topic(chat_id, None);
                    }
                    for t in app.topics.clone() {
                        let sel = app.topic_sel == Some(t.id);
                        if ui.selectable_label(sel, &t.title).clicked() {
                            app.select_topic(chat_id, Some(t.id));
                        }
                    }
                    if ui.small_button("＋").on_hover_text("Создать топик").clicked() {
                        app.topic_create_open = true;
                    }
                });
            }
            // Подсказка: какое правило сработает
            ui.label(
                egui::RichText::new(format!(
                    "правил: {} · это чат: «{title}» / id {chat_id} / «*»",
                    app.cfg.downloader.rules.len()
                ))
                .small()
                .weak(),
            );
            // Поиск по загруженной истории.
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.add(
                    egui::TextEdit::singleline(&mut app.chat_search)
                        .hint_text("поиск по истории…")
                        .desired_width(220.0),
                );
            });
            ui.separator();
            let q = app.chat_search.to_lowercase();
            let items: Vec<crate::telegram::MessageInfo> = app
                .history
                .get(&chat_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| {
                    q.is_empty()
                        || m.text.to_lowercase().contains(&q)
                        || m.file_name.as_deref().unwrap_or("").to_lowercase().contains(&q)
                })
                .collect();
            // Подсветка правил — как раньше (медиа-воронка Telecomfort).
            let rules = app.cfg.downloader.clone();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for m in items {
                    message_bubble(app, ui, chat_id, &title, &rules, &m);
                }
                if app.history.get(&chat_id).map(|v| v.is_empty()).unwrap_or(true) {
                    ui.label("история пуста");
                }
            });
            ui.separator();
            // Выбор цели пересылки — окно со списком диалогов.
            if app.forward_pick.is_some() {
                let dialogs = app.dialogs.clone();
                egui::Window::new("➦ Куда переслать?")
                    .collapsible(false)
                    .show(ui.ctx(), |ui| {
                        ui.label(
                            egui::RichText::new("Выберите целевой чат:")
                                .small()
                                .weak(),
                        );
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for d in dialogs {
                                if ui.selectable_label(false, &d.title).clicked() {
                                    app.do_forward(d.id);
                                }
                            }
                        });
                        if ui.small_button("Отмена").clicked() {
                            app.forward_pick = None;
                        }
                    });
            }
            composer(app, ui, chat_id);
            // Окна: создание топика/канала, подтверждение, инвайт, запланированные.
            if app.topic_create_open {
                egui::Window::new("🧵 Новый топик").collapsible(false).show(ui.ctx(), |ui| {
                    ui.label("Название топика:");
                    ui.text_edit_singleline(&mut app.topic_create_title);
                    ui.horizontal(|ui| {
                        if ui.button("Создать").clicked() {
                            app.do_create_topic(chat_id);
                        }
                        if ui.button("Отмена").clicked() {
                            app.topic_create_open = false;
                            app.topic_create_title.clear();
                        }
                    });
                });
            }
            if let Some(link) = app.invite_link.clone() {
                egui::Window::new("🔗 Пригласительная ссылка").collapsible(false).show(
                    ui.ctx(),
                    |ui| {
                        ui.label("Поделитесь ссылкой (выделите и скопируйте):");
                        ui.label(egui::RichText::new(link).strong());
                        if ui.button("Закрыть").clicked() {
                            app.invite_link = None;
                        }
                    },
                );
            }
            // Подтверждения опасных действий — глобальное окно в app.rs (работает с любого экрана).
            if app.scheduled_open {
                egui::Window::new("⏰ Запланированные").collapsible(false).show(ui.ctx(), |ui| {
                    if app.scheduled.is_empty() {
                        ui.label("нет запланированных сообщений");
                    }
                    for m in app.scheduled.clone() {
                        ui.horizontal(|ui| {
                            ui.label(crate::utils::truncate(&m.text, 60));
                            if ui.small_button("✕").on_hover_text("Отменить").clicked() {
                                app.toggle_scheduled_cancel(chat_id, m.id);
                            }
                        });
                    }
                    if ui.button("Закрыть").clicked() {
                        app.scheduled_open = false;
                    }
                });
            }
            if app.create_chat_open {
                egui::Window::new("＋ Новая группа/канал").collapsible(false).show(
                    ui.ctx(),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Название:");
                            ui.text_edit_singleline(&mut app.create_chat_title);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Описание:");
                            ui.text_edit_singleline(&mut app.create_chat_about);
                        });
                        ui.checkbox(&mut app.create_chat_broadcast, "Канал (иначе — группа)");
                        ui.horizontal(|ui| {
                            if ui.button("Создать").clicked() {
                                app.do_create_channel();
                            }
                            if ui.button("Отмена").clicked() {
                                app.create_chat_open = false;
                            }
                        });
                    },
                );
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Выберите диалог слева");
            });
        }
    });
}

/// Один пузырь сообщения: отправитель/время, текст, реакции, медиа, действия.
fn message_bubble(
    app: &mut TelecomfortApp,
    ui: &mut egui::Ui,
    chat_id: i64,
    title: &str,
    rules: &crate::config::DownloaderSettings,
    m: &crate::telegram::MessageInfo,
) {
    let kind = crate::media::AutoDownloader::classify(
        m.file_name.as_deref(),
        m.media_kind.as_deref(),
    );
    let size = m.file_size.unwrap_or(0).max(0) as u64;
    let matched = crate::media::AutoDownloader::matches_rules(rules, title, &kind, size)
        || crate::media::AutoDownloader::matches_rules(rules, &chat_id.to_string(), &kind, size);
    // Исходящие — акцентной заливкой (темозависимой), входящие — обычный group.
    if m.outgoing {
        let fill = ui.visuals().selection.bg_fill;
        egui::Frame::group(ui.style()).fill(fill).show(ui, |ui| {
            bubble_content(app, ui, chat_id, matched, m);
        });
    } else {
        ui.group(|ui| {
            bubble_content(app, ui, chat_id, matched, m);
        });
    }
}

fn bubble_content(
    app: &mut TelecomfortApp,
    ui: &mut egui::Ui,
    chat_id: i64,
    matched: bool,
    m: &crate::telegram::MessageInfo,
) {
    // Шапка пузыря: отправитель (входящие в группах) + время + «изм.».
    ui.horizontal(|ui| {
        ui.label(format!("#{}", m.id));
        if !m.outgoing {
            if let Some(s) = &m.sender {
                ui.label(egui::RichText::new(s).strong());
            }
        } else {
            ui.label(egui::RichText::new("вы").small().weak());
        }
        let time = format_msg_time(m.date);
        if !time.is_empty() {
            ui.label(egui::RichText::new(time).small().weak());
        }
        if m.edited {
            ui.label(egui::RichText::new("изм.").small().weak());
        }
        if let Some(k) = &m.media_kind {
            ui.label(egui::RichText::new(format!("[{k}]")).strong());
        }
        if let Some(n) = m.reactions {
            if n > 0 {
                ui.label(egui::RichText::new(format!("⭐ {n}")).small());
            }
        }
        if let Some(v) = m.views {
            ui.label(egui::RichText::new(format!("👁 {v}")).small().weak());
        }
        if let Some(f) = m.forwards {
            if f > 0 {
                ui.label(egui::RichText::new(format!("➦ {f}")).small().weak());
            }
        }
    });
    if let Some(f) = &m.file_name {
        ui.horizontal(|ui| {
            ui.label(f);
            if let Some(s) = m.file_size {
                ui.label(
                    egui::RichText::new(crate::utils::human_size(s as u64))
                        .weak()
                        .small(),
                );
            }
            if m.has_media {
                if matched {
                    ui.label(
                        egui::RichText::new("✓ под правило")
                            .small()
                            .color(egui::Color32::from_rgb(40, 140, 60)),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("× вне правил")
                            .small()
                            .weak(),
                    );
                }
            }
        });
    }
    if !m.text.is_empty() {
        ui.label(crate::utils::truncate(&m.text, 2000));
    }
    // Стикеры: превью по месту (качается в кэш, не в Загрузки).
    if m.media_kind.as_deref() == Some("sticker") {
        let ctx = ui.ctx().clone();
        if let Some(tex) = app.ensure_sticker(&ctx, chat_id, m) {
            let size = tex.size_vec2();
            let scale =
                (128.0 / size.x.max(1.0)).min(128.0 / size.y.max(1.0));
            ui.image((tex.id(), egui::Vec2::new(size.x * scale, size.y * scale)));
        } else {
            ui.label(
                egui::RichText::new("🎭 стикер (превью грузится…)")
                    .small()
                    .weak(),
            );
        }
    }
    // Видео: превью первого кадра (встроенные декодеры, кэш) + ⬇.
    if m.media_kind.as_deref() == Some("video") {
        let ctx = ui.ctx().clone();
        if let Some(tex) = app.ensure_video_thumb(&ctx, m) {
            let size = tex.size_vec2();
            let scale =
                (160.0 / size.x.max(1.0)).min(160.0 / size.y.max(1.0));
            ui.image((tex.id(), egui::Vec2::new(size.x * scale, size.y * scale)));
        } else {
            ui.label(
                egui::RichText::new("🎬 видео (превью грузится…)")
                    .small()
                    .weak(),
            );
        }
    }
    // Веб-превью ссылки: карточка с фото, заголовком и кнопкой.
    if let Some(web) = &m.web_preview {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                if let Some(photo) = &web.photo {
                    let ctx = ui.ctx().clone();
                    if let Some(tex) = app.ensure_web_thumb(&ctx, photo) {
                        let size = tex.size_vec2();
                        let scale =
                            (80.0 / size.x.max(1.0)).min(80.0 / size.y.max(1.0));
                        ui.image((tex.id(), egui::Vec2::new(size.x * scale, size.y * scale)));
                    }
                }
                ui.vertical(|ui| {
                    if !web.title.is_empty() {
                        ui.label(egui::RichText::new(&web.title).strong());
                    }
                    if !web.description.is_empty() {
                        ui.label(
                            egui::RichText::new(crate::utils::truncate(&web.description, 140))
                                .small()
                                .weak(),
                        );
                    }
                    if !web.url.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(crate::utils::truncate(&web.url, 60))
                                    .small()
                                    .weak(),
                            );
                            if ui.small_button("↗").on_hover_text("Открыть ссылку").clicked() {
                                let _ = open::that(web.url.clone());
                            }
                        });
                    }
                });
            });
        });
    }
    // Гео и контакты — display-only строки.
    if let Some((lat, long)) = m.geo {
        ui.label(
            egui::RichText::new(format!("🌍 {lat:.5}, {long:.5}"))
                .small()
                .weak(),
        );
    }
    if let Some((name, phone)) = &m.contact {
        ui.label(
            egui::RichText::new(format!("👤 {name} · {phone}"))
                .small()
                .strong(),
        );
    }
    // Опрос: вопрос + варианты с голосами, тап — голосовать.
    if let Some(poll) = &m.poll {
        poll_card(app, ui, chat_id, m.id, poll);
    }
    // Медиа-воронка Telecomfort + плеер голосовых.
    if m.has_media {
        ui.horizontal(|ui| {
            // Голосовые/аудио — встроенный плеер (rodio), без сохранения в Загрузки.
            if m.media_kind.as_deref() == Some("audio") {
                let playing = app
                    .playback
                    .as_ref()
                    .map(|p| p.chat_id == chat_id && p.message_id == m.id)
                    .unwrap_or(false);
                let label = if playing { "⏹" } else { "▶" };
                if ui
                    .small_button(label)
                    .on_hover_text(if playing { "Остановить" } else { "Слушать" })
                    .clicked()
                {
                    app.play_audio(chat_id, m);
                }
            }
            if ui.small_button("⬇ Скачать (обход защиты)").clicked() {
                let title = app
                    .dialogs
                    .iter()
                    .find(|d| d.id == chat_id)
                    .map(|d| d.title.clone())
                    .unwrap_or_default();
                app.download_message(chat_id, title, m.clone());
            }
        });
    }
    // Действия как в официальном клиенте.
    ui.horizontal(|ui| {
        if ui.small_button("↩").on_hover_text("Ответить").clicked() {
            app.start_reply(chat_id, m);
        }
        for emoji in QUICK_REACTIONS {
            if ui.small_button(emoji).on_hover_text("Реакция (повтор — убрать)").clicked() {
                app.react_to(chat_id, m.id, emoji);
            }
        }
        if m.outgoing {
            if ui.small_button("✏️").on_hover_text("Редактировать").clicked() {
                app.start_edit(chat_id, m);
            }
            if ui.small_button("🗑").on_hover_text("Удалить у всех").clicked() {
                app.delete_msg(chat_id, m.id);
            }
        }
        if ui.small_button("📌").on_hover_text("Закрепить в чате").clicked() {
            app.toggle_pin(chat_id, m.id, false);
        }
        if ui.small_button("➦").on_hover_text("Переслать в другой чат").clicked() {
            app.start_forward(chat_id, m.id);
        }
    });
}

/// Карточка опроса: вопрос, варианты с голосами/выбором, тап — голосовать.
/// Закрытые/quiz помечаются; multiple-choice — по одному варианту за раз.
fn poll_card(
    app: &mut TelecomfortApp,
    ui: &mut egui::Ui,
    chat_id: i64,
    message_id: i32,
    poll: &crate::telegram::PollInfo,
) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            let mut title = format!("📊 {}", poll.question);
            if poll.quiz {
                title.push_str(" (викторина)");
            }
            if poll.closed {
                title.push_str(" · закрыт");
            }
            ui.label(egui::RichText::new(title).strong());
            if let Some(total) = poll.total_voters {
                ui.label(
                    egui::RichText::new(format!("голосов: {total}"))
                        .small()
                        .weak(),
                );
            }
            if !poll.closed {
                for a in &poll.answers {
                    let mut label = a.text.clone();
                    if let Some(v) = a.votes {
                        label.push_str(&format!(" — {v}"));
                    }
                    if a.chosen {
                        label.push_str(" ✓");
                    }
                    if ui.small_button(label).clicked() {
                        app.vote_poll(chat_id, message_id, a.option.clone());
                    }
                }
            } else {
                for a in &poll.answers {
                    let mut label = format!("· {}", a.text);
                    if let Some(v) = a.votes {
                        label.push_str(&format!(" — {v}"));
                    }
                    if a.chosen {
                        label.push_str(" ✓");
                    }
                    ui.label(egui::RichText::new(label).small());
                }
            }
        });
    });
}

/// Композер: баннер reply/edit + ввод + attach + отправка.
/// Черновик живёт в `app.composer_current` и сохраняется при смене чата.
fn composer(app: &mut TelecomfortApp, ui: &mut egui::Ui, chat_id: i64) {
    if let Some((_, mid, snippet)) = app.reply_to.clone() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("↩ ответ на #{mid}: {snippet}"))
                    .small()
                    .strong(),
            );
            if ui.small_button("✕").clicked() {
                app.reply_to = None;
            }
        });
    }
    if let Some((_, mid)) = app.editing {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("✏️ правка #{mid} (Enter — сохранить)"))
                    .small()
                    .strong(),
            );
            if ui.small_button("✕").on_hover_text("Отменить правку").clicked() {
                app.editing = None;
                app.composer_current.clear();
            }
        });
    }
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::multiline(&mut app.composer_current)
                .hint_text("Сообщение… (Ctrl+Enter — отправить)")
                .desired_rows(2),
        );
        // Ctrl+Enter — отправить (Enter — новая строка, как в десктопном Telegram).
        let hotkey =
            ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl) && resp.has_focus();
        if ui
            .button("📎")
            .on_hover_text("Прикрепить файл (фото — как фото, остальное — документом)")
            .clicked()
        {
            app.attach_file();
        }
        if ui
            .button("⏰")
            .on_hover_text("Отложенная отправка (сервер доставит через N часов)")
            .clicked()
        {
            app.sched_open = true;
        }
        let send_label = if app.editing.is_some() { "💾 Сохранить" } else { "➤ Отправить" };
        if ui.button(send_label).clicked() || hotkey {
            // В топике — отправка в топик, иначе обычная.
            if app.topic_sel.is_some() {
                let _ = chat_id;
                app.send_to_current_topic();
            } else {
                // Черновик текущего чата уже в composer_current; select_chat сохранит.
                let _ = chat_id;
                app.send_current();
            }
        }
    });
    // Окно отложенной отправки: часы задержки.
    if app.sched_open {
        egui::Window::new("⏰ Отложенная отправка")
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.label("Через сколько часов доставить:");
                ui.add(
                    egui::TextEdit::singleline(&mut app.sched_hours)
                        .hint_text("например, 2.5")
                        .desired_width(120.0),
                );
                ui.horizontal(|ui| {
                    if ui.button("Отправить позже").clicked() {
                        let hours: f64 =
                            app.sched_hours.replace(',', ".").parse().unwrap_or(0.0);
                        if hours > 0.0 {
                            app.send_scheduled(hours);
                        } else {
                            app.log("отложка: укажите часы > 0".to_string());
                        }
                    }
                    if ui.button("Отмена").clicked() {
                        app.sched_open = false;
                    }
                });
            });
    }
    ui.label(
        egui::RichText::new("Ctrl+Enter — отправить · черновик сохраняется при переключении чата")
            .small()
            .weak(),
    );
}

fn ensure_rule_for_chat(app: &mut TelecomfortApp, title: &str, chat_id: i64) {
    let chat_id_s = chat_id.to_string();
    let exists = app.cfg.downloader.rules.iter().any(|r| r.chat == title || r.chat == chat_id_s);
    if exists {
        app.log(format!("правило для «{title}» уже есть (см. Настройки)"));
        return;
    }
    app.cfg.downloader.rules.push(AutodownloadRule {
        chat: title.to_string(),
        kinds: vec![MediaKind::Any],
        max_size_mb: 1500,
        enabled: true,
        bypass_protected: true,
    });
    if let Err(e) = app.cfg.save() {
        app.log(format!("не удалось сохранить правило: {e:#}"));
    } else {
        app.log(format!("добавлено правило для «{title}» (Any, 1500 МБ)"));
    }
}
