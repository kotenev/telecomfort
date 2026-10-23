use crate::config::{AutodownloadRule, MediaKind};
use crate::ui::app::TelecomfortApp;

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
            egui::ScrollArea::vertical().show(ui, |ui| {
                for d in app.dialogs.clone() {
                    let sel = app.selected_chat == Some(d.id);
                    let title = format!(
                        "{}{} {}",
                        if d.protected { "🔒 " } else { "" },
                        d.title,
                        if d.unread > 0 {
                            format!("({})", d.unread)
                        } else {
                            String::new()
                        }
                    );
                    if ui.selectable_label(sel, title).clicked() {
                        app.load_history(d.id);
                    }
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
            let (title, protected) = app
                .dialogs
                .iter()
                .find(|d| d.id == chat_id)
                .map(|d| (d.title.clone(), d.protected))
                .unwrap_or_else(|| (format!("chat {chat_id}"), false));
            ui.heading(title.clone());
            ui.label(
                egui::RichText::new(
                    "Файлы/видео/аудио из закрытых групп скачиваются напрямую через MTProto (флаг noforwards игнорируется — он лишь прячет кнопки в официальных клиентах).",
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
            ui.separator();
            let items = app.history.get(&chat_id).cloned().unwrap_or_default();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for m in items {
                    // Матчится ли сообщение под правила (подсветка)?
                    let kind = crate::media::AutoDownloader::classify(
                        m.file_name.as_deref(),
                        m.media_kind.as_deref(),
                    );
                    let size = m.file_size.unwrap_or(0).max(0) as u64;
                    let matched = crate::media::AutoDownloader::matches_rules(
                        &app.cfg.downloader,
                        &title,
                        &kind,
                        size,
                    ) || crate::media::AutoDownloader::matches_rules(
                        &app.cfg.downloader,
                        &chat_id.to_string(),
                        &kind,
                        size,
                    );
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("#{}", m.id));
                            if let Some(k) = &m.media_kind {
                                ui.label(egui::RichText::new(format!("[{k}]")).strong());
                            }
                            if let Some(f) = &m.file_name {
                                ui.label(f);
                            }
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
                        if !m.text.is_empty() {
                            ui.label(crate::utils::truncate(&m.text, 300));
                        }
                        if m.has_media {
                            if ui.small_button("⬇ Скачать (обход защиты)").clicked() {
                                app.download_message(chat_id, title.clone(), m.clone());
                            }
                        }
                    });
                }
                if app.history.get(&chat_id).map(|v| v.is_empty()).unwrap_or(true) {
                    ui.label("история пуста");
                }
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Выберите диалог слева");
            });
        }
    });
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
