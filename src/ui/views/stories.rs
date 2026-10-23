use crate::ui::app::TelecomfortApp;

pub fn show(app: &mut TelecomfortApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Сторис · ИИ-антиреклама");
            if ui.small_button("↻ Обновить").clicked() {
                app.refresh_stories();
            }
            if ui
                .small_button("＋ Опубликовать")
                .on_hover_text("Фото/видео с устройства → сторис (видно контактам, 24 часа)")
                .clicked()
            {
                app.pick_story();
            }
            if ui
                .small_button("🥷 Стелс")
                .on_hover_text("Не палить просмотры сторис (прошлые + будущие)")
                .clicked()
            {
                app.enable_stealth();
            }
            let ads = app
                .stories
                .iter()
                .filter(|c| c.score.is_ad(app.ad_threshold))
                .count();
            ui.label(
                egui::RichText::new(format!("всего {} · рекламы {}", app.stories.len(), ads))
                    .small()
                    .weak(),
            );
        });
        ui.label(
            egui::RichText::new(
                "Текст + визуал кадра + CLIP-лайт (+ONNX при модели, batch + кэш). Видео — первый кадр: тамб/H.264/HEVC/AV1/ffmpeg. Заблюривается только рекламная зона (низ/верх). Кэш: ~/.cache/telecomfort/stories.",
            )
            .small()
            .weak(),
        );
        ui.separator();

        // Окно подтверждения публикации сторис (файл + подпись).
        if let Some(sp) = app.story_publish.as_ref() {
            let fname = sp
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut open = true;
            egui::Window::new("＋ Новая сторис")
                .collapsible(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    ui.label(format!("Файл: {fname}"));
                    ui.label(
                        egui::RichText::new("Будет видно вашим контактам (24 часа).")
                            .small()
                            .weak(),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Подпись:");
                        if let Some(sp) = app.story_publish.as_mut() {
                            ui.text_edit_singleline(&mut sp.caption);
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Опубликовать").clicked() {
                            app.do_publish_story();
                        }
                        if ui.button("Отмена").clicked() {
                            app.story_publish = None;
                        }
                    });
                });
            if !open {
                app.story_publish = None;
            }
        }

        // Окно ответа на сторис (текст уходит в чат автора).
        if app.story_reply_open {
            let target = app.story_reply_target;
            egui::Window::new("💬 Ответить на сторис")
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(
                        egui::RichText::new("Ответ уйдёт в личные сообщения автора сторис.")
                            .small()
                            .weak(),
                    );
                    ui.text_edit_multiline(&mut app.story_reply_text);
                    ui.horizontal(|ui| {
                        if ui.button("Отправить").clicked() && target.is_some() {
                            app.do_story_reply();
                        }
                        if ui.button("Отмена").clicked() {
                            app.story_reply_open = false;
                            app.story_reply_target = None;
                        }
                    });
                });
        }

        let n = app.stories.len();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for idx in 0..n {
                    let ctx = ui.ctx().clone();
                    app.ensure_story_textures(&ctx, idx);
                    let (is_ad, score, reasons, regions_n, title, caption, revealed, has_preview, visual_done, story_id, peer_id, is_video, has_poster) = {
                        let c = &app.stories[idx];
                        let is_video = c.local_path.as_ref().map(|p| crate::media::video::is_video_path(p)).unwrap_or(false);
                        let has_poster = c.local_path.as_ref().map(|p| {
                            if crate::media::video::is_video_path(p) {
                                crate::media::video::poster_path_for(p).exists()
                            } else {
                                false
                            }
                        }).unwrap_or(false);
                        (
                            c.score.is_ad(app.ad_threshold),
                            c.score.score,
                            c.score.reasons.clone(),
                            c.score.regions.len(),
                            c.info.peer_title.clone(),
                            c.info.caption.clone(),
                            c.revealed,
                            c.local_path.is_some(),
                            c.visual_done,
                            c.info.story_id,
                            c.info.peer_id,
                            is_video,
                            has_poster,
                        )
                    };
                    ui.group(|ui| {
                        ui.set_min_width(280.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(title.clone()).strong());
                                if is_ad {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "РЕКЛАМА {:.0}%",
                                            score * 100.0
                                        ))
                                        .color(egui::Color32::from_rgb(220, 60, 60))
                                        .strong(),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new(format!("ок {:.0}%", score * 100.0))
                                            .weak()
                                            .small(),
                                    );
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(if has_preview {
                                        if is_video { "▶ видео" } else { "▣ кадр" }
                                    } else {
                                        "▨ заглушка"
                                    })
                                    .small()
                                    .weak(),
                                );
                                if is_video {
                                    ui.label(
                                        egui::RichText::new(if has_poster { "🎞 кадр ✓" } else { "🎞 кадр …" })
                                            .small()
                                            .weak(),
                                    );
                                }
                                if visual_done {
                                    ui.label(
                                        egui::RichText::new("👁 визуал ✓").small().weak(),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new("👁 визуал …").small().weak(),
                                    );
                                }
                                ui.label(
                                    egui::RichText::new(format!("#{story_id} · {peer_id}"))
                                        .small()
                                        .weak(),
                                );
                                // Свои сторис — можно удалить (с подтверждением).
                                if app.me_id == Some(peer_id) {
                                    if ui
                                        .small_button("🗑 Удалить")
                                        .on_hover_text("Удалить свою сторис")
                                        .clicked()
                                    {
                                        app.confirm_action = Some(
                                            crate::ui::app::ConfirmAction::DeleteStories {
                                                story_ids: vec![story_id],
                                            },
                                        );
                                    }
                                    // Просмотры своей сторис (лениво, один раз).
                                    match app.story_views.get(&story_id).copied() {
                                        Some(v) => {
                                            ui.label(
                                                egui::RichText::new(format!("👁 {v}"))
                                                    .small()
                                                    .weak(),
                                            );
                                        }
                                        None => {
                                            if ui
                                                .small_button("👁")
                                                .on_hover_text("Показать просмотры")
                                                .clicked()
                                            {
                                                app.load_story_views(story_id);
                                            }
                                        }
                                    }
                                } else if ui
                                    .small_button("💬 Ответить")
                                    .on_hover_text("Ответить на сторис текстом (уйдёт в чат автора)")
                                    .clicked()
                                {
                                    app.story_reply_target = Some((peer_id, story_id));
                                    app.story_reply_text.clear();
                                    app.story_reply_open = true;
                                }
                            });
                            let show_blurred = is_ad && app.auto_hide_ads && !revealed;
                            let tex = {
                                let c = &app.stories[idx];
                                if show_blurred {
                                    c.texture_blurred.clone()
                                } else {
                                    c.texture.clone()
                                }
                            };
                            if let Some(tex) = tex {
                                let size = tex.size_vec2();
                                let max_w = 260.0;
                                let scale = (max_w / size.x).min(1.0);
                                ui.image((tex.id(), egui::Vec2::new(size.x * scale, size.y * scale)));
                                if show_blurred {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "🙈 скрыта рекламная зона ({regions_n})"
                                        ))
                                        .small()
                                        .weak(),
                                    );
                                    if ui.small_button("Показать всё равно").clicked() {
                                        app.stories[idx].revealed = true;
                                    }
                                } else if is_ad
                                    && ui.small_button("Скрыть снова").clicked()
                                {
                                    app.stories[idx].revealed = false;
                                }
                            }
                            if !caption.is_empty() {
                                ui.label(crate::utils::truncate(&caption, 220));
                            }
                            if !reasons.is_empty() {
                                ui.label(
                                    egui::RichText::new(format!("почему: {}", reasons.join("; ")))
                                        .small()
                                        .weak(),
                                );
                            }
                        });
                    });
                }
                if app.stories.is_empty() {
                    ui.label("сторис пока нет — нажмите «Обновить»");
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(
                    "Порог и сила блюра — в Настройках. Кнопка ниже сбрасывает кэш превью.",
                )
                .small()
                .weak(),
            );
            if ui.small_button("Очистить кэш превью").clicked() {
                let dir = crate::telegram::stories_cache_dir();
                let removed = std::fs::remove_dir_all(&dir).is_ok();
                let _ = std::fs::create_dir_all(&dir);
                app.log(if removed {
                    "кэш превью сторис очищен"
                } else {
                    "кэш уже пуст"
                });
                for c in &mut app.stories {
                    c.texture = None;
                    c.texture_blurred = None;
                    if c.local_path.as_ref().map(|p| p.starts_with(&dir)).unwrap_or(false) {
                        c.local_path = None;
                        c.visual_done = false;
                    }
                }
            }
        });
    });
}
