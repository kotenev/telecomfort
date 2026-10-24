//! Вход MTProto + мультиаккаунты + экспорт/импорт сессии.

use crate::config::{session_path_for, TelegramSettings};
use crate::ui::app::{BgEvent, TelecomfortApp};

pub fn show(app: &mut TelecomfortApp, ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new("Ключи — https://my.telegram.org → API development tools. Регистрация новых аккаунтов через сторонние клиенты запрещена — аккаунт должен существовать.")
            .small()
            .weak(),
    );
    ui.separator();
    // --- Профили ---
    ui.horizontal(|ui| {
        ui.label("Профиль:");
        egui::ComboBox::from_id_salt("profile_combo")
            .selected_text(app.active_profile.clone())
            .show_ui(ui, |ui| {
                for p in &app.profiles {
                    ui.selectable_value(&mut app.active_profile, p.name.clone(), &p.name);
                }
            });
        if ui.small_button("Переключить").clicked() {
            switch_profile(app);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Новый профиль:");
        ui.text_edit_singleline(&mut app.new_profile_name);
        if ui.small_button("+ Создать").clicked() && !app.new_profile_name.trim().is_empty()
        {
            let name = app.new_profile_name.trim().to_string();
            if !app.profiles.iter().any(|p| p.name == name) {
                app.profiles.push(TelegramSettings {
                    name: name.clone(),
                    api_id: 0,
                    api_hash: String::new(),
                    phone: String::new(),
                    session_path: session_path_for(&name),
                    proxy_url: String::new(),
                });
                app.active_profile = name;
                switch_profile(app);
                app.new_profile_name.clear();
            } else {
                app.status = "такой профиль уже есть".into();
            }
        }
        if app.profiles.len() > 1 {
            if ui.small_button("🗑 Удалить активный").clicked() {
                let name = app.active_profile.clone();
                app.profiles.retain(|p| p.name != name);
                app.active_profile = app
                    .profiles
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "main".into());
                switch_profile(app);
                app.profiles_to_cfg();
                let _ = app.cfg.save();
            }
        }
    });
    ui.label(
        egui::RichText::new(format!(
            "вошли как: {} · сессия: {}",
            app.me_label,
            app.profiles
                .iter()
                .find(|p| p.name == app.active_profile)
                .map(|p| p.session_path.to_string_lossy().to_string())
                .unwrap_or_default()
        ))
        .small()
        .weak(),
    );
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("API ID:");
        ui.text_edit_singleline(&mut app.api_id);
        ui.label("API HASH:");
        ui.text_edit_singleline(&mut app.api_hash);
    });
    ui.horizontal(|ui| {
        ui.label("SOCKS5:");
        ui.text_edit_singleline(&mut app.proxy_url).on_hover_text(
            "socks5://user:pass@host:port — пусто = напрямую. MTProto-прокси не поддерживаются.",
        );
    });
    ui.horizontal(|ui| {
        ui.label("Телефон:");
        ui.text_edit_singleline(&mut app.phone);
    });
    ui.horizontal(|ui| {
        if ui.button("📲 Получить код").clicked() {
            start_login(app);
        }
        ui.label(
            egui::RichText::new(format!("этап: {}", app.login_stage))
                .small()
                .weak(),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Код:");
        ui.text_edit_singleline(&mut app.code);
        if ui.button("Войти по коду").clicked() {
            submit_code(app);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Пароль 2FA:");
        ui.add(egui::TextEdit::singleline(&mut app.password).password(true));
        if ui.button("Войти с паролем").clicked() {
            submit_password(app);
        }
    });
    ui.separator();
    // Вход по QR-коду: телефон/код не нужны, только API ID+HASH.
    ui.horizontal(|ui| {
        if !app.qr_polling {
            if ui.button("📷 Войти по QR").clicked() {
                app.start_qr_login();
            }
        } else if ui.button("✕ Отменить QR").clicked() {
            app.cancel_qr_login();
        }
        // Баланс звёзд (загружается после входа).
        match app.stars {
            Some((amount, _)) => {
                ui.label(format!("⭐ {amount}"));
            }
            None => {
                if app.login_stage == "done"
                    && ui
                        .small_button("⭐")
                        .on_hover_text("Баланс звёзд")
                        .clicked()
                {
                    app.load_stars();
                }
            }
        }
    });
    if let Some((token, expires)) = app.qr_token.clone() {
        let now = chrono::Local::now().timestamp();
        let left = (expires - now).max(0);
        ui.label(format!(
            "Отсканируйте в Telegram на телефоне (Настройки → Устройства). Осталось ~{left} с."
        ));
        let payload = format!(
            "tg://login?token={}",
            super::super::app::qr_login_payload(&token)
        );
        // Текстура строится один раз по токену (кэш в app.qr_tex).
        let tex = app.qr_texture(ui.ctx(), &payload);
        if let Some(t) = tex {
            ui.image((t.id(), egui::Vec2::new(232.0, 232.0)));
        }
        ui.label(egui::RichText::new(payload).small().weak().monospace());
    }
    ui.separator();
    ui.horizontal(|ui| {
        if ui.small_button("💾 Экспорт сессии…").clicked() {
            export_session(app);
        }
        if ui.small_button("📥 Импорт сессии…").clicked() {
            import_session(app);
        }
        if ui.small_button("🚪 Выйти (стереть сессию)").clicked() {
            logout(app);
        }
    });
    ui.label(
        egui::RichText::new(
            "Сессия хранится локально (session_<профиль>.session). Демо-режим работает без ключей.",
        )
        .small()
        .weak(),
    );
}

fn current_session_path(app: &TelecomfortApp) -> std::path::PathBuf {
    app.profiles
        .iter()
        .find(|p| p.name == app.active_profile)
        .map(|p| p.session_path.clone())
        .unwrap_or_else(|| session_path_for(&app.active_profile))
}

fn switch_profile(app: &mut TelecomfortApp) {
    if let Some(p) = app
        .profiles
        .iter()
        .find(|p| p.name == app.active_profile)
        .cloned()
    {
        app.api_id = if p.api_id == 0 {
            String::new()
        } else {
            p.api_id.to_string()
        };
        app.api_hash = p.api_hash.clone();
        app.phone = p.phone.clone();
        app.proxy_url = p.proxy_url.clone();
        app.login_stage = "api".into();
        app.status = format!("профиль «{}» выбран — введите код или войдите", p.name);
        app.profiles_to_cfg();
        let _ = app.cfg.save();
    }
}

fn export_session(app: &mut TelecomfortApp) {
    let src = current_session_path(app);
    if !src.exists() {
        app.status = "сессии пока нет — сначала войдите".into();
        return;
    }
    if let Some(dst) = rfd::FileDialog::new()
        .set_file_name(format!("{}_session.session", app.active_profile))
        .save_file()
    {
        match std::fs::copy(&src, &dst) {
            Ok(_) => app.status = format!("сессия экспортирована: {}", dst.display()),
            Err(e) => app.status = format!("экспорт не удался: {e:#}"),
        }
    }
}

fn import_session(app: &mut TelecomfortApp) {
    if let Some(src) = rfd::FileDialog::new().pick_file() {
        let dst = current_session_path(app);
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::copy(&src, &dst) {
            Ok(_) => {
                app.status = format!("сессия импортирована из {}", src.display());
                app.log("сессия импортирована — нажмите «Получить код» для переподключения");
            }
            Err(e) => app.status = format!("импорт не удался: {e:#}"),
        }
    }
}

fn logout(app: &mut TelecomfortApp) {
    let path = current_session_path(app);
    let _ = std::fs::remove_file(&path);
    app.login_stage = "api".into();
    app.status = "сессия стёрта — войдите заново".into();
    app.log("logout: файл сессии удалён");
}

fn start_login(app: &mut TelecomfortApp) {
    let api_id: i32 = match app.api_id.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            app.status = "API ID должен быть числом".into();
            return;
        }
    };
    if app.api_hash.trim().is_empty() || app.phone.trim().is_empty() {
        app.status = "заполните API HASH и телефон".into();
        return;
    }
    let api_hash = app.api_hash.trim().to_string();
    let phone = app.phone.trim().to_string();
    let profile_name = app.active_profile.clone();
    let tx = app.tx.clone();
    let rt = app.rt.clone();
    // Сохраняем в профиль + конфиг для следующего запуска.
    if let Some(p) = app.profiles.iter_mut().find(|p| p.name == profile_name) {
        p.api_id = api_id;
        p.api_hash = api_hash.clone();
        p.phone = phone.clone();
        p.proxy_url = app.proxy_url.trim().to_string();
        if p.session_path.as_os_str().is_empty() {
            p.session_path = session_path_for(&profile_name);
        }
    }
    app.profiles_to_cfg();
    let session_path = current_session_path(app);
    let workers = app.cfg.downloader.parallel_chunks;
    let _ = app.cfg.save();
    app.status = "подключаюсь…".into();
    app.login_stage = "connecting".into();
    let proxy = {
        let pu = app.proxy_url.trim().to_string();
        if pu.is_empty() {
            None
        } else {
            Some(pu)
        }
    };

    rt.spawn(async move {
        match crate::telegram::backend_grammers::GrammersBackend::connect_with_proxy(
            &session_path,
            api_id,
            &api_hash,
            proxy,
        )
        .await
        {
            Ok(backend) => {
                // Чанки MTProto из настроек (при смене — перевойдите).
                backend.set_parallel_workers(workers);
                match backend.is_authorized().await {
                    Ok(true) => {
                        let _ = tx.send(BgEvent::Log("сессия уже авторизована".into()));
                        set_pending_backend(backend, None);
                        let _ = tx.send(BgEvent::CodeSent);
                        let _ = tx.send(BgEvent::Log(
                            "вы уже вошли — нажмите «Войти по коду», чтобы применить сессию".into(),
                        ));
                    }
                    _ => match backend.request_code(&phone).await {
                        Ok(token) => {
                            set_pending_backend(backend, Some(token));
                            let _ = tx.send(BgEvent::CodeSent);
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Error(format!("request_code: {e:#}")));
                        }
                    },
                }
            }
            Err(e) => {
                let _ = tx.send(BgEvent::Error(format!("connect: {e:#}")));
            }
        }
    });
}

