//! Офлайн mock: позволяет открыть UI без API-ключей и показать все фичи.

use super::{AccountInfo, ChatInfo, MessageInfo, StoryInfo, TelegramService};
use async_trait::async_trait;

pub struct MockTelegram;

#[async_trait]
impl TelegramService for MockTelegram {
    async fn dialogs(&self) -> anyhow::Result<Vec<ChatInfo>> {
        Ok(vec![
            ChatInfo {
                id: -1001,
                title: "Закрытая группа «Книги» 🔒".into(),
                username: None,
                protected: true,
                unread: 12,
            },
            ChatInfo {
                id: -1002,
                title: "Учёба / Лекции".into(),
                username: None,
                protected: true,
                unread: 3,
            },
            ChatInfo {
                id: 2001,
                title: "Друзья".into(),
                username: Some("friends".into()),
                protected: false,
                unread: 0,
            },
        ])
    }

    async fn history(&self, chat_id: i64, _limit: usize) -> anyhow::Result<Vec<MessageInfo>> {
        Ok(vec![
            MessageInfo {
                id: 101,
                chat_id,
                text: "Лекция 12 — видеозапись".into(),
                date: 0,
                has_media: true,
                media_kind: Some("video".into()),
                file_name: Some("lecture12.mp4".into()),
                file_size: Some(180_000_000),
                outgoing: false,
                sender: Some("Лектор".into()),
                edited: false,
                reactions: Some(5),
            },
            MessageInfo {
                id: 102,
                chat_id,
                text: "Учебник, только для участников (копирование запрещено)".into(),
                date: 0,
                has_media: true,
                media_kind: Some("document".into()),
                file_name: Some("phys_levels.epub".into()),
                file_size: Some(4_200_000),
                outgoing: true,
                sender: None,
                edited: true,
                reactions: None,
            },
            MessageInfo {
                id: 103,
                chat_id,
                text: "Аудио-разбор домашки".into(),
                date: 0,
                has_media: true,
                media_kind: Some("audio".into()),
                file_name: Some("hw_voice.ogg".into()),
                file_size: Some(2_000_000),
                outgoing: false,
                sender: Some("Анна".into()),
                edited: false,
                reactions: None,
            },
        ])
    }

    async fn stories(&self) -> anyhow::Result<Vec<StoryInfo>> {
        Ok(vec![
            StoryInfo {
                peer_id: 3001,
                peer_title: "Магазин «Всё по 99»".into(),
                story_id: 1,
                caption: "🔥🔥🔥 СКИДКА -50% только сегодня! Переходи по ссылке https://shop.example, промокод SALE10 @shop".into(),
                has_url: true,
                date: 0,
                preview_path: None,
            },
            StoryInfo {
                peer_id: 3002,
                peer_title: "Анна".into(),
                story_id: 2,
                caption: "Гуляли в парке, хорошая погода ☀️".into(),
                has_url: false,
                date: 0,
                preview_path: None,
            },
            StoryInfo {
                peer_id: 3003,
                peer_title: "Crypto Signals".into(),
                story_id: 3,
                caption: "Заработок от 500$ в день! Трейдинг, успей подписаться, свайп вверх 🚀💰".into(),
                has_url: true,
                date: 0,
                preview_path: None,
            },
            StoryInfo {
                peer_id: 3004,
                peer_title: "Доставка еды «Вкусно»".into(),
                story_id: 4,
                caption: "Промокод EDA20 -20% на первый заказ! Жми https://eda.example 🍔🎁".into(),
                has_url: true,
                date: 0,
                preview_path: None,
            },
            StoryInfo {
                peer_id: 3005,
                peer_title: "Иван".into(),
                story_id: 5,
                caption: "Новый велик, завтра первый выезд 🚲".into(),
                has_url: false,
                date: 0,
                preview_path: None,
            },
            StoryInfo {
                peer_id: 3006,
                peer_title: "Казино «Удача»".into(),
                story_id: 6,
                caption: "🎰 БОНУС 100% + 50 FS! Регистрация за 1 минуту t.me/luckycasino Успей забрать!".into(),
                has_url: true,
                date: 0,
                preview_path: None,
            },
        ])
    }

    async fn download_message_media(
        &self,
        _chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        tokio::fs::create_dir_all(dest_dir).await?;
        // Реалистичные имена, чтобы демо показывало всю цепочку:
        // mp4 → видео, epub → книга (Calibre), ogg → аудио.
        let fname: String = match message_id {
            101 => "lecture12.mp4".to_string(),
            102 => "phys_levels.epub".to_string(),
            103 => "hw_voice.ogg".to_string(),
            id => format!("mock_file_{id}.bin"),
        };
        let path = dest_dir.join(&fname);
        // Пишем детерминированный контент, чтобы sha256-дедуп был воспроизводим.
        let payload = format!("telecomfort mock payload :: {fname} :: {message_id}\n");
        tokio::fs::write(&path, payload.as_bytes()).await?;
        Ok(path)
    }

