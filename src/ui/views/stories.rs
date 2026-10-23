use crate::ui::app::TelecomfortApp;

pub fn show(app: &mut TelecomfortApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Сторис · ИИ-антиреклама");
            if ui.small_button("↻ Обновить").clicked() {
                app.refresh_stories();
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
