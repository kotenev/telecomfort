use crate::ui::app::TelecomfortApp;

pub fn show(app: &mut TelecomfortApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Загрузки");
        ui.label(
            egui::RichText::new(
                "Автозагрузки из закрытых групп → локальная папка → (опционально) Яндекс Диск → (книги) Calibre с дедупликацией.",
            )
            .small()
            .weak(),
        );
        let (total, size, dupes) = app.db.stats();
        ui.horizontal(|ui| {
            ui.label(format!("файлов: {total}"));
            ui.separator();
            ui.label(format!("объём: {}", crate::utils::human_size(size.max(0) as u64)));
            ui.separator();
            ui.label(format!("уникальных хэшей: {dupes}"));
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Папка:");
            ui.text_edit_singleline(&mut app.download_dir);
            if ui.small_button("Открыть").clicked() {
                let _ = open::that(app.download_dir.clone());
            }
            if ui.small_button("Выбрать…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    app.download_dir = dir.to_string_lossy().to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Поиск:");
            ui.text_edit_singleline(&mut app.dl_query);
            egui::ComboBox::from_label("Фильтр")
                .selected_text(app.dl_filter.clone())
                .show_ui(ui, |ui| {
                    for f in ["Все", "Видео", "Аудио", "Книги", "Яндекс ✓", "Calibre ✓"] {
                        ui.selectable_value(&mut app.dl_filter, f.to_string(), f);
                    }
                });
            if ui.small_button("✕").on_hover_text("Сбросить фильтры").clicked() {
                app.dl_query.clear();
                app.dl_filter = "Все".into();
            }
        });
        // Активные передачи: скачивание MTProto (чанки+resume) и заливки на Яндекс.
        if !app.uploads.is_empty() {
            ui.separator();
            ui.heading("Активные передачи (live)");
            // Чистим завершённые (100%), чтобы не висели.
            app.uploads.retain(|_, (d, t)| *d < (*t).max(1));
            let mut keys: Vec<String> = app.uploads.keys().cloned().collect();
            keys.sort();
            for k in keys {
                if let Some((done, total)) = app.uploads.get(&k) {
                    let frac = (*done as f32 / (*total).max(1) as f32).clamp(0.0, 1.0);
                    ui.horizontal(|ui| {
                        let name = std::path::Path::new(&k)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or(k.clone());
                        ui.label(egui::RichText::new(format!("☁ {name}")).small().monospace());
                        ui.add(
                            egui::ProgressBar::new(frac)
                                .text(format!(
                                    "{:.0}% · {}/{}",
                                    frac * 100.0,
                                    crate::utils::human_size(*done),
                                    crate::utils::human_size(*total)
                                ))
                                .desired_width(320.0),
                        );
                    });
                }
            }
        }
        ui.separator();
        // Фильтрация — до групп (группы кластеризуют уже отфильтрованное).
        let rows = app.db.recent_downloads(200);
        let q = app.dl_query.to_lowercase();
        let filtered: Vec<_> = rows
            .into_iter()
            .filter(|r| {
                if !q.is_empty()
                    && !(r.file_name.to_lowercase().contains(&q)
                        || r.chat.to_lowercase().contains(&q)
                        || r.local_path.to_lowercase().contains(&q))
                {
                    return false;
                }
                match app.dl_filter.as_str() {
                    "Видео" => {
                        let e = std::path::Path::new(&r.file_name)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        matches!(e.as_str(), "mp4" | "mkv" | "avi" | "mov" | "webm")
                    }
                    "Аудио" => {
                        let e = std::path::Path::new(&r.file_name)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        matches!(e.as_str(), "mp3" | "ogg" | "oga" | "opus" | "m4a" | "flac" | "wav")
                    }
                    "Книги" => crate::utils::is_ebook(std::path::Path::new(&r.file_name)),
                    "Яндекс ✓" => !r.yandex_path.is_empty(),
                    "Calibre ✓" => r.calibre_imported,
                    _ => true,
                }
            })
            .collect();
        // --- Похожие файлы: связные pHash-группы с предпросмотром ---
        // Кластеризация (ai::cluster: DSU + BK-дерево) поверх текущего фильтра:
        // один мем, пересланный пять раз с разным ресайзом, — одна группа.
        {
            let hash_rows: Vec<(usize, u64)> = filtered
                .iter()
                .enumerate()
                .filter_map(|(i, r)| r.phash.map(|h| (i, h as u64)))
                .collect();
            if hash_rows.len() >= 2 {
                let hashes: Vec<u64> = hash_rows.iter().map(|(_, h)| *h).collect();
                let groups = crate::ai::cluster::cluster_indices(
                    &hashes,
                    crate::ai::phash::MAX_HAMMING,
                );
                if !groups.is_empty() {
                    let in_groups: usize = groups.iter().map(|g| g.len()).sum();
                    ui.heading(format!(
                        "Похожие (pHash): групп {}, файлов {in_groups}",
                        groups.len()
                    ));
                    ui.label(
                        egui::RichText::new(
                            "Связные группы визуальных дублей: ресайз/рекомпресс Telegram повторы не прячет. Раскройте группу для предпросмотра и быстрых действий.",
                        )
                        .small()
                        .weak(),
                    );
                    let ctx = ui.ctx().clone();
                    for (gi, g) in groups.iter().enumerate().take(50) {
                        let preview_names: Vec<String> = g
                            .iter()
                            .take(3)
                            .map(|&m| {
                                filtered[hash_rows[m].0]
                                    .file_name
                                    .clone()
                            })
                            .collect();
                        let mut title = format!(
                            "Группа {} · файлов {} · {}",
                            gi + 1,
                            g.len(),
                            preview_names.join(", ")
                        );
                        if g.len() > 3 {
                            title.push('…');
                        }
                        egui::CollapsingHeader::new(title)
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    const MAX_SHOWN: usize = 24;
                                    for &m in g.iter().take(MAX_SHOWN) {
                                        let r = filtered[hash_rows[m].0].clone();
                                        ui.group(|ui| {
                                            ui.set_min_width(132.0);
                                            ui.vertical(|ui| {
                                                if let Some(tex) =
                                                    app.ensure_dl_thumb(&ctx, &r.local_path)
                                                {
                                                    let size = tex.size_vec2();
                                                    let scale =
                                                        (112.0 / size.x.max(1.0)).min(112.0 / size.y.max(1.0));
                                                    ui.image((
                                                        tex.id(),
                                                        egui::Vec2::new(size.x * scale, size.y * scale),
                                                    ));
                                                } else {
                                                    ui.label(
                                                        egui::RichText::new("нет превью")
                                                            .small()
                                                            .weak(),
                                                    );
                                                }
                                                ui.label(
                                                    egui::RichText::new(crate::utils::truncate(
                                                        &r.file_name,
                                                        22,
                                                    ))
                                                    .small()
                                                    .strong(),
                                                );
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{} · {}",
                                                        crate::utils::human_size(
                                                            r.file_size.max(0) as u64
                                                        ),
                                                        r.chat
                                                    ))
                                                    .small()
                                                    .weak(),
                                                );
                                                ui.horizontal(|ui| {
                                                    let lp = r.local_path.clone();
                                                    if ui.small_button("▶").on_hover_text("Открыть файл").clicked() {
                                                        let _ = open::that(lp.clone());
                                                    }
                                                    if ui.small_button("📂").on_hover_text("Показать в папке").clicked() {
                                                        let _ = open::that(
                                                            std::path::Path::new(&lp)
                                                                .parent()
                                                                .map(|x| x.to_path_buf())
                                                                .unwrap_or_else(|| {
                                                                    std::path::PathBuf::from(".")
                                                                }),
                                                        );
                                                    }
                                                    if crate::utils::is_ebook(std::path::Path::new(&r.file_name)) {
                                                        if ui.small_button("📚").on_hover_text("В Calibre").clicked() {
                                                            app.reimport_to_calibre(lp.clone());
                                                        }
                                                    }
                                                    if ui.small_button("☁").on_hover_text("На Яндекс Диск").clicked() {
                                                        app.reupload_to_yandex(lp.clone());
                                                    }
                                                });
                                            });
                                        });
                                    }
                                    if g.len() > MAX_SHOWN {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "+ ещё {} (уточните Поиском)",
                                                g.len() - MAX_SHOWN
                                            ))
                                            .small()
                                            .weak(),
                                        );
                                    }
                                });
                            });
                    }
                    if groups.len() > 50 {
                        ui.label(
                            egui::RichText::new(format!(
                                "…и ещё {} групп (уточните Поиском)",
                                groups.len() - 50
                            ))
                            .small()
                            .weak(),
                        );
                    }
                    ui.separator();
                }
            }
        }
        ui.heading("Последние файлы (SQLite)");
        if filtered.is_empty() {
            ui.label("ничего не найдено — измените поиск/фильтр");
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for r in filtered {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(r.file_name.clone()).strong());
                        ui.label(
                            egui::RichText::new(crate::utils::human_size(r.file_size.max(0) as u64))
                                .small()
                                .weak(),
                        );
                    });
                    ui.label(
                        egui::RichText::new(format!("{} · msg {} · {}", r.chat, r.message_id, r.created_at))
                            .small()
                            .weak(),
                    );
                    ui.label(egui::RichText::new(r.local_path.clone()).small().monospace());
                    ui.horizontal(|ui| {
                        if let Some(fname) = std::path::Path::new(&r.local_path).file_name() {
                            ui.label(fname.to_string_lossy().to_string());
                        }
                        if r.calibre_imported {
                            ui.label(egui::RichText::new("→ Calibre ✓").small());
                        }
                    });
                    if !r.near_duplicate_of.is_empty() {
                        let other = std::path::Path::new(&r.near_duplicate_of)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| r.near_duplicate_of.clone());
                        ui.label(
                            egui::RichText::new(format!("≈ похож на {other} (pHash)"))
                                .small()
                                .weak(),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("📂 Показать").clicked() {
                            let p = r.local_path.clone();
                            let _ = open::that(
                                std::path::Path::new(&p)
                                    .parent()
                                    .map(|x| x.to_path_buf())
                                    .unwrap_or_else(|| std::path::PathBuf::from(".")),
                            );
                        }
                        if ui.small_button("▶ Открыть").clicked() {
                            let _ = open::that(r.local_path.clone());
                        }
                        let lp = r.local_path.clone();
                        if ui
                            .small_button("☁ На Яндекс")
                            .on_hover_text("Повторно залить на Яндекс Диск")
                            .clicked()
                        {
                            app.reupload_to_yandex(lp.clone());
                        }
                        if crate::utils::is_ebook(std::path::Path::new(&r.file_name)) {
                            let lp2 = r.local_path.clone();
                            if ui.small_button("📚 В Calibre").clicked() {
                                app.reimport_to_calibre(lp2);
                            }
                        }
                    });
                });
            }
        });
        ui.separator();
        ui.heading("Журнал");
        ui.horizontal(|ui| {
            if ui.small_button("Очистить журнал").clicked() {
                app.logs.clear();
            }
            if ui.small_button("Синхронизировать сейчас").clicked() {
                app.run_background_once();
            }
        });
        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
            for l in app.logs.iter().rev().take(80) {
                ui.label(egui::RichText::new(l).small().monospace());
            }
        });
    });
}
