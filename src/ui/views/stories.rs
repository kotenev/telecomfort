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
            if ui
                .small_button("🗂 Подборки")
                .on_hover_text("Хайлайтс-подборки своих сторис")
                .clicked()
            {
                app.albums_loaded = false;
                app.load_albums();
            }
            if ui
                .small_button("＋ Подборка")
                .on_hover_text("Создать подборку из своих сторис")
                .clicked()
            {
                app.album_create_open = true;
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
        // Окно правки подписи своей сторис.
        if let Some(sid) = app.story_edit_open {
            egui::Window::new("✏️ Подпись сторис").collapsible(false).show(ui.ctx(), |ui| {
                ui.text_edit_multiline(&mut app.story_edit_text);
                ui.horizontal(|ui| {
                    if ui.button("Сохранить").clicked() {
                        app.save_story_caption(sid);
                    }
                    if ui.button("Отмена").clicked() {
                        app.story_edit_open = None;
                    }
                });
            });
        }
        // Окно зрителей своей сторис.
        if let Some(sid) = app.story_viewers_open {
            let (count, names) =
                app.story_viewers.get(&sid).cloned().unwrap_or((0, vec![]));
            egui::Window::new(format!("👁 Просмотры ({count})"))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    if names.is_empty() {
                        ui.label("пока никто / загрузка…");
                    }
                    for n in names.iter().take(50) {
                        ui.label(format!("• {n}"));
                    }
                    if ui.button("Закрыть").clicked() {
                        app.story_viewers_open = None;
                    }
                });
        }

        if !app.albums.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label("🗂");
                for a in app.albums.clone() {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new(&a.title).strong());
                        ui.label(egui::RichText::new(format!("{} шт.", a.story_count)).small().weak());
                        if ui.small_button("🗑").on_hover_text("Удалить подборку").clicked() {
                            app.delete_album(a.album_id);
                        }
                    });
                }
            });
        }
        // Окно создания подборки: название + выбор своих сторис.
        if app.album_create_open {
            let own: Vec<(i32, String)> = app
                .stories
                .iter()
                .filter(|c| Some(c.info.peer_id) == app.me_id)
                .map(|c| (c.info.story_id, c.info.caption.clone()))
                .collect();
            egui::Window::new("＋ Новая подборка").collapsible(false).show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Название:");
                    ui.text_edit_singleline(&mut app.album_create_title);
                });
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (sid, cap) in own {
                        let mut sel = app.album_pick.contains(&sid);
                        let label = if cap.is_empty() { format!("#{sid}") } else { crate::utils::truncate(&cap, 50) };
                        if ui.checkbox(&mut sel, label).changed() {
                            if sel {
                                app.album_pick.insert(sid);
                            } else {
                                app.album_pick.remove(&sid);
                            }
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Создать").clicked() {
                        app.do_create_album();
                    }
                    if ui.button("Отмена").clicked() {
                        app.album_create_open = false;
                        app.album_pick.clear();
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
                                    if ui
                                        .small_button("✏️")
                                        .on_hover_text("Изменить подпись")
                                        .clicked()
                                    {
                                        app.story_edit_open = Some(story_id);
                                        app.story_edit_text.clear();
                                    }
                                    // Просмотры своей сторис (лениво, один раз; тап — кто смотрел).
                                    match app.story_views.get(&story_id).copied() {
                                        Some(v) => {
                                            if ui
                                                .small_button(format!("👁 {v}"))
                                                .on_hover_text("Кто смотрел")
                                                .clicked()
                                            {
                                                app.load_story_viewers(story_id);
                                            }
                                        }
                                        None => {
                                            if ui
                                                .small_button("👁")
                                                .on_hover_text("Показать просмотры")
                                                .clicked()
                                            {
                                                app.load_story_viewers(story_id);
                                            }
                                        }
                                    }
                                    if ui
                                        .small_button("📌")
                                        .on_hover_text("Закрепить/открепить в профиле")
                                        .clicked()
                                    {
                                        app.toggle_story_pin(story_id, false);
                                    }
                                } else {
                                    if ui
                                        .small_button("💬 Ответить")
                                        .on_hover_text("Ответить на сторис текстом (уйдёт в чат автора)")
                                        .clicked()
                                    {
                                        app.story_reply_target = Some((peer_id, story_id));
                                        app.story_reply_text.clear();
                                        app.story_reply_open = true;
                                    }
                                    if ui
                                        .small_button("❤️")
                                        .on_hover_text("Реакция на сторис")
                                        .clicked()
                                    {
                                        app.story_react(peer_id, story_id, "❤️");
                                    }
                                    if ui
                                        .small_button("🙈")
                                        .on_hover_text("Скрыть сторис этого автора из ленты")
                                        .clicked()
                                    {
                                        app.hide_peer_stories(peer_id, true);
                                    }
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
