//! Кластеризация визуальных дублей для UI Загрузок.
//!
//! Одиночные пометки «≈ похож на …» (см. `Db::find_near_phash`) не дают
//! картины: один мем, пересланный пять раз с разным ресайзом, — это одна
//! группа, а не пять попарных стрелок. Здесь — связные компоненты графа
//! «похожести» (ребро = Хэмминг ≤ `max_dist`, обычно `MAX_HAMMING`):
//! транзитивное замыкание, т.е. A≈B и B≈C — одна группа, даже если A и C
//! напрямую дальше порога (цепочка рекомпрессий).
//!
//! Наивный O(n²) перебор на десятках тысяч загрузок неприемлем, поэтому
//! соседи ищутся через BK-дерево (`ai::bktree`) инкрементально: для каждого
//! хэша запрашиваем уже вставленных соседей и объединяем через DSU.
//! Итог — в среднем O(n log n). Синглтоны (уникальные файлы) отбрасываются —
//! UI показывает только группы из ≥2 файлов, крупнее — первыми.

use crate::ai::bktree::BkTree;
use std::collections::HashMap;

/// Одна группа: индексы во входном слайсе хэшей (по возрастанию —
/// хронология добавления), группы — по убыванию размера.
pub type DupGroup = Vec<usize>;

/// Кластеризовать хэши: связные компоненты при пороге `max_dist`.
/// Пустой вход / все синглтоны → пустой вектор.
pub fn cluster_indices(hashes: &[u64], max_dist: u32) -> Vec<DupGroup> {
    let n = hashes.len();
    if n < 2 {
        return vec![];
    }
    let mut dsu = Dsu::new(n);
    let mut tree: BkTree<usize> = BkTree::new();
    for (i, &h) in hashes.iter().enumerate() {
        // Все ранее вставленные соседи — в ту же компоненту.
        for (j, _) in tree.query(h, max_dist) {
            dsu.union(i, *j);
        }
        tree.insert(h, i);
    }
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        by_root.entry(dsu.find(i)).or_default().push(i);
    }
    let mut groups: Vec<DupGroup> =
        by_root.into_values().filter(|g| g.len() >= 2).collect();
    for g in groups.iter_mut() {
        g.sort_unstable();
    }
    // Крупные группы первыми; при равном размере — раньше созданные.
    groups.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    groups
}