fn submit_code(app: &mut TelecomfortApp) {
    let code = app.code.trim().to_string();
    if code.is_empty() {
        app.status = "введите код".into();
        return;
    }
    let tx = app.tx.clone();
    let rt = app.rt.clone();
    rt.spawn(async move {
        let taken = take_pending_backend();
        let Some((backend, token_opt)) = taken else {
            let _ = tx.send(BgEvent::Error("сначала нажмите «Получить код»".into()));
            return;
        };
        let Some(token) = token_opt else {
            swap_backend(&tx, backend).await;
            let _ = tx.send(BgEvent::SignedIn);
            return;
        };
        match backend.sign_in_code(&token, &code).await {
            Ok(()) => {
                swap_backend(&tx, backend).await;
                let _ = tx.send(BgEvent::SignedIn);
            }
            Err(grammers_client::SignInError::PasswordRequired(pw)) => {
                set_pending_backend_pw(backend, pw);
                let _ = tx.send(BgEvent::Log(
                    "нужен пароль 2FA — введите и нажмите «Войти с паролем»".into(),
                ));
            }
            Err(e) => {
                set_pending_backend(backend, None);
                let _ = tx.send(BgEvent::Error(format!("sign_in: {e}")));
            }
        }
    });
}

fn submit_password(app: &mut TelecomfortApp) {
    let password = app.password.trim().to_string();
    if password.is_empty() {
        app.status = "введите пароль 2FA".into();
        return;
    }
    let tx = app.tx.clone();
    let rt = app.rt.clone();
    rt.spawn(async move {
        let taken = take_pending_backend_pw();
        let Some((backend, pw)) = taken else {
            let _ = tx.send(BgEvent::Error("нет ожидающего 2FA-входа".into()));
            return;
        };
        match backend.sign_in_password(pw, &password).await {
            Ok(()) => {
                swap_backend(&tx, backend).await;
                let _ = tx.send(BgEvent::SignedIn);
            }
            Err(e) => {
                let _ = tx.send(BgEvent::Error(format!("2FA: {e:#}")));
            }
        }
    });
}

