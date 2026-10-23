//! BK-дерево для pHash-поиска визуальных дублей.
//!
//! Линейный перебор (`downloads` × Хэмминг) на десятках тысяч загрузок
//! становится узким местом: каждый новый файл — полный скан таблицы,
//! а кластеризация «всех ко всем» — вообще O(n²).
//! BK-дерево (Burkhard–Keller) использует неравенство треугольника для
//! метрики Хэмминга: из узла с расстоянием `d` до запроса при радиусе `r`
//! нужно спускаться только в детей с ребром `|d − r| ..= d + r`.
//! На практике это отсекает большую часть дерева: поиск — суб-линейный,
//! построение — O(n log n) в среднем.
//!
//! Дерево — чисто in-memory индекс поверх SQLite (`Db::bk_index` собирает
//! его из `downloads.phash`, `Db::find_near_phash` ищет через него).
//! Персистентность не нужна: pHash'и — в БД, дерево перестраивается за
//! миллисекунды даже на десятках тысяч строк.

use crate::ai::phash::hamming;

/// BK-дерево над 64-битными pHash с произвольным payload (обычно путь файла).
#[derive(Debug, Default)]
pub struct BkTree<P> {
    root: Option<Node<P>>,
    len: usize,
}

#[derive(Debug)]
struct Node<P> {
    hash: u64,
    payloads: Vec<P>,
    /// Дети: (расстояние от `hash` до хэша ребёнка, поддерево).
    /// Расстояния в одном узле уникальны — инвариант вставки.
    children: Vec<(u32, Box<Node<P>>)>,
}

impl<P> BkTree<P> {
    /// Пустое дерево.
    pub fn new() -> Self {
        Self { root: None, len: 0 }
    }

    /// Число записей (payload'ов, не уникальных хэшей).
    pub fn len(&self) -> usize {
        self.len
    }

    /// Пустое ли дерево.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Вставить (хэш → payload). Одинаковые хэши копятся в одном узле.
    pub fn insert(&mut self, hash: u64, payload: P) {
        self.len += 1;
        let mut node = match self.root.as_mut() {
            Some(n) => n,
            None => {
                self.root = Some(Node { hash, payloads: vec![payload], children: vec![] });
                return;
            }
        };
        loop {
            let d = hamming(hash, node.hash);
            if d == 0 {
                node.payloads.push(payload);
                return;
            }
            // Позицию ищем без удержания заимствования — иначе push
            // в тот же Vec конфликтовал бы с итератором (E0499).
            let pos = node.children.iter().position(|(edge, _)| *edge == d);
            match pos {
                Some(i) => node = &mut node.children[i].1,
                None => {
                    node.children.push((
                        d,
                        Box::new(Node { hash, payloads: vec![payload], children: vec![] }),
                    ));
                    return;
                }
            }
        }
    }

    /// Все payload'ы в радиусе `max_dist` от `hash` (включая точные совпадения).
    /// Возвращает клоны payload'ов с расстояниями, без гарантии порядка
    /// (вызывающий сортирует при нужде — см. `query_sorted`).
    pub fn query(&self, hash: u64, max_dist: u32) -> Vec<(&P, u32)> {
        let mut out = Vec::new();
        let Some(root) = self.root.as_ref() else {
            return out;
        };
        // Явный стек вместо рекурсии: глубина ограничена, но запросы идут
        // из UI/фона — стек потока eframe лучше не тратить.
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let d = hamming(hash, node.hash);
            if d <= max_dist {
                out.extend(node.payloads.iter().map(|p| (p, d)));
            }
            // Неравенство треугольника: ребёнок с ребром `e` может содержать
            // хиты только если |d − e| <= max_dist, т.е. e в [d−r, d+r].
            let lo = d.saturating_sub(max_dist);
            let hi = d.saturating_add(max_dist);
            for (edge, child) in node.children.iter() {
                if *edge >= lo && *edge <= hi {
                    stack.push(child);
                }
            }
        }
        out
    }

    /// То же, но отсортировано по возрастанию расстояния.
    pub fn query_sorted(&self, hash: u64, max_dist: u32) -> Vec<(&P, u32)>
    where
        P: Clone,
    {
        let mut out = self.query(hash, max_dist);
        out.sort_by_key(|(_, d)| *d);
        out
    }
}

