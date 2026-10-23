use crate::config::MediaKind;
use crate::ui::app::{BgEvent, TelecomfortApp};

pub fn show(app: &mut TelecomfortApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Настройки");
            ui.separator();

            ui.collapsing("🔑 Telegram (MTProto) · профили", |ui| {
                super::login::show(app, ui);
            });

            ui.collapsing("👤 Профиль (имя + о себе)", |ui| {
                if !app.profile_loaded {
                    app.load_profile();
                }
                ui.horizontal(|ui| {
                    ui.label("Имя:");
                    ui.text_edit_singleline(&mut app.profile_first);
                    ui.label("Фамилия:");
                    ui.text_edit_singleline(&mut app.profile_last);
                });
                ui.horizontal(|ui| {
                    ui.label("О себе:");
                    ui.text_edit_singleline(&mut app.profile_about);
                });
                if ui.small_button("💾 Сохранить профиль").clicked() {
                    app.save_profile();
                }
                ui.label(
                    egui::RichText::new("Имя/о себе видны всем, у кого вы в контактах.")
                        .small()
                        .weak(),
                );
            });

            ui.collapsing("📱 Активные сессии", |ui| {
                if !app.sessions_loaded {
                    app.load_sessions();
                }
                if app.sessions.is_empty() {
                    ui.label(
                        egui::RichText::new("сессии не загружены — нажмите ↻")
                            .small()
                            .weak(),
                    );
                }
                for s in app.sessions.clone() {
                    ui.horizontal(|ui| {
                        let cur = if s.current { " · текущая" } else { "" };
                        ui.label(
                            egui::RichText::new(format!("{}{}", s.app_name, cur)).strong(),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "{} · {} · {}{}",
                                s.device,
                                s.platform,
                                s.country,
                                if s.ip.is_empty() { String::new() } else { format!(" · {}", s.ip) }
                            ))
                            .small()
                            .weak(),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    if ui.small_button("↻ Обновить").clicked() {
                        app.sessions_loaded = false;
                        app.load_sessions();
                    }
                    if ui
                        .small_button("⏏ Завершить другие")
                        .on_hover_text("auth.resetAuthorizations: все сессии, кроме текущей")
                        .clicked()
                    {
                        app.terminate_others();
                    }
                });
            });

            ui.collapsing("✨ Сторис · анти-реклама", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Порог рекламы:");
                    ui.add(egui::Slider::new(&mut app.ad_threshold, 0.0..=1.0).text("threshold"));
                });
                ui.horizontal(|ui| {
                    ui.label("Сила блюра:");
                    ui.add(egui::Slider::new(&mut app.blur_sigma, 1.0..=20.0).text("sigma"));
                });
                ui.checkbox(
                    &mut app.blur_only_regions,
                    "Блюрить только рекламные зоны (а не всё фото)",
                );
                ui.checkbox(&mut app.auto_hide_ads, "Автоскрывать рекламу");
                ui.label(
                    egui::RichText::new(
                        "Текст: CTA (RU/EN), URL, @username, цены/скидки, промокоды, телефоны, эмодзи-спам, КАПС. Визуал: палитра, плашки, контраст + QR (буст 0..0.45). Сверху: CLIP-лайт 0..0.25 и ONNX 0..0.30. Итог = сумма, clamp 0..0.98.",
                    )
                    .small()
                    .weak(),
                );
                if ui.small_button("Пересчитать оценки").clicked() {
                    app.log("порог применён к отображению (для полного пересчёта нажмите «Обновить» в Сторис)");
                }
                ui.separator();
                ui.checkbox(&mut app.clip_enabled, "CLIP-лайт (текст↔кадр, чистый Rust, без модели)");
                ui.label(
                    egui::RichText::new("Двухбашенный эмбеддинг 32 dim: промо-лексика + визуальные сигналы → близость к прототипу «реклама».")
                        .small()
                        .weak(),
                );
            });

            ui.collapsing("🧠 ONNX · видео-превью", |ui| {
                ui.label(
                    egui::RichText::new(format!("Статус: {}", crate::ai::ort_status(&app.onnx_model, app.onnx_enabled, &app.onnx_provider)))
                        .small(),
                );
                ui.checkbox(&mut app.onnx_enabled, "Включить ONNX-скоринг (нужна сборка с --features ort)");
                ui.horizontal(|ui| {
                    ui.label("Модель (.onnx):");
                    ui.text_edit_singleline(&mut app.onnx_model);
                    if ui.small_button("Выбрать…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("ONNX", &["onnx"]).pick_file() {
                            app.onnx_model = f.to_string_lossy().to_string();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Имя входа (пусто = авто):");
                    ui.text_edit_singleline(&mut app.onnx_input);
                });
                ui.horizontal(|ui| {
                    ui.label("Execution provider:");
                    egui::ComboBox::from_id_salt("onnx_ep")
                        .selected_text(app.onnx_provider.clone())
                        .show_ui(ui, |ui| {
                            for c in crate::ai::available_providers() {
                                ui.selectable_value(&mut app.onnx_provider, c.to_string(), c);
                            }
                        });
                    ui.label(
                        egui::RichText::new("в этом бинарнике: CPU (+ собранные ort-*). GPU: пересоберите с --features ort,ort-coreml / ort-cuda")
                            .small()
                            .weak(),
                    );
                });
                ui.label(
                    egui::RichText::new("Имена входов/выходов, dtype и shape логгируются при загрузке модели (см. журнал). Поддерживаются float32 0..1, uint8/int8, int32/int64 и float16 входы, NCHW/NHWC, динамические dims → 224. Квантованные выходы (u8/i8) маппятся в 0..1.")
                        .small()
                        .weak(),
                );
                ui.label(
                    egui::RichText::new("Сборка: cargo run --features ort. Без модели/фичи — автоматический fallback на CLIP-лайт, скор не выдумывается. Своя модель: подставьте имена входов в ai/onnx.rs::run_model.")
                        .small()
                        .weak(),
                );
                ui.separator();
                ui.label(egui::RichText::new(format!("ffmpeg: {}", app.ffmpeg_status)).small());
                ui.label(
                    egui::RichText::new("Видеосторис: первый кадр — MTProto-тамб → встроенные H.264/HEVC/AV1-декодеры (чистый Rust, AV1: MP4 av01 + WebM/MKV) → обложка covr → внешний ffmpeg (scale 270x480) в story_<peer>_<id>.poster.jpg → скоринг/блюр. Без всего — плейсхолдер с кинополосами.")
                        .small()
                        .weak(),
                );
            });

            ui.collapsing("⬇ Автозагрузки · правила per-chat", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Папка загрузок:");
                    ui.text_edit_singleline(&mut app.download_dir);
                    if ui.small_button("Выбрать…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            app.download_dir = dir.to_string_lossy().to_string();
                        }
                    }
                });
                ui.label(
                    egui::RichText::new(
                        "Из закрытых групп с запретом копирования качаем напрямую через MTProto (upload.getFile). Сервер отдаёт файл, т.к. у аккаунта есть доступ к чату.",
                    )
                    .small()
                    .weak(),
                );
                ui.horizontal(|ui| {
                    ui.label("Параллельных чанков MTProto на файл:");
                    ui.add(egui::DragValue::new(&mut app.cfg.downloader.parallel_chunks).range(1..=8));
                    ui.label(
                        egui::RichText::new("(применяется при следующем входе; файлы хранят .part для докачки)")
                            .small()
                            .weak(),
                    );
                });
                ui.separator();
                ui.strong("Правила (первое совпавшее срабатывает):");
                let mut remove_idx: Option<usize> = None;
                let mut toggle_save = false;
                for (i, rule) in app.cfg.downloader.rules.iter_mut().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("#{i}"));
                            ui.text_edit_singleline(&mut rule.chat);
                            ui.checkbox(&mut rule.enabled, "вкл");
                            if ui.small_button("🗑").on_hover_text("Удалить правило").clicked() {
                                remove_idx = Some(i);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Типы:");
                            for (label, kind) in [
                                ("Any", MediaKind::Any),
                                ("Фото", MediaKind::Photo),
                                ("Видео", MediaKind::Video),
                                ("Аудио", MediaKind::Audio),
                                ("Файлы", MediaKind::Document),
                            ] {
                                let mut has = rule.kinds.contains(&kind);
                                if ui.checkbox(&mut has, label).changed() {
                                    if has && !rule.kinds.contains(&kind) {
                                        // Any взаимоисключающ с конкретными
                                        if kind == MediaKind::Any {
                                            rule.kinds.clear();
                                            rule.kinds.push(MediaKind::Any);
                                        } else {
                                            rule.kinds.retain(|k| *k != MediaKind::Any);
                                            rule.kinds.push(kind.clone());
                                        }
                                    } else if !has {
                                        rule.kinds.retain(|k| *k != kind);
                                    }
                                    toggle_save = true;
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                    ui.label("Макс МБ (0 = ∞):");
                    ui.add(egui::DragValue::new(&mut rule.max_size_mb).range(0..=10000));
                            ui.checkbox(&mut rule.bypass_protected, "обход noforwards");
                        });
                    });
                }
                if let Some(i) = remove_idx {
                    app.cfg.downloader.rules.remove(i);
                    let _ = app.cfg.save();
                    app.log("правило удалено");
                } else if toggle_save {
                    // kinds отредактированы — без автосохранения, ждём «Сохранить».
                }
                if app.cfg.downloader.rules.is_empty() {
                    ui.label("нет правил — добавьте ниже, иначе ничего качаться не будет");
                }
                ui.separator();
                ui.strong("Новое правило:");
                ui.horizontal(|ui| {
                    ui.label("Чат (id / название / *):");
                    ui.text_edit_singleline(&mut app.new_rule_chat);
                    ui.label("Макс МБ:");
                    ui.text_edit_singleline(&mut app.new_rule_max);
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut app.new_rule_photo, "Фото");
                    ui.checkbox(&mut app.new_rule_video, "Видео");
                    ui.checkbox(&mut app.new_rule_audio, "Аудио");
                    ui.checkbox(&mut app.new_rule_doc, "Файлы/книги");
                    if ui.button("+ Добавить правило").clicked() {
                        let mut kinds = vec![];
                        if app.new_rule_photo {
                            kinds.push(MediaKind::Photo);
                        }
                        if app.new_rule_video {
                            kinds.push(MediaKind::Video);
                        }
                        if app.new_rule_audio {
                            kinds.push(MediaKind::Audio);
                        }
                        if app.new_rule_doc {
                            kinds.push(MediaKind::Document);
                        }
                        if kinds.is_empty() {
                            kinds.push(MediaKind::Any);
                        }
                        let max_mb = app.new_rule_max.trim().parse().unwrap_or(1500);
                        let chat = if app.new_rule_chat.trim().is_empty() {
                            "*".to_string()
                        } else {
                            app.new_rule_chat.trim().to_string()
                        };
                        app.cfg.downloader.rules.push(crate::config::AutodownloadRule {
                            chat,
                            kinds,
                            max_size_mb: max_mb,
                            enabled: true,
                            bypass_protected: true,
                        });
                        match app.cfg.save() {
                            Ok(_) => app.log("правило добавлено"),
                            Err(e) => app.log(format!("не сохранено: {e:#}")),
                        }
                    }
                });
                ui.label(
                    egui::RichText::new("Подсказка: чат можно добавить кнопкой «+ Правило для этого чата» на экране Чатов. Доступны «*» (все), точное название или числовой id.")
                        .small()
                        .weak(),
                );
            });

            ui.collapsing("🔄 Фоновая синхронизация", |ui| {
                ui.checkbox(&mut app.sync_enabled, "Включить фон (диалоги/сторис/автозагрузка)");
                ui.horizontal(|ui| {
                    ui.label("Интервал, сек (≥15):");
                    ui.text_edit_singleline(&mut app.sync_interval);
                });
                ui.checkbox(&mut app.sync_auto_new, "Автокачать новые медиа по правилам");
                ui.checkbox(&mut app.notify_dl, "Уведомлять о новых файлах");
                ui.checkbox(&mut app.notify_ad, "Уведомлять о новой рекламе в сторис");
                ui.horizontal(|ui| {
                    if ui.button("▶ Синхронизировать сейчас").clicked() {
                        app.sync_form_to_cfg();
                        app.run_background_once();
                    }
                    ui.label(
                        egui::RichText::new(format!("последний синк: {}", app.last_sync))
                            .small()
                            .weak(),
                    );
                });
                ui.label(
                    egui::RichText::new("Фон: каждые N сек обновляет диалоги/сторис, докачивает только новые message_id (watermark в sync_state), шлёт десктоп-уведомления.")
                        .small()
                        .weak(),
                );
            });

            ui.collapsing("☁ Яндекс Диск (WebDAV · стриминг)", |ui| {
                ui.horizontal(|ui| {
                    ui.label("OAuth-токен:");
                    ui.text_edit_singleline(&mut app.yandex_token);
                });
                ui.horizontal(|ui| {
                    ui.label("Папка:");
                    ui.text_edit_singleline(&mut app.yandex_folder);
                });
                ui.checkbox(&mut app.yandex_auto, "Авто-загрузка файлов на Диск");
                ui.label(
                    egui::RichText::new("Файлы >8 МБ стримятся чанками по 1 МБ (без чтения в RAM), прогресс — в Загрузках. Ретраи ×3 с бэкоффом 2/4с. Параллельно — до 4 заливок (семафор).")
                        .small()
                        .weak(),
                );
                if ui.small_button("Проверить папку").clicked() {
                    let token = app.yandex_token.clone();
                    let folder = app.yandex_folder.clone();
                    let tx = app.tx.clone();
                    let rt = app.rt.clone();
                    rt.spawn(async move {
                        match crate::integrations::yandex::ensure_folder(&token, &folder).await {
                            Ok(_) => {
                                let _ = tx.send(BgEvent::Log(format!("Яндекс Диск OK: {folder}")));
                            }
                            Err(e) => {
                                let _ = tx.send(BgEvent::Error(format!("Яндекс: {e:#}")));
                            }
                        }
                    });
                }
            });

            ui.collapsing("📚 Calibre (дедупликация книг)", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Библиотека:");
                    ui.text_edit_singleline(&mut app.calibre_lib);
                    if ui.small_button("Выбрать…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            app.calibre_lib = dir.to_string_lossy().to_string();
                        }
                    }
                });
                ui.checkbox(&mut app.calibre_auto, "Авто-импорт книг (epub/fb2/pdf/mobi)");
                ui.checkbox(&mut app.calibre_delete, "Удалять файл после импорта");
                ui.label(
                    egui::RichText::new(
                        "Дубли отсекаются по sha256 + нормализованному названию (calibredb list). Требуется установленный Calibre (calibredb в PATH).",
                    )
                    .small()
                    .weak(),
                );
            });

            ui.collapsing("📦 Все настройки одним файлом", |ui| {
                ui.label(
                    egui::RichText::new("Экспорт: профили, правила, сторис/синк/яндекс/calibre — один TOML. Импорт: валидация + применение + рестарт фона.")
                        .small()
                        .weak(),
                );
                ui.horizontal(|ui| {
                    if ui.button("📦 Экспорт настроек…").clicked() {
                        app.export_settings_bundle();
                    }
                    if ui.button("📥 Импорт настроек…").clicked() {
                        app.import_settings_bundle();
                    }
                });
            });

            ui.separator();
            if ui.button("💾 Сохранить настройки").clicked() {
                app.save_settings();
            }
            ui.separator();
            ui.label(egui::RichText::new(format!("Статус: {}", app.status)).small());
        });
    });
}
