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
            });
            ui.separator();
            let ctx = ui.ctx().clone();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for d in app.dialogs.clone() {
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
                }
            });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(chat_id) = app.selected_chat {
            let (title, username, protected) = app
                .dialogs
                .iter()
                .find(|d| d.id == chat_id)
                .map(|d| (d.title.clone(), d.username.clone(), d.protected))
                .unwrap_or_else(|| (format!("chat {chat_id}"), None, false));
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
                if ui
                    .small_button("+ Правило для этого чата")
                    .on_hover_text("Добавить per-chat правило автозагрузки")
                    .clicked()
                {
                    ensure_rule_for_chat(app, &title, chat_id);
                }
            });
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
            composer(app, ui, chat_id);
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
    // Медиа-воронка Telecomfort.
    if m.has_media {
        ui.horizontal(|ui| {
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
        let send_label = if app.editing.is_some() { "💾 Сохранить" } else { "➤ Отправить" };
        if ui.button(send_label).clicked() || hotkey {
            // Черновик текущего чата уже в composer_current; select_chat сохранит.
            let _ = chat_id;
            app.send_current();
        }
    });
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
