//! タイトル / タグ / フォルダ名 / URL・パス / メモ を横断する検索（C# 版 SearchService と同じ採点）。
//! 複数ワードは AND 条件（すべての語がどこかに一致する必要がある）。

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::Utc;

use crate::model::Shortcut;
use crate::normalize::{normalize, split_terms};
use crate::target::try_get_host;
use crate::util::culture_cmp_ignore_case;

// スコアの基準値（相対的な順位付けのためのもの）
const SCORE_TITLE_EXACT: i32 = 1000;
const SCORE_TITLE_PREFIX: i32 = 700;
const SCORE_TITLE_CONTAINS: i32 = 500;
const SCORE_TAG_EXACT: i32 = 460;
const SCORE_TAG_PREFIX: i32 = 380;
const SCORE_TAG_CONTAINS: i32 = 320;
const SCORE_FOLDER_EXACT: i32 = 300;
const SCORE_FOLDER_CONTAINS: i32 = 240;
const SCORE_TARGET_HOST_PREFIX: i32 = 260;
const SCORE_TARGET_CONTAINS: i32 = 200;
const SCORE_NOTE_CONTAINS: i32 = 90;

/// 使用頻度・最近使ったかによる加点の上限（検索語の一致より優先されすぎないよう抑えめにする）。
const MAX_USAGE_BONUS: i32 = 150;

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub id: String,
    pub score: i32,
}

/// 検索用に正規化した文字列のキャッシュ。項目の内容が変わったものだけ作り直す。
#[derive(Default)]
pub struct SearchCache {
    entries: HashMap<String, IndexEntry>,
}

struct IndexEntry {
    store_revision: u64,
    item_revision: u64,
    title: String,
    target: String,
    host: String,
    folder_path: String,
    folder_segments: Vec<String>,
    note: String,
    tags: Vec<String>,
}

impl SearchCache {
    fn get(&mut self, item: &Shortcut, store_revision: u64) -> &IndexEntry {
        let stale = match self.entries.get(&item.id) {
            Some(e) => e.store_revision != store_revision || e.item_revision != item.revision,
            None => true,
        };
        if stale {
            let folder_path = normalize(&item.folder_path_text);
            let folder_segments = folder_path.split('>').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
            self.entries.insert(
                item.id.clone(),
                IndexEntry {
                    store_revision,
                    item_revision: item.revision,
                    title: normalize(item.display_title()),
                    target: normalize(&item.target),
                    host: normalize_host(&item.target),
                    folder_path,
                    folder_segments,
                    note: normalize(item.note.as_deref().unwrap_or("")),
                    tags: item.tags.iter().map(|t| normalize(t)).collect(),
                },
            );
        }
        &self.entries[&item.id]
    }
}

/// 検索してスコア順に並べた結果を返す。query が空なら None（呼び出し側で既定の並びを使う）。
pub fn search<'a>(
    source: impl IntoIterator<Item = &'a Shortcut>,
    query: &str,
    cache: &mut SearchCache,
    store_revision: u64,
    max_results: usize,
) -> Option<Vec<SearchHit>> {
    let terms = split_terms(query);
    if terms.is_empty() {
        return None;
    }

    let mut hits: Vec<(SearchHit, &Shortcut)> = Vec::new();
    for item in source {
        let index = cache.get(item, store_revision);
        let mut total = 0;
        let mut matched_all = true;
        for term in &terms {
            let best = score_term(index, term);
            if best <= 0 {
                matched_all = false;
                break;
            }
            total += best;
        }
        if !matched_all {
            continue;
        }
        hits.push((SearchHit { id: item.id.clone(), score: total + usage_bonus(item) }, item));
    }

    hits.sort_by(|(a, ai), (b, bi)| compare_hits(a, ai, b, bi));
    hits.truncate(max_results);
    Some(hits.into_iter().map(|(h, _)| h).collect())
}

