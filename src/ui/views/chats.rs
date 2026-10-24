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
                if ui
                    .small_button("📍")
                    .on_hover_text("Люди и группы рядом (геопозиция из окон ввода)")
                    .clicked()
                {
                    app.nearby_open = true;
                    app.load_nearby();
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
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
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
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
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
            // Управление папками: создать/переименовать/удалить.
            ui.horizontal(|ui| {
                if ui
                    .small_button("＋ Папка")
                    .on_hover_text("Создать папку диалогов")
                    .clicked()
                {
                    app.folder_create_open = true;
                    app.folder_create_title.clear();
                    app.folder_create_peers.clear();
                }
                if ui
                    .small_button("⚙ Папки")
                    .on_hover_text("Переименовать/удалить папки сервера")
                    .clicked()
                {
                    app.folder_rename = Some((-1, String::new()));
                }
            });
            if app.folder_create_open {
                egui::Window::new("＋ Новая папка")
                    .collapsible(false)
                    .show(ui.ctx(), |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Название:");
                            ui.text_edit_singleline(&mut app.folder_create_title);
                            ui.label("Эмодзи:");
                            ui.text_edit_singleline(&mut app.folder_create_emoji);
                        });
                        ui.label("Диалоги в папке:");
                        egui::ScrollArea::vertical()
                            .max_height(260.0)
                            .show(ui, |ui| {
                                for d in app.dialogs.clone() {
                                    if d.archived {
                                        continue;
                                    }
                                    let mut sel = app.folder_create_peers.contains(&d.id);
                                    if ui.checkbox(&mut sel, &d.title).changed() {
                                        if sel {
                                            app.folder_create_peers.insert(d.id);
                                        } else {
                                            app.folder_create_peers.remove(&d.id);
                                        }
                                    }
                                }
                            });
                        ui.horizontal(|ui| {
                            if ui.button("Создать").clicked() {
                                app.do_create_folder();
                            }
                            if ui.button("Отмена").clicked() {
                                app.folder_create_open = false;
                            }
                        });
                    });
            }
            if app.folder_rename.is_some() {
                let folders = app.folders.clone();
                egui::Window::new("⚙ Папки сервера")
                    .collapsible(false)
                    .show(ui.ctx(), |ui| {
                        for f in folders {
                            ui.horizontal(|ui| {
                                ui.label(format!("[{}] {}", f.id, f.title));
                                if ui.small_button("✏️").clicked() {
                                    app.folder_rename = Some((f.id, f.title.clone()));
                                }
                                if ui.small_button("🗑").clicked() {
                                    app.do_delete_folder(f.id);
                                }
                            });
                        }
                        if let Some((fid, _)) = app.folder_rename.clone() {
                            if fid >= 0 {
                                ui.horizontal(|ui| {
                                    ui.label("Новое название:");
                                    if let Some((_, name)) = app.folder_rename.as_mut() {
                                        ui.text_edit_singleline(name);
                                    }
                                    if ui.button("OK").clicked() {
                                        let name = app
                                            .folder_rename
                                            .as_ref()
                                            .map(|(_, n)| n.clone())
                                            .unwrap_or_default();
                                        app.do_rename_folder(fid, name);
                                    }
                                });
                            }
                        }
                        if ui.button("Закрыть").clicked() {
                            app.folder_rename = None;
                        }
                    });
            }
            ui.separator();
            let ctx = ui.ctx().clone();
            egui::ScrollArea::vertical().show(ui, |ui| {
                let folder_sel = app.folder_sel;
                let peer_set: Option<std::collections::HashSet<i64>> = folder_sel.and_then(|fid| {
                    if fid == -1 {
                        None
                    } else {
                        app.folders
                            .iter()
                            .find(|f| f.id == fid)
                            .map(|f| f.peer_ids.iter().copied().collect())
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
                                let is_pinned = app.dialog_pinned.contains(&d.id);
                                if ui
                                    .small_button(if is_pinned { "📌" } else { "📍" })
                                    .on_hover_text(if is_pinned {
                                        "Открепить диалог"
                                    } else {
                                        "Закрепить диалог"
                                    })
                                    .clicked()
                                {
                                    app.toggle_dialog_pin(d.id);
                                }
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
                                    ui.label(egui::RichText::new("🔇").small().weak());
                                }
                                if d.archived {
                                    ui.label(egui::RichText::new("📦").small().weak());
                                }
                            });
                            if let Some(u) = &d.username {
                                ui.label(egui::RichText::new(format!("@{u}")).small().weak());
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
                if ui.small_button("📊").on_hover_text("Статистика канала").clicked() {
                    app.load_channel_stats(chat_id);
                }
                if ui.small_button("ℹ️").on_hover_text("О чате: описание, числа, TTL").clicked() {
                    app.load_chat_full(chat_id);
                    app.load_peer_full(chat_id);
                }
                if ui.small_button("⏳").on_hover_text("Автоудаление истории (TTL)").clicked() {
                    app.ttl_open = Some(chat_id);
                }
                if ui.small_button("📅").on_hover_text("Перейти к дате").clicked() {
                    app.jump_open = Some(chat_id);
                }
                if ui.small_button("🎤").on_hover_text("Отправить аудиофайл как голосовое").clicked() {
                    app.send_voice_pick(chat_id);
                }
                if ui.small_button("⭐").on_hover_text("Тарифы Stars-подарков").clicked() {
                    app.load_star_gifts(chat_id);
                }
                if ui.small_button("⤵J").on_hover_text("Экспорт истории в JSON").clicked() {
                    app.export_history(chat_id, "json");
                }
                if ui.small_button("⤵H").on_hover_text("Экспорт истории в HTML").clicked() {
                    app.export_history(chat_id, "html");
                }
                if ui.small_button("📌×N").on_hover_text("Все закреплённые сообщения").clicked() {
                    app.load_pinned_all(chat_id);
                }
                if ui.small_button("👥").on_hover_text("Участники (каналы/супергруппы)").clicked() {
                    app.load_participants(chat_id);
                }
                if ui.small_button("⏱").on_hover_text("Слоумод (задержка отправки)").clicked() {
                    app.slowmode_open = Some(chat_id);
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
            // Поиск по загруженной истории + медиа-табы (серверный фильтр).
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.add(
                    egui::TextEdit::singleline(&mut app.chat_search)
                        .hint_text("поиск по истории…")
                        .desired_width(220.0),
                );
                let tab = app.media_tab.clone();
                let mut chip = |ui: &mut egui::Ui, label: &str, kind: Option<&str>| {
                    let sel = tab.as_deref() == kind;
                    if ui.selectable_label(sel, label).clicked() {
                        app.select_media_tab(chat_id, kind.map(|s| s.to_string()));
                    }
                };
                ui.separator();
                chip(ui, "Все", None);
                chip(ui, "📷 Фото", Some("photo"));
                chip(ui, "🎬 Видео", Some("video"));
                chip(ui, "🔗 Ссылки", Some("link"));
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
            // Окно голосов опроса.
            if let Some(mid) = app.poll_voters_open {
                let (count, names) = app.poll_voters.get(&mid).cloned().unwrap_or((0, vec![]));
                egui::Window::new(format!("👥 Голоса ({count})")).collapsible(false).show(ui.ctx(), |ui| {
                    if names.is_empty() {
                        ui.label("пока пусто");
                    }
                    for n in names.iter().take(50) {
                        ui.label(format!("• {n}"));
                    }
                    if ui.button("Закрыть").clicked() {
                        app.poll_voters_open = None;
                    }
                });
            }
            // Окно «О чате»: описание, числа, TTL, bio собеседника, подарки.
            if let Some(cid) = app.chat_full_open {
                let info = app.chat_full.get(&cid).cloned().unwrap_or_default();
                egui::Window::new("ℹ️ О чате").collapsible(false).show(ui.ctx(), |ui| {
                    ui.set_min_width(360.0);
                    if info.about.is_empty() {
                        ui.label(egui::RichText::new("загружаю…").small().weak());
                    } else {
                        ui.label(&info.about);
                    }
                    let mut parts: Vec<String> = vec![];
                    if let Some(n) = info.participants_count {
                        parts.push(format!("участников: {n}"));
                    }
                    if let Some(n) = info.admins_count {
                        parts.push(format!("админов: {n}"));
                    }
                    if let Some(n) = info.online_count {
                        parts.push(format!("онлайн: {n}"));
                    }
                    if !parts.is_empty() {
                        ui.label(parts.join(" · "));
                    }
                    if let Some(linked) = info.linked_chat_id {
                        ui.horizontal(|ui| {
                            ui.label(format!("связанный чат: {linked}"));
                            if ui.small_button("→").on_hover_text("Открыть связанный чат").clicked() {
                                app.chat_full_open = None;
                                app.select_chat(linked);
                            }
                        });
                    }
                    if let Some(ttl) = info.ttl_period {
                        ui.label(if ttl == 0 {
                            "автоудаление: выкл".into()
                        } else {
                            format!("автоудаление: {ttl}с")
                        });
                    }
                    ui.separator();
                    // Собеседник (для 1-1): bio + флаги.
                    if let Some(pf) = app.peer_full.get(&cid).cloned() {
                        if !pf.about.is_empty() {
                            ui.label(format!("о себе: {}", pf.about));
                        }
                        let mut flags: Vec<&str> = vec![];
                        if pf.premium {
                            flags.push("⭐ Premium");
                        }
                        if pf.verified {
                            flags.push("✓ верифицирован");
                        }
                        if pf.scam {
                            flags.push("⚠ SCAM");
                        }
                        if pf.fake {
                            flags.push("⛔ фейк");
                        }
                        if pf.voice_forbidden {
                            flags.push("🚫 голосовые запрещены");
                        }
                        if !flags.is_empty() {
                            ui.label(egui::RichText::new(flags.join(" · ")).small().strong());
                        }
                    }
                    // Тарифы Stars-подарков.
                    if let Some(opts) = app.star_gifts.get(&cid).cloned() {
                        ui.separator();
                        ui.label("🎁 Тарифы подарков (только просмотр):");
                        for o in opts.iter().take(10) {
                            ui.label(format!("• ⭐ {} — {} {} ({} за штуку)", o.stars, o.amount, o.currency, o.currency));
                        }
                        ui.label(
                            egui::RichText::new("Отправка подарков: нужны коллекционные gift_id — вне скоупа Bot API.")
                                .small()
                                .weak(),
                        );
                    } else if ui.small_button("🎁 Тарифы подарков").clicked() {
                        app.load_star_gifts(cid);
                    }
                    ui.separator();
                    if ui.button("Закрыть").clicked() {
                        app.chat_full_open = None;
                    }
                });
            }
            // Окно TTL: пресеты + свой ввод.
            if let Some(cid) = app.ttl_open {
                egui::Window::new("⏳ Автоудаление истории").collapsible(false).show(ui.ctx(), |ui| {
                    ui.label("Сообщения удалятся у всех через:");
                    ui.horizontal(|ui| {
                        for (label, secs) in [("Выкл", 0), ("1 день", 86400), ("7 дней", 604800), ("1 месяц", 2592000), ("1 год", 31536000)] {
                            if ui.small_button(label).clicked() {
                                app.set_ttl(cid, secs);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Своё (сек):");
                        ui.text_edit_singleline(&mut app.ttl_custom);
                        if ui.small_button("Применить").clicked() {
                            let secs: i32 = app.ttl_custom.trim().parse().unwrap_or(-1);
                            if secs >= 0 {
                                app.set_ttl(cid, secs);
                            } else {
                                app.status = "секунды — число ≥ 0".into();
                            }
                        }
                    });
                    if ui.button("Закрыть").clicked() {
                        app.ttl_open = None;
                    }
                });
            }
            // Окно прыжка к дате.
            if let Some(cid) = app.jump_open {
                egui::Window::new("📅 Перейти к дате").collapsible(false).show(ui.ctx(), |ui| {
                    ui.label("Дата (ГГГГ-ММ-ДД или ДД.ММ.ГГГГ):");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut app.jump_date);
                        if ui.button("Перейти").clicked() {
                            app.jump_to_date(cid);
                            app.jump_open = None;
                        }
                    });
                    if ui.button("Закрыть").clicked() {
                        app.jump_open = None;
                    }
                });
            }
            // Окно «кто реагировал».
            if let Some((vcid, vmid)) = app.voters_open {
                let voters = app.voters.get(&(vcid, vmid)).cloned().unwrap_or_default();
                egui::Window::new("👥 Реакции").collapsible(false).show(ui.ctx(), |ui| {
                    if voters.is_empty() {
                        ui.label("загружаю…");
                    }
                    for v in voters.iter().take(100) {
                        ui.label(format!("{} {} · {}", v.emoji, v.name, crate::ui::app::format_msg_time(v.date)));
                    }
                    if ui.button("Закрыть").clicked() {
                        app.voters_open = None;
                    }
                });
            }
            // Окно «рядом».
            if app.nearby_open {
                egui::Window::new("📍 Рядом").collapsible(false).show(ui.ctx(), |ui| {
                    ui.set_min_width(320.0);
                    ui.horizontal(|ui| {
                        ui.label("lat:");
                        ui.text_edit_singleline(&mut app.nearby_lat);
                        ui.label("long:");
                        ui.text_edit_singleline(&mut app.nearby_long);
                        if ui.small_button("↻").on_hover_text("Обновить").clicked() {
                            app.load_nearby();
                        }
                    });
                    if app.nearby.is_empty() {
                        ui.label("пока пусто — нажмите ↻");
                    }
                    for p in app.nearby.clone() {
                        ui.horizontal(|ui| {
                            ui.label(if p.is_user { "👤" } else { "👥" });
                            ui.label(&p.title);
                            if !p.is_user && ui.small_button("→").on_hover_text("Открыть чат").clicked() {
                                app.nearby_open = false;
                                app.select_chat(p.id);
                            }
                        });
                    }
                    if ui.button("Закрыть").clicked() {
                        app.nearby_open = false;
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
                        ui.horizontal(|ui| {
                            if ui.small_button("⏏ Отозвать").on_hover_text("Аннулировать ссылку").clicked() {
                                app.revoke_invite(chat_id);
                            }
                            if ui.button("Закрыть").clicked() {
                                app.invite_link = None;
                            }
                        });
                    },
                );
            }
            // Окно всех закрепов чата.
            if let Some(cid) = app.pinned_more_open {
                let items = app.pinned_more.get(&cid).cloned().unwrap_or_default();
                egui::Window::new("📌 Все закрепы").collapsible(false).show(ui.ctx(), |ui| {
                    if items.is_empty() {
                        ui.label("закрепов нет");
                    }
                    for m in items {
                        ui.horizontal(|ui| {
                            ui.label(format!("#{} {}", m.id, crate::utils::truncate(&m.text, 80)));
                            if ui.small_button("✕").clicked() {
                                app.toggle_pin(cid, m.id, true);
                            }
                        });
                    }
                    if ui.button("Закрыть").clicked() {
                        app.pinned_more_open = None;
                    }
                });
            }
            // Окно участников (только каналы/супергруппы; кик — нужны права).
            if let Some(cid) = app.participants_open {
                let items = app.participants.get(&cid).cloned().unwrap_or_default();
                egui::Window::new("👥 Участники").collapsible(false).show(ui.ctx(), |ui| {
                    if items.is_empty() {
                        ui.label("участники не загружены / не канал");
                    }
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for p in items {
                            ui.horizontal(|ui| {
                                ui.label(format!("{} {}", if p.admin { "⭐" } else { "•" }, p.name));
                                if ui.small_button("⚙ Права").on_hover_text("Назначить/снять/настроить админа (нужны права)").clicked() {
                                    app.open_admin_editor(cid, &p);
                                }
                                if !p.admin {
                                    if ui.small_button("🚫").on_hover_text("Исключить").clicked() {
                                        app.kick_participant(cid, p.id);
                                    }
                                }
                            });
                        }
                    });
                    if ui.button("Закрыть").clicked() {
                        app.participants_open = None;
                    }
                });
            }
            // Редактор прав администратора: тонкие галки как в официальном клиенте.
            if app.admin_edit.is_some() {
                let mut close = false;
                let mut apply = false;
                egui::Window::new("🛡 Права администратора")
                    .collapsible(false)
                    .show(ui.ctx(), |ui| {
                        if let Some(ed) = app.admin_edit.as_mut() {
                            ui.label(format!("{} · id {}", ed.user_name, ed.user_id));
                            ui.horizontal(|ui| {
                                ui.label("Титул:");
                                ui.text_edit_singleline(&mut ed.title);
                            });
                            egui::Grid::new("admin_rights_grid").show(ui, |ui| {
                                let r = &mut ed.rights;
                                ui.checkbox(&mut r.change_info, "Инфо");
                                ui.checkbox(&mut r.post_messages, "Посты");
                                ui.end_row();
                                ui.checkbox(&mut r.edit_messages, "Правка");
                                ui.checkbox(&mut r.delete_messages, "Удаление");
                                ui.end_row();
                                ui.checkbox(&mut r.ban_users, "Бан");
                                ui.checkbox(&mut r.invite_users, "Инвайты");
                                ui.end_row();
                                ui.checkbox(&mut r.pin_messages, "Закрепы");
                                ui.checkbox(&mut r.manage_call, "Звонки");
                                ui.end_row();
                                ui.checkbox(&mut r.manage_topics, "Топики");
                                ui.checkbox(&mut r.post_stories, "Посты сторис");
                                ui.end_row();
                                ui.checkbox(&mut r.edit_stories, "Правка сторис");
                                ui.checkbox(&mut r.delete_stories, "Удаление сторис");
                                ui.end_row();
                                ui.checkbox(&mut r.manage_direct_messages, "Директ");
                                ui.label("");
                                ui.end_row();
                            });
                            ui.label(
                                egui::RichText::new("Пустой набор = разжаловать. Без анонимности и права добавлять админов.")
                                    .small()
                                    .weak(),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Применить").clicked() {
                                    apply = true;
                                }
                                if ui.button("Отмена").clicked() {
                                    close = true;
                                }
                            });
                        }
                    });
                if apply {
                    app.apply_admin_rights();
                }
                if close {
                    app.admin_edit = None;
                }
            }
            // Окно шаринга контакта: выбор из телефонной книги.
            if app.contact_share_pick.is_some() {
                let contacts = app.contacts.clone();
                ui.horizontal(|ui| {
                    if ui.small_button("👤 Мой контакт").on_hover_text("Поделиться своим номером").clicked() {
                        if let Some(cid) = app.contact_share_pick {
                            app.share_own_contact(cid);
                        }
                    }
                });
                if contacts.is_empty() {
                    app.load_contacts();
                }
                egui::Window::new("👤 Поделиться контактом").collapsible(false).show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for c in contacts {
                            if ui.selectable_label(false, format!("{} · {}", c.name, c.phone)).clicked() {
                                if let Some(cid) = app.contact_share_pick {
                                    app.share_contact_to(cid, &c);
                                }
                            }
                        }
                    });
                    if ui.button("Отмена").clicked() {
                        app.contact_share_pick = None;
                    }
                });
            }
            if let Some(cid) = app.channel_stats_open {
                if let Some(st) = app.channel_stats.get(&cid).cloned() {
                    egui::Window::new("📊 Статистика канала").collapsible(false).show(ui.ctx(), |ui| {
                        ui.set_min_width(360.0);
                        ui.horizontal(|ui| {
                            for (id, label) in [
                                ("summary", "Сводка"),
                                ("growth", "📈 Рост"),
                                ("interactions", "💬 Вовлечённость"),
                            ] {
                                let sel = app.stats_tab == id;
                                if ui.selectable_label(sel, label).clicked() {
                                    app.stats_tab = id.into();
                                }
                            }
                        });
                        ui.separator();
                        match app.stats_tab.as_str() {
                            "growth" | "interactions" => {
                                let kind = app.stats_tab.clone();
                                app.load_graph(cid, &kind);
                                match app.graphs.get(&(cid, kind.clone())).cloned() {
                                    Some((labels, values)) => draw_series(ui, &labels, &values),
                                    None => {
                                        ui.label("загружаю график…");
                                    }
                                }
                            }
                            _ => {
                                ui.label(format!("Подписчики: {} (было {})", st.followers, st.followers_prev));
                                ui.label(format!("Просмотры на пост: {:.0}", st.views_per_post));
                                ui.label(format!("Репосты на пост: {:.1}", st.shares_per_post));
                                ui.label(format!("Реакции на пост: {:.1}", st.reactions_per_post));
                                ui.label(format!("Просмотры сторис: {:.0}", st.views_per_story));
                                ui.label(format!("С уведомлениями: {:.0}%", st.notifications_pct));
                            }
                        }
                        if ui.button("Закрыть").clicked() {
                            app.channel_stats_open = None;
                        }
                    });
                }
            }
            // Подтверждения опасных действий — глобальное окно в app.rs (работает с любого экрана).
            if let Some(cid) = app.slowmode_open {
                egui::Window::new("⏱ Слоумод").collapsible(false).show(ui.ctx(), |ui| {
                    ui.label("Задержка между сообщениями (0 — выкл):");
                    ui.horizontal(|ui| {
                        for (label, secs) in [("Выкл", "0"), ("30с", "30"), ("1м", "60"), ("5м", "300"), ("1ч", "3600")] {
                            if ui.small_button(label).clicked() {
                                app.slowmode_secs = secs.to_string();
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut app.slowmode_secs);
                        if ui.button("Применить").clicked() {
                            let secs: i32 = app.slowmode_secs.trim().parse().unwrap_or(0).max(0);
                            app.set_slowmode(cid, secs);
                            app.slowmode_open = None;
                        }
                        if ui.button("Отмена").clicked() {
                            app.slowmode_open = None;
                        }
                    });
                });
            }
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
    let kind =
        crate::media::AutoDownloader::classify(m.file_name.as_deref(), m.media_kind.as_deref());
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
                if ui
                    .small_button("👥")
                    .on_hover_text("Кто реагировал")
                    .clicked()
                {
                    app.load_voters(chat_id, m.id);
                }
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
                    ui.label(egui::RichText::new("× вне правил").small().weak());
                }
            }
        });
    }
    if !m.text.is_empty() {
        ui.label(crate::utils::truncate(&m.text, 2000));
    }
    // Показываем перевод под текстом, если запрошен.
    if let Some(tr) = app.translations.get(&(chat_id, m.id)) {
        ui.separator();
        ui.label(
            egui::RichText::new(crate::utils::truncate(tr, 2000))
                .italics()
                .weak(),
        );
    }
    // Стикеры: превью по месту (качается в кэш, не в Загрузки).
    if m.media_kind.as_deref() == Some("sticker") {
        let ctx = ui.ctx().clone();
        if let Some(tex) = app.ensure_sticker(&ctx, chat_id, m) {
            let size = tex.size_vec2();
            let scale = (128.0 / size.x.max(1.0)).min(128.0 / size.y.max(1.0));
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
            let scale = (160.0 / size.x.max(1.0)).min(160.0 / size.y.max(1.0));
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
                        let scale = (80.0 / size.x.max(1.0)).min(80.0 / size.y.max(1.0));
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
                            if ui
                                .small_button("↗")
                                .on_hover_text("Открыть ссылку")
                                .clicked()
                            {
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
                    .on_hover_text(if playing {
                        "Остановить"
                    } else {
                        "Слушать"
                    })
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
            if ui
                .small_button(emoji)
                .on_hover_text("Реакция (повтор — убрать)")
                .clicked()
            {
                app.react_to(chat_id, m.id, emoji);
            }
        }
        if m.outgoing {
            if ui
                .small_button("✏️")
                .on_hover_text("Редактировать")
                .clicked()
            {
                app.start_edit(chat_id, m);
            }
            if ui
                .small_button("🗑")
                .on_hover_text("Удалить у всех")
                .clicked()
            {
                app.delete_msg(chat_id, m.id);
            }
        }
        if ui
            .small_button("📌")
            .on_hover_text("Закрепить в чате")
            .clicked()
        {
            app.toggle_pin(chat_id, m.id, false);
        }
        if ui
            .small_button("➦")
            .on_hover_text("Переслать в другой чат")
            .clicked()
        {
            app.start_forward(chat_id, m.id);
        }
        // 🌐 Перевод: повторный тап скрывает.
        let has_tr = app.translations.contains_key(&(chat_id, m.id));
        if ui
            .small_button(if has_tr { "🌐✓" } else { "🌐" })
            .on_hover_text("Перевести сообщение (авто: кириллица→en, иначе→ru)")
            .clicked()
        {
            app.translate_message(chat_id, m);
        }
        if ui
            .small_button("🔗")
            .on_hover_text("Скопировать ссылку на сообщение")
            .clicked()
        {
            let link = message_link(app, chat_id, m.id);
            ui.ctx().copy_text(link.clone());
            app.log(format!("ссылка скопирована: {link}"));
        }
    });
}

