//! Контакты телефонной книги Telegram: имя, телефон, @username.
//! Тап «💬» — переход в диалог (матчим по username, иначе по имени в названии),
//! иначе подсказка. Загрузка — ленивая, один раз за сессию.

use crate::ui::app::TelecomfortApp;

pub fn show(app: &mut TelecomfortApp, ui: &mut egui::Ui) {
    ui.heading("👥 Контакты");
    ui.label(
        egui::RichText::new("Телефонная книга Telegram (имя · телефон · @username).")
            .small()
            .weak(),
    );
    ui.horizontal(|ui| {
        if ui.small_button("↻ Обновить").clicked() {
            app.contacts_loaded = false;
            app.load_contacts();
        }
        if ui.small_button("＋ Добавить").on_hover_text("Добавить контакт по телефону").clicked() {
            app.contact_add_open = true;
        }
        if ui.small_button("🚫 Чёрный список").on_hover_text("Показать заблокированных").clicked() {
            app.load_blocked();
        }
    });
    if app.contact_add_open {
        egui::Window::new("＋ Новый контакт").collapsible(false).show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Телефон:");
                ui.text_edit_singleline(&mut app.contact_add_phone);
            });
            ui.horizontal(|ui| {
                ui.label("Имя:");
                ui.text_edit_singleline(&mut app.contact_add_first);
                ui.label("Фамилия:");
                ui.text_edit_singleline(&mut app.contact_add_last);
            });
            ui.horizontal(|ui| {
                if ui.button("Добавить").clicked() {
                    app.do_add_contact();
                }
                if ui.button("Отмена").clicked() {
                    app.contact_add_open = false;
                }
            });
        });
    }
    ui.separator();
    if app.contacts.is_empty() {
        ui.horizontal(|ui| {
            ui.label("пока пусто");
            if ui.small_button("↻ Загрузить").clicked() {
                app.load_contacts();
            }
        });
        return;
    }
    ui.label(
        egui::RichText::new(format!("контактов: {}", app.contacts.len()))
            .small()
            .weak(),
    );
    egui::ScrollArea::vertical().show(ui, |ui| {
        for c in app.contacts.clone() {
            ui.horizontal(|ui| {
                let initials: String = c.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
                ui.label(egui::RichText::new(format!("[{initials}]")).strong());
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&c.name).strong());
                    let mut sub = c.phone.clone();
                    if let Some(u) = &c.username {
                        sub.push_str(&format!(" · @{u}"));
                    }
                    ui.label(egui::RichText::new(sub).small().weak());
                });
                if ui.small_button("💬").on_hover_text("Открыть чат").clicked() {
                    open_chat_with(app, &c);
                }
                if ui
                    .small_button("🗑")
                    .on_hover_text("Удалить контакт")
                    .clicked()
                {
                    app.delete_contact(c.user_id, c.access_hash);
                }
            });
            ui.separator();
        }
        // Чёрный список.
        if !app.blocked.is_empty() {
            ui.heading("🚫 Заблокированные");
            for b in app.blocked.clone() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&b.name).strong());
                    if ui
                        .small_button("✅ Разблокировать")
                        .on_hover_text("Снять блокировку")
                        .clicked()
                    {
                        app.toggle_block(b.id, true);
                    }
                });
            }
        }
    });
}

/// Переход в диалог контакта: сначала по username, иначе по совпадению имени.
fn open_chat_with(app: &mut TelecomfortApp, c: &crate::telegram::ContactInfo) {
    // 1-1 диалоги обычно названы именем контакта; username — точный матч.
    let mut best: Option<i64> = None;
    if let Some(u) = &c.username {
        best = app
            .dialogs
            .iter()
            .find(|d| d.username.as_deref() == Some(u.as_str()))
            .map(|d| d.id);
    }
    if best.is_none() {
        let first = c.name.split_whitespace().next().unwrap_or(&c.name);
        best = app
            .dialogs
            .iter()
            .find(|d| d.title.contains(first) || d.title.contains(&c.name))
            .map(|d| d.id);
    }
    match best {
        Some(id) => {
            app.screen = crate::ui::app::Screen::Chats;
            app.select_chat(id);
        }
        None => {
            app.log(format!("чат с «{}» не найден в диалогах (напишите первым)", c.name));
        }
    }
}