// ---- Мост pending-login через глобальную статическую ячейку ----

use std::sync::{Mutex, OnceLock};

type Pending = (
    crate::telegram::backend_grammers::GrammersBackend,
    Option<grammers_client::client::LoginToken>,
);
type PendingPw = (
    crate::telegram::backend_grammers::GrammersBackend,
    grammers_client::client::PasswordToken,
);

static PENDING: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();
static PENDING_PW: OnceLock<Mutex<Option<PendingPw>>> = OnceLock::new();

fn pending_lock() -> &'static Mutex<Option<Pending>> {
    PENDING.get_or_init(|| Mutex::new(None))
}
fn pending_pw_lock() -> &'static Mutex<Option<PendingPw>> {
    PENDING_PW.get_or_init(|| Mutex::new(None))
}

fn set_pending_backend(
    backend: crate::telegram::backend_grammers::GrammersBackend,
    token: Option<grammers_client::client::LoginToken>,
) {
    *pending_lock().lock().unwrap() = Some((backend, token));
}
fn take_pending_backend() -> Option<Pending> {
    pending_lock().lock().unwrap().take()
}
fn set_pending_backend_pw(
    backend: crate::telegram::backend_grammers::GrammersBackend,
    pw: grammers_client::client::PasswordToken,
) {
    *pending_pw_lock().lock().unwrap() = Some((backend, pw));
}
fn take_pending_backend_pw() -> Option<PendingPw> {
    pending_pw_lock().lock().unwrap().take()
}

pub(crate) async fn swap_backend(
    tx: &std::sync::mpsc::Sender<BgEvent>,
    backend: crate::telegram::backend_grammers::GrammersBackend,
) {
    *SWAP_BACKEND
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() =
        Some(std::sync::Arc::new(backend) as std::sync::Arc<dyn crate::telegram::TelegramService>);
    let _ = tx.send(BgEvent::Connected {
        backend_desc: "grammers live".into(),
    });
}

static SWAP_BACKEND: OnceLock<Mutex<Option<std::sync::Arc<dyn crate::telegram::TelegramService>>>> =
    OnceLock::new();

#[allow(dead_code)]
pub fn take_swap_backend() -> Option<std::sync::Arc<dyn crate::telegram::TelegramService>> {
    SWAP_BACKEND.get().and_then(|m| m.lock().unwrap().take())
}