/// Ссылка на сообщение: t.me/username/id или t.me/c/internal/id.
pub fn message_link(app: &TelecomfortApp, chat_id: i64, message_id: i32) -> String {
    if let Some(u) = app
        .dialogs
        .iter()
        .find(|d| d.id == chat_id)
        .and_then(|d| d.username.clone())
    {
        return format!("https://t.me/{u}/{message_id}");
    }
    // Внутренний id канала: chat_id = -(10^12 + channel_id).
    let internal = if chat_id < -1_000_000_000_000 {
        (-chat_id - 1_000_000_000_000).to_string()
    } else {
        chat_id.abs().to_string()
    };
    format!("https://t.me/c/{internal}/{message_id}")
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
            if let Some(sol) = &poll.solution {
                if !sol.is_empty() {
                    ui.label(egui::RichText::new(format!("💡 {sol}")).small().weak());
                }
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
                if ui
                    .small_button("■ Остановить")
                    .on_hover_text("Закрыть голосование")
                    .clicked()
                {
                    app.stop_poll(chat_id, message_id);
                }
                let vlabel = match app.poll_voters.get(&message_id) {
                    Some((total, _)) => format!("👥 {total}"),
                    None => "👥".to_string(),
                };
                if ui
                    .small_button(vlabel)
                    .on_hover_text("Кто голосовал")
                    .clicked()
                {
                    app.load_poll_voters(chat_id, message_id, None);
                }
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
    // Панель стикеров: превью или эмодзи, клик — отправка.
    if app.stickers_open {
        let stickers = app.stickers.clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("😀").small().weak());
            egui::ScrollArea::horizontal()
                .max_height(84.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for st in stickers.iter().take(24) {
                            let sent = if let Some(path) = &st.preview_path {
                                if let Some(tex) = TelecomfortApp::thumb_from_file(
                                    ui.ctx(),
                                    path,
                                    &format!("sticker::{}", st.id),
                                    64,
                                ) {
                                    ui.add(
                                        egui::Image::new((tex.id(), egui::Vec2::new(56.0, 56.0)))
                                            .sense(egui::Sense::click()),
                                    )
                                    .clicked()
                                } else {
                                    ui.small_button(st.emoji.clone().unwrap_or_else(|| "😀".into()))
                                        .clicked()
                                }
                            } else {
                                ui.small_button(st.emoji.clone().unwrap_or_else(|| "😀".into()))
                                    .clicked()
                            };
                            if sent {
                                app.send_saved_document(
                                    chat_id,
                                    st.id,
                                    st.access_hash,
                                    st.file_reference.clone(),
                                );
                            }
                        }
                    });
                });
            if ui.small_button("✕").clicked() {
                app.stickers_open = false;
            }
        });
    }
    // Панель GIF: превью-кадры, клик — отправка.
    if app.gifs_open {
        let gifs = app.gifs.clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🎞").small().weak());
            egui::ScrollArea::horizontal()
                .max_height(84.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for g in gifs.iter().take(24) {
                            let sent = if let Some(path) = &g.preview_path {
                                if let Some(tex) = TelecomfortApp::thumb_from_file(
                                    ui.ctx(),
                                    path,
                                    &format!("gif::{}", g.id),
                                    64,
                                ) {
                                    ui.add(
                                        egui::Image::new((tex.id(), egui::Vec2::new(72.0, 54.0)))
                                            .sense(egui::Sense::click()),
                                    )
                                    .clicked()
                                } else {
                                    ui.small_button("🎞").clicked()
                                }
                            } else {
                                ui.small_button("🎞").clicked()
                            };
                            if sent {
                                app.send_saved_document(
                                    chat_id,
                                    g.id,
                                    g.access_hash,
                                    g.file_reference.clone(),
                                );
                            }
                        }
                    });
                });
            if ui.small_button("✕").clicked() {
                app.gifs_open = false;
            }
        });
    }
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
            if ui
                .small_button("✕")
                .on_hover_text("Отменить правку")
                .clicked()
            {
                app.editing = None;
                app.composer_current.clear();
            }
        });
    }
    // Очередь вложений: файлы уйдут вместе с текстом (подпись) по Ctrl+Enter.
    if let Some(chat_id) = app.selected_chat {
        if let Some(queued) = app.pending_attach.get(&chat_id).cloned() {
            if !queued.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("📎 {}:", queued.len()))
                            .small()
                            .weak(),
                    );
                    for (i, path) in queued.iter().enumerate() {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "?".into());
                        let size_txt = std::fs::metadata(path)
                            .map(|m| crate::utils::human_size(m.len()))
                            .unwrap_or_default();
                        ui.label(
                            egui::RichText::new(format!("{name} {size_txt}"))
                                .small()
                                .monospace(),
                        );
                        if ui
                            .small_button("✕")
                            .on_hover_text("Убрать из очереди")
                            .clicked()
                        {
                            app.drop_pending_attach(chat_id, i);
                        }
                    }
                });
            }
        }
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
            .on_hover_text("Прикрепить файлы в очередь (отправятся с сообщением)")
            .clicked()
        {
            if let Some(chat_id) = app.selected_chat {
                app.send_album_pick(chat_id);
            }
        }
        let mut sched = app.attach_scheduled;
        if ui
            .checkbox(&mut sched, "⏰")
            .on_hover_text("Отложить следующее вложение (часы — из окна ⏰)")
            .changed()
        {
            app.attach_scheduled = sched;
        }
        if ui
            .button("👤")
            .on_hover_text("Поделиться контактом")
            .clicked()
        {
            app.contact_share_pick = Some(chat_id);
        }
        if ui
            .button("⏰")
            .on_hover_text("Отложенная отправка (сервер доставит через N часов)")
            .clicked()
        {
            app.sched_open = true;
        }
        if ui
            .button("📊")
            .on_hover_text("Создать опрос/викторину")
            .clicked()
        {
            app.poll_open = true;
        }
        if ui
            .button("📍")
            .on_hover_text("Поделиться геопозицией")
            .clicked()
        {
            app.geo_open = true;
        }
        if ui
            .button("🎤")
            .on_hover_text("Голосовое из аудиофайла (ogg/opus/mp3/m4a)")
            .clicked()
        {
            app.send_voice_pick(chat_id);
        }
        if ui.button("😀").on_hover_text("Недавние стикеры").clicked() {
            app.load_stickers();
        }
        if ui.button("🎞").on_hover_text("Сохранённые GIF").clicked() {
            app.load_gifs();
        }
        ui.checkbox(&mut app.silent_send, "🔕")
            .on_hover_text("Тихая отправка (без звука у получателя)");
        let send_label = if app.editing.is_some() {
            "💾 Сохранить"
        } else {
            "➤ Отправить"
        };
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
                        let hours: f64 = app.sched_hours.replace(',', ".").parse().unwrap_or(0.0);
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
    // Конструктор опроса/викторины.
    if app.poll_open {
        egui::Window::new("📊 Новый опрос")
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Вопрос:");
                    ui.text_edit_singleline(&mut app.poll_question);
                });
                ui.label("Варианты (по одному на строку, 2–10):");
                ui.add(
                    egui::TextEdit::multiline(&mut app.poll_options)
                        .hint_text("Вариант 1\nВариант 2\n…")
                        .desired_rows(3),
                );
                ui.horizontal(|ui| {
                    ui.checkbox(&mut app.poll_quiz, "Викторина");
                    ui.checkbox(&mut app.poll_anonymous, "Анонимный");
                    ui.checkbox(&mut app.poll_multiple, "Несколько ответов");
                });
                if app.poll_quiz {
                    ui.horizontal(|ui| {
                        ui.label("Правильный № (с 0):");
                        ui.text_edit_singleline(&mut app.poll_correct);
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Опубликовать").clicked() {
                        app.do_create_poll(chat_id);
                    }
                    if ui.button("Отмена").clicked() {
                        app.poll_open = false;
                    }
                });
            });
    }
    // Отправка геопозиции.
    if app.geo_open {
        egui::Window::new("📍 Геопозиция")
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Широта:");
                    ui.text_edit_singleline(&mut app.geo_lat);
                });
                ui.horizontal(|ui| {
                    ui.label("Долгота:");
                    ui.text_edit_singleline(&mut app.geo_long);
                });
                ui.horizontal(|ui| {
                    if ui.button("Отправить точку").clicked() {
                        app.do_send_geo(chat_id);
                    }
                    if ui.button("Отмена").clicked() {
                        app.geo_open = false;
                    }
                });
            });
    }
    ui.label(
        egui::RichText::new("Ctrl+Enter — отправить · черновик сохраняется при переключении чата · поддерживается **жирный** *курсив* `код`")
            .small()
            .weak(),
    );
}