impl<P: Clone> BkTree<P> {
    /// Клонирующий поиск (удобно, когда дерево временное).
    pub fn query_cloned(&self, hash: u64, max_dist: u32) -> Vec<(P, u32)> {
        let mut out: Vec<(P, u32)> =
            self.query(hash, max_dist).into_iter().map(|(p, d)| (p.clone(), d)).collect();
        out.sort_by_key(|(_, d)| *d);
        out
    }

    /// Построить дерево из итератора пар (хэш, payload).
    pub fn from_iter<I: IntoIterator<Item = (u64, P)>>(iter: I) -> Self {
        let mut tree = Self::new();
        for (h, p) in iter {
            tree.insert(h, p);
        }
        tree
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree_queries_empty() {
        let t: BkTree<String> = BkTree::new();
        assert!(t.is_empty() && t.len() == 0);
        assert!(t.query(0x1234, 6).is_empty());
    }

    #[test]
    fn exact_and_near_hits() {
        let mut t = BkTree::new();
        t.insert(0x1234, "a");
        t.insert(0x1235, "b"); // 1 бит разницы
        t.insert(0xFFFF_FFFF_FFFF_FFFF, "far");
        assert_eq!(t.len(), 3);
        let hits = t.query_cloned(0x1234, 6);
        assert_eq!(hits, vec![("a", 0), ("b", 1)]);
        // Нулевой радиус — только точное совпадение.
        assert_eq!(t.query_cloned(0x1234, 0), vec![("a", 0)]);
        assert!(t.query_cloned(0x7777, 0).is_empty());
    }

    #[test]
    fn duplicate_hashes_accumulate() {
        let mut t = BkTree::new();
        t.insert(42, "first");
        t.insert(42, "second");
        assert_eq!(t.len(), 2);
        let hits = t.query_cloned(42, 0);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|(_, d)| *d == 0));
    }

    /// Детерминированный ГПСЧ (xorshift64*), чтобы тест не тянул `rand`.
    fn rng_next(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Совпадение с полным перебором на случайных данных при разных радиусах.
    #[test]
    fn matches_brute_force() {
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        let hashes: Vec<u64> = (0..2000).map(|_| rng_next(&mut state)).collect();
        let tree: BkTree<usize> =
            BkTree::from_iter(hashes.iter().copied().enumerate().map(|(i, h)| (h, i)));
        assert_eq!(tree.len(), 2000);
        let mut probe_state = 0xDEAD_BEEF_CAFE_1234u64;
        for radius in [0u32, 2, 6, 10] {
            for _ in 0..25 {
                let q = rng_next(&mut probe_state);
                let mut expected: Vec<(usize, u32)> = hashes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, h)| {
                        let d = hamming(q, *h);
                        (d <= radius).then_some((i, d))
                    })
                    .collect();
                expected.sort_by_key(|(_, d)| *d);
                let mut got = tree.query_cloned(q, radius);
                got.sort_by_key(|(_, d)| *d);
                // query_cloned уже сортирует, но для мульти-хитов с равным
                // расстоянием порядок не гарантирован — сравниваем как множества.
                let mut e_sorted = expected.clone();
                let mut g_sorted = got.clone();
                e_sorted.sort();
                g_sorted.sort();
                assert_eq!(g_sorted, e_sorted, "radius={radius} q={q:#x}");
            }
        }
    }

    #[test]
    fn clustered_hashes_all_found() {
        // Плотный кластер вокруг базы: все в радиусе 6.
        let mut tree = BkTree::new();
        let base = 0x0F0F_0F0F_0F0F_0F0Fu64;
        for i in 0..64u64 {
            // Меняем младшие биты — расстояние ≤ 6.
            tree.insert(base ^ ((1u64 << (i % 6)) - 1).wrapping_mul(0x0101), i as usize);
        }
        let hits = tree.query(base, 6);
        assert!(!hits.is_empty());
        // Все найденные — действительно в радиусе (без ложных от прунинга).
        for (_, d) in &hits {
            assert!(*d <= 6);
        }
        // А полный перебор даёт столько же (прунинг ничего не потерял).
        let mut brute = 0;
        let mut stack = vec![tree.root.as_ref().unwrap()];
        while let Some(n) = stack.pop() {
            for _ in &n.payloads {
                if hamming(base, n.hash) <= 6 {
                    brute += 1;
                }
            }
            for (_, c) in &n.children {
                stack.push(c);
            }
        }
        assert_eq!(hits.len(), brute);
    }
}