    async fn download_message_media_with_progress(
        &self,
        chat_id: i64,
        message_id: i32,
        dest_dir: &std::path::Path,
        workers: usize,
        on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> anyhow::Result<std::path::PathBuf> {
        let _ = workers;
        // Имитируем чанки, чтобы прогресс-бар было видно и в демо.
        on_progress(0, 3);
        let path = self.download_message_media(chat_id, message_id, dest_dir).await?;
        on_progress(2, 3);
        on_progress(3, 3);
        Ok(path)
    }

    async fn download_story_media(
        &self,
        peer_id: i64,
        story_id: i32,
        dest_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        // Генерируем детерминированный PNG-кадр 270x480, чтобы визуальный
        // скоринг и блюр работали и в офлайне. Рекламные сторис — тёплая
        // палитра + белые плашки (ценник), личные — спокойная.
        tokio::fs::create_dir_all(dest_dir).await?;
        let path = dest_dir.join(format!("story_{peer_id}_{story_id}.png"));
        if path.exists() {
            return Ok(path);
        }
        let is_ad = matches!(story_id, 1 | 3 | 4 | 6);
        let (w, h) = (270u32, 480u32);
        let mut img = image::RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let t = y as f32 / h as f32;
                let (r, g, b) = if is_ad {
                    // Тёплый promo-градиент
                    (
                        (215.0 + 40.0 * t) as u8,
                        (110.0 - 30.0 * t + 20.0 * (x as f32 / w as f32)) as u8,
                        (50.0 + 15.0 * t) as u8,
                    )
                } else {
                    (
                        (70.0 + 30.0 * t) as u8,
                        (140.0 + 25.0 * (x as f32 / w as f32)) as u8,
                        (175.0 - 25.0 * t) as u8,
                    )
                };
                img.put_pixel(x, y, image::Rgb([r, g, b]));
            }
        }
        if is_ad {
            // Белая плашка-ценник снизу + «текст»-полосы для edge-density.
            for y in (h - 150)..(h - 30) {
                for x in 15..(w - 15) {
                    img.put_pixel(x, y, image::Rgb([245, 245, 245]));
                }
            }
            for row in [h - 130, h - 110, h - 90] {
                for y in row..(row + 8) {
                    for x in 30..(w - 30) {
                        if y < h {
                            img.put_pixel(x, y, image::Rgb([20, 20, 20]));
                        }
                    }
                }
            }
            // Верхняя promo-полоса
            for y in 20..60 {
                for x in 15..(w - 15) {
                    img.put_pixel(x, y, image::Rgb([255, 220, 40]));
                }
            }
        } else {
            // Лёгкая виньетка для «живого» фото
            for y in 0..h {
                for x in 0..w {
                    if (x as i32 - w as i32 / 2).abs() + (y as i32 - h as i32 / 2).abs() > 300 {
                        let p = img.get_pixel(x, y);
                        img.put_pixel(
                            x,
                            y,
                            image::Rgb([
                                p[0].saturating_sub(15),
                                p[1].saturating_sub(15),
                                p[2].saturating_sub(10),
                            ]),
                        );
                    }
                }
            }
        }
        img.save(&path)?;
        Ok(path)
    }

    async fn me(&self) -> anyhow::Result<AccountInfo> {
        Ok(AccountInfo {
            id: 0,
            name: "Демо (mock)".into(),
            phone: None,
            username: None,
        })
    }

    /// Демо-отправка: синтетический id, приложение само добавит сообщение в историю.
    async fn send_text(
        &self,
        _chat_id: i64,
        text: &str,
        _reply_to: Option<i32>,
    ) -> anyhow::Result<i32> {
        // Детерминированный id из хэша текста, чтобы демо было стабильным.
        let mut h: i32 = 9000;
        for b in text.bytes() {
            h = h.wrapping_add(b as i32);
        }
        Ok(h)
    }

    async fn edit_text(&self, _chat_id: i64, _message_id: i32, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn delete_messages(&self, _chat_id: i64, _message_ids: &[i32]) -> anyhow::Result<()> {
        Ok(())
    }

    async fn react(&self, _chat_id: i64, _message_id: i32, _emoji: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_file(
        &self,
        _chat_id: i64,
        path: &std::path::Path,
        _caption: &str,
    ) -> anyhow::Result<i32> {
        let mut h: i32 = 9500;
        for b in path.as_os_str().as_encoded_bytes() {
            h = h.wrapping_add(*b as i32);
        }
        Ok(h)
    }

    fn is_connected(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::TelegramService;

    /// Демо-контракт композера: отправка/правка/удаление/реакции/файлы — Ok.
    #[tokio::test]
    async fn mock_messaging_contract() {
        let m = MockTelegram;
        let id = m.send_text(-1001, "привет", None).await.expect("send");
        assert!(id >= 9000);
        let id2 = m.send_text(-1001, "ответ", Some(id)).await.expect("reply");
        assert_ne!(id, id2);
        m.edit_text(-1001, id, "привет!").await.expect("edit");
        m.delete_messages(-1001, &[id]).await.expect("delete");
        m.react(-1001, id, "👍").await.expect("react");
        let fid = m.send_file(-1001, std::path::Path::new("doc.pdf"), "caption").await.expect("file");
        assert!(fid >= 9500);
        m.mark_read(-1001).await.expect("mark_read");
        assert!(m.avatar_path(-1001).await.expect("avatar").is_none());
    }

    /// Мок-история несёт новые поля (исходящие/отправитель/реакции).
    #[tokio::test]
    async fn mock_history_has_chat_fields() {
        let m = MockTelegram;
        let items = m.history(-1001, 10).await.expect("history");
        assert_eq!(items.len(), 3);
        assert!(items.iter().any(|x| x.outgoing));
        assert!(items.iter().any(|x| x.sender.is_some()));
        assert!(items.iter().any(|x| x.reactions == Some(5)));
        assert!(items.iter().any(|x| x.edited));
    }
}