fn compare_hits(a: &SearchHit, ai: &Shortcut, b: &SearchHit, bi: &Shortcut) -> Ordering {
    b.score
        .cmp(&a.score)
        // 同スコアなら短いタイトルを先に（より「ぴったり」に見えるため）
        .then_with(|| utf16_len(ai.display_title()).cmp(&utf16_len(bi.display_title())))
        .then_with(|| culture_cmp_ignore_case(ai.display_title(), bi.display_title()))
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn score_term(index: &IndexEntry, term: &str) -> i32 {
    let mut best = 0;

    if !index.title.is_empty() {
        if index.title == term {
            best = best.max(SCORE_TITLE_EXACT);
        } else if index.title.starts_with(term) {
            best = best.max(SCORE_TITLE_PREFIX);
        } else if index.title.contains(term) {
            best = best.max(SCORE_TITLE_CONTAINS);
        }
    }

    for tag in &index.tags {
        if tag == term {
            best = best.max(SCORE_TAG_EXACT);
            break;
        }
        if tag.starts_with(term) {
            best = best.max(SCORE_TAG_PREFIX);
        } else if tag.contains(term) {
            best = best.max(SCORE_TAG_CONTAINS);
        }
    }

    if !index.folder_path.is_empty() && index.folder_path.contains(term) {
        // 「開発」のようにフォルダ名そのものと一致する場合は少し高く
        let exact_segment = index.folder_segments.iter().any(|s| s == term);
        best = best.max(if exact_segment { SCORE_FOLDER_EXACT } else { SCORE_FOLDER_CONTAINS });
    }

    if !index.target.is_empty() && index.target.contains(term) {
        let host_prefix = !index.host.is_empty() && index.host.starts_with(term);
        best = best.max(if host_prefix { SCORE_TARGET_HOST_PREFIX } else { SCORE_TARGET_CONTAINS });
    }

    if !index.note.is_empty() && index.note.contains(term) {
        best = best.max(SCORE_NOTE_CONTAINS);
    }

    best
}

/// よく使う/最近使ったものを少しだけ上位に寄せる（§57）。
fn usage_bonus(item: &Shortcut) -> i32 {
    let mut bonus = (item.usage_count.min(30) * 3) as i32;
    if let Some(last) = item.last_used_at {
        let elapsed = Utc::now() - last;
        if elapsed < chrono::Duration::hours(1) {
            bonus += 60;
        } else if elapsed < chrono::Duration::days(1) {
            bonus += 40;
        } else if elapsed < chrono::Duration::days(7) {
            bonus += 20;
        }
    }
    bonus.min(MAX_USAGE_BONUS)
}

/// 最近使った順（ランチャーで検索語が空のとき用, §56）。
pub fn recent<'a>(source: impl IntoIterator<Item = &'a Shortcut>, max_results: usize) -> Vec<String> {
    let mut list: Vec<&Shortcut> = source.into_iter().filter(|s| s.last_used_at.is_some()).collect();
    list.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
    list.into_iter().take(max_results).map(|s| s.id.clone()).collect()
}

/// "www." を落としたホスト名（"github" で www.github.com を先頭一致させるため）。
fn normalize_host(target: &str) -> String {
    match try_get_host(target) {
        Some(host) if !host.is_empty() => {
            let host = if host.len() >= 4 && host[..4].eq_ignore_ascii_case("www.") { &host[4..] } else { &host };
            normalize(host)
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TargetType;

    fn item(title: &str, target: &str, tags: &[&str], folder: &str) -> Shortcut {
        Shortcut {
            title: title.into(),
            target: target.into(),
            target_type: TargetType::Web,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            folder_path_text: folder.into(),
            ..Shortcut::new()
        }
    }

    #[test]
    fn scores_title_tags_folders_and_hosts() {
        let items = vec![
            item("GitHub", "https://www.github.com/", &["開発"], "仕事 > 開発"),
            item("Google", "https://www.google.com/", &["検索"], "未分類"),
            item("社内ポータル", "https://portal.corp/", &["かいはつ"], "仕事"),
        ];
        let mut cache = SearchCache::default();

        let hits = search(&items, "github", &mut cache, 1, usize::MAX).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].score, SCORE_TITLE_EXACT);

        // ひらがなとカタカナ、全角と半角は同じとみなす
        let hits = search(&items, "カイハツ", &mut cache, 1, usize::MAX).unwrap();
        assert_eq!(hits.len(), 1);

        // AND 条件
        let hits = search(&items, "開発 git", &mut cache, 1, usize::MAX).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(search(&items, "開発 存在しない", &mut cache, 1, usize::MAX).unwrap().is_empty());

        // ホスト名の先頭一致
        let hits = search(&items, "goo", &mut cache, 1, usize::MAX).unwrap();
        assert_eq!(hits[0].score, SCORE_TITLE_PREFIX);

        assert!(search(&items, "  ", &mut cache, 1, usize::MAX).is_none());
    }

    #[test]
    fn usage_moves_items_up_a_little() {
        let mut a = item("Alpha", "https://a.example/", &[], "");
        let b = item("Alpine", "https://b.example/", &[], "");
        a.usage_count = 10;
        a.last_used_at = Some(Utc::now());
        let items = vec![b, a];
        let mut cache = SearchCache::default();
        let hits = search(&items, "alp", &mut cache, 1, usize::MAX).unwrap();
        assert_eq!(items.iter().find(|s| s.id == hits[0].id).unwrap().title, "Alpha");
    }

    #[test]
    fn ten_thousand_items_are_searched_quickly() {
        let items: Vec<Shortcut> =
            (0..10_000).map(|i| item(&format!("項目 {i}"), &format!("https://site{i}.example/"), &["タグ"], "フォルダ")).collect();
        let mut cache = SearchCache::default();
        let _ = search(&items, "項目", &mut cache, 1, usize::MAX);
        let start = std::time::Instant::now();
        let hits = search(&items, "項目 99", &mut cache, 1, usize::MAX).unwrap();
        assert!(!hits.is_empty());
        assert!(start.elapsed().as_millis() < 1000, "{:?}", start.elapsed());
    }
}
