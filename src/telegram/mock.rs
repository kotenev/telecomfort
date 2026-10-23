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
                muted: false,
                archived: false,
            },
            ChatInfo {
                id: -1002,
                title: "Учёба / Лекции".into(),
                username: None,
                protected: true,
                unread: 3,
                muted: true,
                archived: false,
            },
            ChatInfo {
                id: 2001,
                title: "Друзья".into(),
                username: Some("friends".into()),
                protected: false,
                unread: 0,
                muted: false,
                archived: true,
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
                views: Some(1204),
                forwards: Some(37),
                poll: None,
                geo: None,
                contact: None,
                web_preview: None,
                topic_id: None,
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
                views: None,
                forwards: None,
                poll: Some(crate::telegram::PollInfo {
                    question: "Когда пара?".into(),
                    closed: false,
                    quiz: false,
                    total_voters: Some(7),
                    answers: vec![
                        crate::telegram::PollAnswerInfo {
                            text: "В 10:00".into(),
                            option: vec![0],
                            votes: Some(5),
                            chosen: true,
                        },
                        crate::telegram::PollAnswerInfo {
                            text: "В 12:00".into(),
                            option: vec![1],
                            votes: Some(2),
                            chosen: false,
                        },
                    ],
                }),
                geo: None,
                contact: None,
                web_preview: None,
                topic_id: None,
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
                views: None,
                forwards: None,
                poll: None,
                geo: Some((55.7558, 37.6173)),
                contact: Some(("Анна".into(), "+7 900 000-00-00".into())),
                web_preview: Some(crate::telegram::WebPreview {
                    url: "https://example.com".into(),
                    title: "Пример".into(),
                    description: "Описание страницы".into(),
                    photo: None,
                }),
                topic_id: None,
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

    async fn send_scheduled(
        &self,
        _chat_id: i64,
        text: &str,
        _reply_to: Option<i32>,
        _at: std::time::SystemTime,
    ) -> anyhow::Result<i32> {
        let mut h: i32 = 9800;
        for b in text.bytes() {
            h = h.wrapping_add(b as i32);
        }
        Ok(h)
    }

    async fn sessions(&self) -> anyhow::Result<Vec<crate::telegram::SessionInfo>> {
        Ok(vec![
            crate::telegram::SessionInfo {
                current: true,
                app_name: "Telecomfort".into(),
                device: "MacBook".into(),
                platform: "macOS".into(),
                country: "Россия".into(),
                ip: "127.0.0.1".into(),
                last_active: 1_700_000_000,
                hash: 1,
            },
            crate::telegram::SessionInfo {
                current: false,
                app_name: "Telegram Desktop".into(),
                device: "PC".into(),
                platform: "Windows".into(),
                country: "Россия".into(),
                ip: "10.0.0.2".into(),
                last_active: 1_699_000_000,
                hash: 2,
            },
        ])
    }

    async fn terminate_others(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_profile(&self) -> anyhow::Result<crate::telegram::ProfileInfo> {
        Ok(crate::telegram::ProfileInfo {
            first_name: "Демо".into(),
            last_name: "Юзер".into(),
            about: "офлайн-демо".into(),
        })
    }

    async fn update_profile(
        &self,
        _first_name: &str,
        _last_name: &str,
        _about: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn publish_story(
        &self,
        path: &std::path::Path,
        _caption: &str,
    ) -> anyhow::Result<i32> {
        let mut h: i32 = 700;
        for b in path.as_os_str().as_encoded_bytes() {
            h = h.wrapping_add(*b as i32);
        }
        Ok(h)
    }

    async fn delete_stories(&self, _story_ids: &[i32]) -> anyhow::Result<()> {
        Ok(())
    }

    async fn forum_topics(&self, _chat_id: i64) -> anyhow::Result<Vec<crate::telegram::TopicInfo>> {
        Ok(vec![
            crate::telegram::TopicInfo { id: 1, title: "Общее".into() },
            crate::telegram::TopicInfo { id: 2, title: "Оффтопик".into() },
        ])
    }

    async fn create_topic(&self, _chat_id: i64, _title: &str) -> anyhow::Result<i32> {
        Ok(3)
    }

    async fn topic_history(
        &self,
        chat_id: i64,
        topic_id: i32,
        _limit: usize,
    ) -> anyhow::Result<Vec<crate::telegram::MessageInfo>> {
        Ok(vec![crate::telegram::MessageInfo {
            id: 201,
            chat_id,
            text: format!("сообщение топика {topic_id}"),
            date: 1_700_000_000,
            has_media: false,
            media_kind: None,
            file_name: None,
            file_size: None,
            outgoing: false,
            sender: Some("Анна".into()),
            edited: false,
            reactions: None,
            views: None,
            forwards: None,
            poll: None,
            geo: None,
            contact: None,
            web_preview: None,
            topic_id: Some(topic_id),
        }])
    }

    async fn send_to_topic(&self, _chat_id: i64, _topic_id: i32, text: &str) -> anyhow::Result<i32> {
        let mut h: i32 = 9600;
        for b in text.bytes() {
            h = h.wrapping_add(b as i32);
        }
        Ok(h)
    }

    async fn global_search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<(i64, crate::telegram::MessageInfo)>> {
        let items = self.history(-1001, 10).await?;
        Ok(items.into_iter().map(|m| (-1001, m)).collect())
    }

    async fn scheduled_history(
        &self,
        chat_id: i64,
    ) -> anyhow::Result<Vec<crate::telegram::MessageInfo>> {
        Ok(vec![crate::telegram::MessageInfo {
            id: 301,
            chat_id,
            text: "отложенное напоминание".into(),
            date: 1_800_000_000,
            has_media: false,
            media_kind: None,
            file_name: None,
            file_size: None,
            outgoing: true,
            sender: None,
            edited: false,
            reactions: None,
            views: None,
            forwards: None,
            poll: None,
            geo: None,
            contact: None,
            web_preview: None,
            topic_id: None,
        }])
    }

    async fn cancel_scheduled(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        Ok(())
    }

    async fn create_channel(
        &self,
        title: &str,
        _about: &str,
        _broadcast: bool,
    ) -> anyhow::Result<i64> {
        Ok(-(1_000_000_000_000 + title.len() as i64))
    }

    async fn invite_link(&self, chat_id: i64) -> anyhow::Result<String> {
        Ok(format!("https://t.me/c/{}/1", chat_id.abs()))
    }

    async fn block_user(&self, _chat_id: i64, _blocked: bool) -> anyhow::Result<()> {
        Ok(())
    }

    async fn clear_history(&self, _chat_id: i64, _revoke: bool) -> anyhow::Result<()> {
        Ok(())
    }

    async fn leave_channel(&self, _chat_id: i64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn add_contact(
        &self,
        _phone: &str,
        _first_name: &str,
        _last_name: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn download_web_photo(
        &self,
        _photo: &crate::telegram::WebPreviewPhoto,
        dest: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        Ok(dest.to_path_buf())
    }

    async fn pin_message(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        Ok(())
    }

    async fn unpin_message(&self, _chat_id: i64, _message_id: i32) -> anyhow::Result<()> {
        Ok(())
    }

    async fn pinned_message(
        &self,
        chat_id: i64,
    ) -> anyhow::Result<Option<crate::telegram::MessageInfo>> {
        Ok(Some(crate::telegram::MessageInfo {
            id: 1,
            chat_id,
            text: "📌 Закреп: читаем правила чата".into(),
            date: 0,
            has_media: false,
            media_kind: None,
            file_name: None,
            file_size: None,
            outgoing: false,
            sender: Some("Админ".into()),
            edited: false,
            reactions: None,
            views: Some(999),
            forwards: None,
            poll: None,
            geo: None,
            contact: None,
            web_preview: None,
            topic_id: None,
        }))
    }

    async fn vote(
        &self,
        _chat_id: i64,
        _message_id: i32,
        _option: Vec<u8>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn forward(
        &self,
        _from_chat: i64,
        message_ids: &[i32],
        _to_chat: i64,
    ) -> anyhow::Result<usize> {
        Ok(message_ids.len())
    }

    async fn mute(&self, _chat_id: i64, _muted: bool) -> anyhow::Result<()> {
        Ok(())
    }

    async fn archive(&self, _chat_id: i64, _archived: bool) -> anyhow::Result<()> {
        Ok(())
    }

    async fn dialog_filters(&self) -> anyhow::Result<Vec<crate::telegram::DialogFilterInfo>> {
        Ok(vec![
            crate::telegram::DialogFilterInfo {
                id: 1,
                title: "Учёба".into(),
                peer_ids: vec![-1002],
            },
            crate::telegram::DialogFilterInfo {
                id: 2,
                title: "Личное".into(),
                peer_ids: vec![2001],
            },
        ])
    }

    async fn contacts(&self) -> anyhow::Result<Vec<crate::telegram::ContactInfo>> {
        Ok(vec![
            crate::telegram::ContactInfo {
                name: "Анна".into(),
                phone: "+7 900 000-00-00".into(),
                username: Some("anna".into()),
                user_id: 2001,
            },
            crate::telegram::ContactInfo {
                name: "Лектор".into(),
                phone: "+7 900 000-00-01".into(),
                username: None,
                user_id: 2002,
            },
        ])
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
        assert!(items.iter().any(|x| x.views == Some(1204)));
        assert!(items.iter().any(|x| x.forwards == Some(37)));
        assert!(items.iter().any(|x| x.poll.as_ref().map(|p| p.answers.len()) == Some(2)));
        assert!(items.iter().any(|x| x.geo.is_some()));
        assert!(items.iter().any(|x| x.contact.is_some()));
    }

    /// Контракт второго parity-пакета: пины/опросы/пересылка/мьют/архив/папки/контакты.
    #[tokio::test]
    async fn mock_management_contract() {
        let m = MockTelegram;
        m.pin_message(-1001, 5).await.expect("pin");
        m.unpin_message(-1001, 5).await.expect("unpin");
        let pinned = m.pinned_message(-1001).await.expect("pinned").expect("some");
        assert_eq!(pinned.id, 1);
        m.vote(-1001, 102, vec![0]).await.expect("vote");
        assert_eq!(m.forward(-1001, &[101, 102], 2001).await.expect("forward"), 2);
        m.mute(-1001, true).await.expect("mute");
        m.archive(-1001, true).await.expect("archive");
        let filters = m.dialog_filters().await.expect("filters");
        assert_eq!(filters.len(), 2);
        assert!(filters.iter().any(|f| f.title == "Учёба"));
        let contacts = m.contacts().await.expect("contacts");
        assert_eq!(contacts.len(), 2);
        assert!(contacts.iter().any(|c| c.username.as_deref() == Some("anna")));
        let dialogs = m.dialogs().await.expect("dialogs");
        assert!(dialogs.iter().any(|d| d.muted));
        assert!(dialogs.iter().any(|d| d.archived));
    }

    /// Контракт третьего parity-пакета: отложка/черновики/сессии/профиль/сторис.
    #[tokio::test]
    async fn mock_management_contract_2() {
        let m = MockTelegram;
        let id = m
            .send_scheduled(-1001, "потом", None, std::time::SystemTime::now())
            .await
            .expect("scheduled");
        assert!(id >= 9800);
        m.save_draft(-1001, "черновик").await.expect("draft");
        let sessions = m.sessions().await.expect("sessions");
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|s| s.current));
        m.terminate_others().await.expect("terminate");
        let p = m.get_profile().await.expect("profile");
        assert_eq!(p.first_name, "Демо");
        m.update_profile("A", "B", "bio").await.expect("update profile");
        let sid =
            m.publish_story(std::path::Path::new("photo.jpg"), "привет").await.expect("story");
        assert!(sid >= 700);
    }

    /// Контракт четвёртого parity-пакета: сторис-менеджмент, топики, поиск,
    /// запланированные, создание, модерация, контакты.
    #[tokio::test]
    async fn mock_management_contract_3() {
        let m = MockTelegram;
        m.delete_stories(&[7]).await.expect("delete stories");
        let topics = m.forum_topics(-1002).await.expect("topics");
        assert_eq!(topics.len(), 2);
        assert_eq!(m.create_topic(-1002, "Новинки").await.expect("create topic"), 3);
        let th = m.topic_history(-1002, 2, 10).await.expect("topic history");
        assert_eq!(th.len(), 1);
        assert_eq!(th[0].topic_id, Some(2));
        assert!(m.send_to_topic(-1002, 2, "в топик").await.expect("send topic") >= 9600);
        let gs = m.global_search("лекция", 10).await.expect("global search");
        assert!(!gs.is_empty());
        let sch = m.scheduled_history(-1001).await.expect("scheduled");
        assert_eq!(sch.len(), 1);
        m.cancel_scheduled(-1001, 301).await.expect("cancel");
        let cid = m.create_channel("Новости", "о нас", true).await.expect("create");
        assert!(cid < -1_000_000_000_000);
        assert!(m.invite_link(-1001).await.expect("invite").starts_with("https://t.me/"));
        m.block_user(2001, true).await.expect("block");
        m.clear_history(-1001, false).await.expect("clear");
        m.leave_channel(-1002).await.expect("leave");
        m.add_contact("+7 900 000-00-02", "Иван", "").await.expect("add contact");
    }
}