/// Disjoint Set Union (путь + ранг).
struct Dsu {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl Dsu {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect(), rank: vec![0; n] }
    }

    fn find(&mut self, mut x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // Сжатие пути.
        while self.parent[x] != x {
            let p = self.parent[x];
            self.parent[x] = root;
            x = p;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (mut ra, mut rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        if self.rank[ra] < self.rank[rb] {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        if self.rank[ra] == self.rank[rb] {
            self.rank[ra] = self.rank[ra].saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::phash::{hamming, MAX_HAMMING};

    #[test]
    fn empty_and_single_have_no_groups() {
        assert!(cluster_indices(&[], MAX_HAMMING).is_empty());
        assert!(cluster_indices(&[0x1234], MAX_HAMMING).is_empty());
    }

    #[test]
    fn identical_pair_is_one_group() {
        let groups = cluster_indices(&[0xABCD, 0x1234, 0xABCD], MAX_HAMMING);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], vec![0, 2]);
    }

    #[test]
    fn singletons_excluded() {
        let groups = cluster_indices(
            &[0x1111, 0x2222_2222_2222_2222, 0x1111, 0x3333_3333_3333_3333],
            MAX_HAMMING,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], vec![0, 2]);
    }

    #[test]
    fn transitive_chain_is_one_group() {
        // A~B (d=2), B~C (d=2), A!~C (d=4) при пороге 2 — всё равно одна группа.
        let (a, b, c) = (0u64, 0b11u64, 0b1111u64);
        assert_eq!(hamming(a, b), 2);
        assert_eq!(hamming(b, c), 2);
        assert_eq!(hamming(a, c), 4);
        let groups = cluster_indices(&[a, b, c, 0xFFFF_FFFF_FFFF_FFFF], 2);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], vec![0, 1, 2]);
    }

    #[test]
    fn separate_clusters_sorted_by_size() {
        // Кластер 3 вокруг 0x000… + кластер 2 вокруг 0xFFF… + синглтон.
        // Базы на расстоянии 64 — кластеры заведомо не склеиваются.
        let (a0, b0) = (0x0000_0000_0000_0000u64, 0xFFFF_FFFF_FFFF_FFFFu64);
        let hashes = [
            a0,
            b0,
            a0 ^ 0b1,   // ~a0 (d=1)
            b0 ^ 0b1,   // ~b0 (d=1)
            a0 ^ 0b11,  // ~a0 (d=2)
            0x1234_5678_9ABC_DEF0, // синглтон (далеко от всех)
        ];
        assert!(hamming(a0, b0) > 2);
        let groups = cluster_indices(&hashes, 2);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], vec![0, 2, 4]); // тройка первая
        assert_eq!(groups[1], vec![1, 3]);
    }

    fn rng_next(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Эталон: связные компоненты brute-force графа (BFS по всем парам).
    fn brute_groups(hashes: &[u64], max_dist: u32) -> Vec<Vec<usize>> {
        let n = hashes.len();
        let mut seen = vec![false; n];
        let mut groups = vec![];
        for s in 0..n {
            if seen[s] {
                continue;
            }
            let mut comp = vec![];
            let mut stack = vec![s];
            seen[s] = true;
            while let Some(i) = stack.pop() {
                comp.push(i);
                for j in 0..n {
                    if !seen[j] && hamming(hashes[i], hashes[j]) <= max_dist {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            if comp.len() >= 2 {
                comp.sort_unstable();
                groups.push(comp);
            }
        }
        groups.sort_by(|a: &Vec<usize>, b: &Vec<usize>| {
            b.len().cmp(&a.len()).then(a[0].cmp(&b[0]))
        });
        groups
    }

    #[test]
    fn matches_brute_force_on_random() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        // Плотные кластеры + шум: берём базы и мутируем младшие биты.
        let mut hashes = vec![];
        for _ in 0..12 {
            let base = rng_next(&mut state);
            hashes.push(base);
            for _ in 0..(rng_next(&mut state) % 5) {
                hashes.push(base ^ (rng_next(&mut state) & 0x3F));
            }
        }
        for _ in 0..40 {
            hashes.push(rng_next(&mut state));
        }
        for radius in [0u32, 2, 6] {
            assert_eq!(
                cluster_indices(&hashes, radius),
                brute_groups(&hashes, radius),
                "radius={radius}"
            );
        }
    }

    #[test]
    fn large_smoke_and_spot_check() {
        // 20k хэшей: должно отрабатывать быстро и без паники (smoke).
        // Кластеры строим явно: 200 баз × ~100 мутантов в радиусе 6.
        let mut state = 0x1234_ABCD_5678_EF01u64;
        let mut hashes = Vec::with_capacity(20_000);
        let mut bases = vec![];
        for _ in 0..200 {
            let base = rng_next(&mut state);
            bases.push(base);
            hashes.push(base);
            for _ in 0..99 {
                // Мутация ≤6 бит младшего байта — гарантированно в радиусе… нет:
                // xor со случайной маской до 0xFF может дать до 8 бит.
                // Маскируем до 6 бит: берём 6 случайных позиций.
                let mut m = 0u64;
                for _ in 0..(rng_next(&mut state) % 7) {
                    m |= 1u64 << (rng_next(&mut state) % 64);
                }
                // Нормируем: оставляем не более 6 бит.
                while m.count_ones() > 6 {
                    m &= m - 1;
                }
                hashes.push(base ^ m);
            }
        }
        let t0 = std::time::Instant::now();
        let groups = cluster_indices(&hashes, MAX_HAMMING);
        let dt = t0.elapsed();
        eprintln!("cluster 20k: {} groups in {dt:?}", groups.len());
        // Каждый мутант обязан оказаться в группе со своей базой:
        // проверяем выборочно первые 20 баз.
        for (bi, base) in bases.iter().enumerate().take(20) {
            let base_idx = bi * 100;
            let g = groups
                .iter()
                .find(|g| g.contains(&base_idx))
                .unwrap_or_else(|| panic!("база {bi} (хэш {base:#x}) без группы"));
            assert!(g.len() >= 50, "группа базы {bi} слишком мала: {}", g.len());
        }
        // Smoke по времени: даже в debug должно уложиться в ~30с.
        assert!(dt.as_secs() < 60, "слишком медленно: {dt:?}");
    }
}