fn ensure_rule_for_chat(app: &mut TelecomfortApp, title: &str, chat_id: i64) {
    let chat_id_s = chat_id.to_string();
    let exists = app
        .cfg
        .downloader
        .rules
        .iter()
        .any(|r| r.chat == title || r.chat == chat_id_s);
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

/// Мини-график временного ряда: сетка + ломаная + подписи мин/макс и крайних дат.
fn draw_series(ui: &mut egui::Ui, labels: &[String], values: &[f64]) {
    if values.len() < 2 {
        ui.label("мало точек для графика");
        return;
    }
    let w = ui.available_width().max(200.0);
    let h = 140.0;
    let (resp, painter) = ui.allocate_painter(egui::Vec2::new(w, h), egui::Sense::hover());
    let rect = resp.rect;
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-9);
    let pad = egui::Vec2::new(8.0, 10.0);
    let plot = egui::Rect::from_min_max(rect.min + pad, rect.max - pad);
    // Горизонтальная сетка (3 линии).
    for i in 0..=2 {
        let y = plot.min.y + plot.height() * (i as f32) / 2.0;
        painter.line_segment(
            [
                egui::Pos2::new(plot.min.x, y),
                egui::Pos2::new(plot.max.x, y),
            ],
            egui::Stroke::new(1.0, ui.visuals().faint_bg_color),
        );
    }
    let n = values.len();
    let pts: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = plot.min.x + plot.width() * (i as f32) / ((n - 1) as f32);
            let y = plot.max.y - plot.height() * ((v - min) as f32) / (span as f32);
            egui::Pos2::new(x, y)
        })
        .collect();
    let last_pt = pts.last().copied();
    painter.add(egui::Shape::line(
        pts,
        egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
    ));
    // Последняя точка.
    if let Some(last) = last_pt {
        painter.circle_filled(last, 3.0, ui.visuals().selection.bg_fill);
    }
    drop(painter);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{:.0}", min)).small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.label(egui::RichText::new(format!("{:.0}", max)).small().weak());
        });
    });
    if let (Some(first), Some(last)) = (labels.first(), labels.last()) {
        ui.label(
            egui::RichText::new(format!("{first} → {last}"))
                .small()
                .weak(),
        );
    }
}
