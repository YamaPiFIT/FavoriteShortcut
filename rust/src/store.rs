//! SQLite への永続化と、メモリ上のキャッシュをまとめて扱うストア（C# 版 AppStore の移植）。
//!
//! 起動時に全件をメモリへ読み込み、検索・並べ替えはメモリ上で行う。
//! 書き込みは常に SQLite へ即時反映する（C# 版と同じ DB を同じ規則で読み書きする）。

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::{Database, DbResult, format_time, parse_time};
use crate::loc;
use crate::model::{Folder, Shortcut, TargetType, new_id};
use crate::util::{culture_cmp, eq_ignore_case, fold};

pub struct Store {
    db: Database,
    folders: HashMap<String, Folder>,
    root_folders: Vec<String>,
    shortcuts: Vec<Shortcut>,
    index: HashMap<String, usize>,
    /// 大文字にそろえたタグ名 -> (表示名, タグ ID)
    tags: HashMap<String, (String, String)>,

    /// 全件を読み直したときに増える番号（検索の索引はこれと照合して作り直す）
    pub revision: u64,
    /// 何か変わるたびに増える番号（画面はこれを見て一覧を作り直す）
    pub data_version: u64,

    batch_depth: u32,
    tree_dirty: bool,
    orphan_tags_dirty: bool,
}

impl Store {
    pub fn open(db: Database) -> DbResult<Self> {
        let mut store = Self {
            db,
            folders: HashMap::new(),
            root_folders: Vec::new(),
            shortcuts: Vec::new(),
            index: HashMap::new(),
            tags: HashMap::new(),
            revision: 0,
            data_version: 0,
            batch_depth: 0,
            tree_dirty: false,
            orphan_tags_dirty: false,
        };
        store.load()?;
        Ok(store)
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    fn conn(&self) -> &Connection {
        self.db.conn()
    }

    fn changed(&mut self) {
        self.data_version = self.data_version.wrapping_add(1);
    }

    // ------------------------------------------------------------- まとめて変更

    /// 複数件をまとめて変更するときに使う（取り込み・複数削除など）。
    /// 後片付け（未使用タグの削除・ツリーの組み直し）を最後に 1 回だけ行う。
    pub fn begin_batch(&mut self) {
        self.batch_depth += 1;
    }

    pub fn end_batch(&mut self) {
        if self.batch_depth == 0 {
            return;
        }
        self.batch_depth -= 1;
        if self.batch_depth > 0 {
            return;
        }
        if self.orphan_tags_dirty {
            self.clean_up_orphan_tags();
        }
        if self.tree_dirty {
            self.rebuild_folder_tree();
        }
        self.changed();
    }

    // ------------------------------------------------------------------ load

    fn load(&mut self) -> DbResult<()> {
        self.load_folders()?;
        self.load_tags()?;
        self.load_shortcuts()?;
        self.rebuild_folder_tree();
        self.revision = self.revision.wrapping_add(1);
        self.changed();
        Ok(())
    }

    /// インポート後などに、DB からすべて読み込み直す。
    pub fn reload(&mut self) -> DbResult<()> {
        self.load()
    }

    fn load_folders(&mut self) -> DbResult<()> {
        self.folders.clear();
        let mut stmt = self.conn().prepare("SELECT id, name, parent_id, sort_order, created_at, updated_at FROM folders")?;
        let rows = stmt.query_map([], |r| {
            Ok(Folder {
                id: r.get(0)?,
                name: r.get(1)?,
                parent_id: r.get(2)?,
                sort_order: r.get(3)?,
                created_at: parse_time(&r.get::<_, String>(4)?),
                updated_at: parse_time(&r.get::<_, String>(5)?),
                full_path: String::new(),
                children: Vec::new(),
            })
        })?;
        let mut folders = HashMap::new();
        for f in rows {
            let f = f?;
            folders.insert(f.id.clone(), f);
        }
        drop(stmt);
        self.folders = folders;
        Ok(())
    }

    fn load_tags(&mut self) -> DbResult<()> {
        let mut stmt = self.conn().prepare("SELECT id, name FROM tags")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut tags = HashMap::new();
        for row in rows {
            let (id, name) = row?;
            tags.insert(fold(&name), (name, id));
        }
        drop(stmt);
        self.tags = tags;
        Ok(())
    }

    fn load_shortcuts(&mut self) -> DbResult<()> {
        let mut list = Vec::new();
        {
            let mut stmt = self.conn().prepare(
                "SELECT s.id, s.folder_id, s.title, s.target, s.target_type, s.icon_path, s.note, \
                        s.sort_order, s.created_at, s.updated_at, u.usage_count, u.last_used_at \
                 FROM shortcuts s LEFT JOIN shortcut_usage u ON u.shortcut_id = s.id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(Shortcut {
                    id: r.get(0)?,
                    folder_id: r.get(1)?,
                    title: r.get(2)?,
                    target: r.get(3)?,
                    target_type: TargetType::from_i64(r.get(4)?),
                    icon_path: r.get(5)?,
                    note: r.get(6)?,
                    sort_order: r.get(7)?,
                    created_at: parse_time(&r.get::<_, String>(8)?),
                    updated_at: parse_time(&r.get::<_, String>(9)?),
                    usage_count: r.get::<_, Option<i64>>(10)?.unwrap_or(0),
                    last_used_at: r.get::<_, Option<String>>(11)?.map(|s| parse_time(&s)),
                    ..Default::default()
                })
            })?;
            for row in rows {
                list.push(row?);
            }
        }

        // タグはまとめて読み込む（N+1 クエリを避ける）
        let name_by_id: HashMap<&str, &str> = self.tags.values().map(|(name, id)| (id.as_str(), name.as_str())).collect();
        let mut per_shortcut: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = self.conn().prepare("SELECT shortcut_id, tag_id FROM shortcut_tags ORDER BY shortcut_id, sort_order")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (sid, tid) = row?;
                if let Some(name) = name_by_id.get(tid.as_str()) {
                    per_shortcut.entry(sid).or_default().push((*name).to_string());
                }
            }
        }

        for s in &mut list {
            if let Some(tags) = per_shortcut.remove(&s.id) {
                s.tags = tags;
            }
        }
        self.shortcuts = list;
        self.rebuild_index();
        Ok(())
    }

    fn rebuild_index(&mut self) {
        self.index = self.shortcuts.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
    }

    // ---------------------------------------------------------------- 参照

    pub fn shortcuts(&self) -> &[Shortcut] {
        &self.shortcuts
    }

    pub fn shortcut_count(&self) -> usize {
        self.shortcuts.len()
    }

    pub fn find_shortcut(&self, id: &str) -> Option<&Shortcut> {
        self.index.get(id).and_then(|&i| self.shortcuts.get(i))
    }

    pub fn find_shortcut_mut(&mut self, id: &str) -> Option<&mut Shortcut> {
        let i = *self.index.get(id)?;
        self.shortcuts.get_mut(i)
    }

    pub fn find_folder(&self, id: Option<&str>) -> Option<&Folder> {
        id.and_then(|id| self.folders.get(id))
    }

    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.get(id)
    }

    pub fn folders(&self) -> impl Iterator<Item = &Folder> {
        self.folders.values()
    }

    pub fn folder_count(&self) -> usize {
        self.folders.len()
    }

    pub fn root_folders(&self) -> &[String] {
        &self.root_folders
    }

    // -------------------------------------------------------------- フォルダ

    pub fn create_folder(&mut self, name: &str, parent_id: Option<&str>) -> DbResult<String> {
        let mut folder = Folder::new(name.trim(), parent_id.map(str::to_string));
        folder.sort_order = self.next_folder_sort_order(parent_id);

        self.conn().execute(
            "INSERT INTO folders (id, name, parent_id, sort_order, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                folder.id,
                folder.name,
                folder.parent_id,
                folder.sort_order,
                format_time(folder.created_at),
                format_time(folder.updated_at)
            ],
        )?;

        let id = folder.id.clone();
        self.folders.insert(id.clone(), folder);

        if self.batch_depth > 0 {
            // ツリーの組み直しは最後に 1 回だけ行う。続けて登録するショートカットの
            // フォルダ表示や重複判定に使うので、フルパスだけはここで決めておく
            let path = self.build_full_path(&id);
            if let Some(f) = self.folders.get_mut(&id) {
                f.full_path = path;
            }
            self.tree_dirty = true;
        } else {
            self.rebuild_folder_tree();
            self.changed();
        }
        Ok(id)
    }

    pub fn rename_folder(&mut self, id: &str, new_name: &str) -> DbResult<()> {
        let new_name = new_name.trim();
        let Some(folder) = self.folders.get_mut(id) else { return Ok(()) };
        if new_name.is_empty() || new_name == folder.name {
            return Ok(());
        }
        folder.name = new_name.to_string();
        folder.updated_at = Utc::now();
        let updated = format_time(folder.updated_at);
        self.conn().execute("UPDATE folders SET name=?1, updated_at=?2 WHERE id=?3", params![new_name, updated, id])?;
        self.rebuild_folder_tree();
        self.changed();
        Ok(())
    }

    /// フォルダを別の親へ移動する。循環参照になる場合は false を返す。
    pub fn move_folder(&mut self, id: &str, new_parent_id: Option<&str>) -> DbResult<bool> {
        if Some(id) == new_parent_id {
            return Ok(false);
        }
        if let Some(parent) = new_parent_id
            && self.is_descendant_of(parent, id)
        {
            return Ok(false);
        }
        let Some(current_parent) = self.folders.get(id).map(|f| f.parent_id.clone()) else { return Ok(false) };
        if current_parent.as_deref() == new_parent_id {
            return Ok(true);
        }

        let order = self.next_folder_sort_order(new_parent_id);
        let now = Utc::now();
        if let Some(f) = self.folders.get_mut(id) {
            f.parent_id = new_parent_id.map(str::to_string);
            f.sort_order = order;
            f.updated_at = now;
        }
        self.conn().execute(
            "UPDATE folders SET parent_id=?1, sort_order=?2, updated_at=?3 WHERE id=?4",
            params![new_parent_id, order, format_time(now), id],
        )?;
        self.rebuild_folder_tree();
        self.changed();
        Ok(true)
    }

    /// 親方向へたどり、ancestor_id が祖先（自身を含む）かどうかを判定する。
    pub fn is_descendant_of(&self, folder_id: &str, ancestor_id: &str) -> bool {
        let mut current = self.folders.get(folder_id);
        let mut guard = 0;
        while let Some(f) = current {
            if guard > 1000 {
                break;
            }
            guard += 1;
            if f.id == ancestor_id {
                return true;
            }
            current = f.parent_id.as_deref().and_then(|p| self.folders.get(p));
        }
        false
    }

    /// フォルダとその配下すべての ID（自身を含む）。
    pub fn collect_folder_and_descendants(&self, id: &str) -> Vec<String> {
        let mut children_of: HashMap<&str, Vec<&str>> = HashMap::new();
        for f in self.folders.values() {
            if let Some(p) = f.parent_id.as_deref() {
                children_of.entry(p).or_default().push(&f.id);
            }
        }

        let mut result = vec![id.to_string()];
        let mut visited: HashSet<&str> = HashSet::from([id]);
        let mut queue: VecDeque<&str> = VecDeque::from([id]);
        while let Some(current) = queue.pop_front() {
            if let Some(children) = children_of.get(current) {
                for &child in children {
                    if visited.insert(child) {
                        result.push(child.to_string());
                        queue.push_back(child);
                    }
                }
            }
        }
        result
    }

    /// フォルダを削除する。サブフォルダも削除される。
    /// delete_shortcuts が false の場合、含まれるショートカットは未分類へ移動する。
    pub fn delete_folder(&mut self, id: &str, delete_shortcuts: bool) -> DbResult<()> {
        let ids: HashSet<String> = self.collect_folder_and_descendants(id).into_iter().collect();

        {
            let tx = self.conn().unchecked_transaction()?;
            if delete_shortcuts {
                for s in &self.shortcuts {
                    if s.folder_id.as_ref().is_some_and(|f| ids.contains(f)) {
                        tx.execute("DELETE FROM shortcuts WHERE id=?1", [&s.id])?;
                    }
                }
            }
            // サブフォルダは ON DELETE CASCADE、残ったショートカットは ON DELETE SET NULL で処理される
            tx.execute("DELETE FROM folders WHERE id=?1", [id])?;
            tx.commit()?;
        }

        if delete_shortcuts {
            self.shortcuts.retain(|s| !s.folder_id.as_ref().is_some_and(|f| ids.contains(f)));
            self.rebuild_index();
        }
        for fid in &ids {
            self.folders.remove(fid);
        }
        for s in &mut self.shortcuts {
            if s.folder_id.as_ref().is_some_and(|f| ids.contains(f)) {
                s.folder_id = None;
                s.touch();
            }
        }

        self.clean_up_orphan_tags();
        self.rebuild_folder_tree();
        self.changed();
        Ok(())
    }

    fn next_folder_sort_order(&self, parent_id: Option<&str>) -> i64 {
        self.folders.values().filter(|f| f.parent_id.as_deref() == parent_id).map(|f| f.sort_order + 1).max().unwrap_or(0)
    }

    /// 表示言語を切り替えたあとに呼ぶ。「未分類」など言語で変わる表示を作り直す。
    pub fn refresh_localized_texts(&mut self) {
        self.rebuild_folder_tree();
        self.changed();
    }

    /// 親子関係と表示用フルパスを作り直す。
    pub fn rebuild_folder_tree(&mut self) {
        self.tree_dirty = false;

        let mut by_parent: HashMap<String, Vec<(i64, String, String)>> = HashMap::new();
        for f in self.folders.values() {
            by_parent.entry(f.parent_id.clone().unwrap_or_default()).or_default().push((f.sort_order, f.name.clone(), f.id.clone()));
        }
        for list in by_parent.values_mut() {
            list.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| culture_cmp(&a.1, &b.1)));
        }

        // 親が存在しないフォルダ（壊れたデータ）は C# 版と同じく表示されない
        self.root_folders = by_parent.get("").map(|l| l.iter().map(|x| x.2.clone()).collect()).unwrap_or_default();

        let ids: Vec<String> = self.folders.keys().cloned().collect();
        for id in &ids {
            let children = by_parent.get(id).map(|l| l.iter().map(|x| x.2.clone()).collect()).unwrap_or_default();
            let path = self.build_full_path(id);
            if let Some(f) = self.folders.get_mut(id) {
                f.children = children;
                f.full_path = path;
            }
        }

        let uncategorized = loc::t("Str.Folder.Uncategorized");
        let paths: HashMap<String, String> = self.folders.values().map(|f| (f.id.clone(), f.full_path.clone())).collect();
        for s in &mut self.shortcuts {
            let text = s.folder_id.as_ref().and_then(|f| paths.get(f)).cloned().unwrap_or_else(|| uncategorized.clone());
            if s.folder_path_text != text {
                s.folder_path_text = text;
                s.touch();
            }
        }
    }

    fn build_full_path(&self, id: &str) -> String {
        let mut parts = Vec::new();
        let mut current = self.folders.get(id);
        let mut guard = 0;
        while let Some(f) = current {
            if guard > 1000 {
                break;
            }
            guard += 1;
            parts.push(f.name.as_str());
            current = f.parent_id.as_deref().and_then(|p| self.folders.get(p));
        }
        parts.reverse();
        parts.join(" > ")
    }

    fn folder_path_text_for(&self, folder_id: Option<&str>) -> String {
        self.find_folder(folder_id).map(|f| f.full_path.clone()).unwrap_or_else(|| loc::t("Str.Folder.Uncategorized"))
    }

    // ----------------------------------------------------------- ショートカット

    /// 新しいショートカットを登録する。ID・作成日時・並び順はここで決める。
    pub fn add_shortcut(&mut self, mut item: Shortcut) -> DbResult<String> {
        let now = Utc::now();
        item.created_at = now;
        item.updated_at = now;
        item.sort_order = self.next_shortcut_sort_order(item.folder_id.as_deref());

        {
            let tx = self.db.conn().unchecked_transaction()?;
            tx.execute(
                "INSERT INTO shortcuts (id, folder_id, title, target, target_type, icon_path, note, \
                                        sort_order, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    item.id,
                    item.folder_id,
                    item.title,
                    item.target,
                    item.target_type as i64,
                    item.icon_path,
                    item.note,
                    item.sort_order,
                    format_time(item.created_at),
                    format_time(item.updated_at)
                ],
            )?;
            Self::sync_tags(&tx, &mut self.tags, &item)?;
            tx.commit()?;
        }

        item.folder_path_text = self.folder_path_text_for(item.folder_id.as_deref());
        item.touch();
        let id = item.id.clone();
        self.index.insert(id.clone(), self.shortcuts.len());
        self.shortcuts.push(item);
        if self.batch_depth == 0 {
            self.changed();
        }
        Ok(id)
    }

    /// 編集した内容で上書きする（使用履歴はそのまま）。
    pub fn update_shortcut(&mut self, edited: &Shortcut) -> DbResult<()> {
        let now = Utc::now();
        {
            let tx = self.db.conn().unchecked_transaction()?;
            tx.execute(
                "UPDATE shortcuts SET folder_id=?1, title=?2, target=?3, target_type=?4, \
                        icon_path=?5, note=?6, sort_order=?7, updated_at=?8 \
                 WHERE id=?9",
                params![
                    edited.folder_id,
                    edited.title,
                    edited.target,
                    edited.target_type as i64,
                    edited.icon_path,
                    edited.note,
                    edited.sort_order,
                    format_time(now),
                    edited.id
                ],
            )?;
            Self::sync_tags(&tx, &mut self.tags, edited)?;
            tx.commit()?;
        }

        let path_text = self.folder_path_text_for(edited.folder_id.as_deref());
        if let Some(s) = self.find_shortcut_mut(&edited.id) {
            s.folder_id = edited.folder_id.clone();
            s.title = edited.title.clone();
            s.target = edited.target.clone();
            s.target_type = edited.target_type;
            s.icon_path = edited.icon_path.clone();
            s.note = edited.note.clone();
            s.sort_order = edited.sort_order;
            s.tags = edited.tags.clone();
            s.updated_at = now;
            s.folder_path_text = path_text;
            s.touch();
        }

        self.clean_up_orphan_tags();
        if self.batch_depth == 0 {
            self.changed();
        }
        Ok(())
    }

    pub fn delete_shortcut(&mut self, id: &str) -> DbResult<()> {
        self.conn().execute("DELETE FROM shortcuts WHERE id=?1", [id])?;
        if let Some(i) = self.index.remove(id) {
            self.shortcuts.remove(i);
            self.rebuild_index();
        }
        self.clean_up_orphan_tags();
        if self.batch_depth == 0 {
            self.changed();
        }
        Ok(())
    }

    pub fn move_shortcut(&mut self, id: &str, folder_id: Option<&str>) -> DbResult<()> {
        let Some(current) = self.find_shortcut(id).map(|s| s.folder_id.clone()) else { return Ok(()) };
        if current.as_deref() == folder_id {
            return Ok(());
        }
        let order = self.next_shortcut_sort_order(folder_id);
        let now = Utc::now();
        self.conn().execute(
            "UPDATE shortcuts SET folder_id=?1, sort_order=?2, updated_at=?3 WHERE id=?4",
            params![folder_id, order, format_time(now), id],
        )?;
        let path_text = self.folder_path_text_for(folder_id);
        if let Some(s) = self.find_shortcut_mut(id) {
            s.folder_id = folder_id.map(str::to_string);
            s.sort_order = order;
            s.updated_at = now;
            s.folder_path_text = path_text;
            s.touch();
        }
        if self.batch_depth == 0 {
            self.changed();
        }
        Ok(())
    }

    /// アイコンのファイル名だけを更新する（一覧の作り直しは起こさない）。
    pub fn update_icon_path(&mut self, id: &str, icon_path: Option<&str>) -> DbResult<()> {
        let Some(s) = self.find_shortcut(id) else { return Ok(()) };
        if s.icon_path.as_deref() == icon_path {
            return Ok(());
        }
        let now = Utc::now();
        self.conn().execute("UPDATE shortcuts SET icon_path=?1, updated_at=?2 WHERE id=?3", params![icon_path, format_time(now), id])?;
        if let Some(s) = self.find_shortcut_mut(id) {
            s.icon_path = icon_path.map(str::to_string);
            s.updated_at = now;
        }
        Ok(())
    }

    /// ショートカットを起動したときに使用履歴を記録する（§56, §57）。
    pub fn record_usage(&mut self, id: &str) -> DbResult<()> {
        let now = Utc::now();
        self.conn().execute(
            "INSERT INTO shortcut_usage (shortcut_id, last_used_at, usage_count) VALUES (?1, ?2, 1) \
             ON CONFLICT(shortcut_id) DO UPDATE \
             SET last_used_at = excluded.last_used_at, usage_count = shortcut_usage.usage_count + 1",
            params![id, format_time(now)],
        )?;
        if let Some(s) = self.find_shortcut_mut(id) {
            s.usage_count += 1;
            s.last_used_at = Some(now);
        }
        self.changed();
        Ok(())
    }

    fn next_shortcut_sort_order(&self, folder_id: Option<&str>) -> i64 {
        self.shortcuts.iter().filter(|s| s.folder_id.as_deref() == folder_id).map(|s| s.sort_order + 1).max().unwrap_or(0)
    }

    // ------------------------------------------------------------------ タグ

    pub fn all_tag_names(&self) -> Vec<String> {
        self.tags.values().map(|(name, _)| name.clone()).collect()
    }

    pub fn tag_count(&self) -> usize {
        self.tags.len()
    }

    /// 使用件数つきのタグ一覧（多い順）。
    pub fn tag_usage(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, (String, usize)> = HashMap::new();
        for s in &self.shortcuts {
            for t in &s.tags {
                let e = counts.entry(fold(t)).or_insert_with(|| (t.clone(), 0));
                e.1 += 1;
            }
        }
        let mut list: Vec<(String, usize)> = counts.into_values().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| culture_cmp(&a.0, &b.0)));
        list
    }

    fn sync_tags(conn: &Connection, tags: &mut HashMap<String, (String, String)>, item: &Shortcut) -> DbResult<()> {
        conn.execute("DELETE FROM shortcut_tags WHERE shortcut_id=?1", [&item.id])?;
        let mut order = 0;
        for raw in &item.tags {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            let tag_id = Self::ensure_tag(conn, tags, name)?;
            conn.execute(
                "INSERT OR IGNORE INTO shortcut_tags (shortcut_id, tag_id, sort_order) VALUES (?1, ?2, ?3)",
                params![item.id, tag_id, order],
            )?;
            order += 1;
        }
        Ok(())
    }

    fn ensure_tag(conn: &Connection, tags: &mut HashMap<String, (String, String)>, name: &str) -> DbResult<String> {
        let key = fold(name);
        if let Some((_, id)) = tags.get(&key) {
            return Ok(id.clone());
        }
        // 大文字小文字違いのタグが DB にだけある場合（別のアプリが追加したなど）はそれを使う
        if let Some(id) =
            conn.query_row("SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE", [name], |r| r.get::<_, String>(0)).optional()?
        {
            tags.insert(key, (name.to_string(), id.clone()));
            return Ok(id);
        }
        let id = new_id();
        conn.execute("INSERT INTO tags (id, name) VALUES (?1, ?2)", params![id, name])?;
        tags.insert(key, (name.to_string(), id.clone()));
        Ok(id)
    }

    /// どのショートカットからも参照されなくなったタグを削除する。
    pub fn clean_up_orphan_tags(&mut self) {
        if self.batch_depth > 0 {
            self.orphan_tags_dirty = true;
            return;
        }
        self.orphan_tags_dirty = false;
        match self.conn().execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM shortcut_tags)", []) {
            Ok(n) if n > 0 => {
                if let Err(e) = self.load_tags() {
                    crate::log::warn_err("タグを読み直せませんでした。", &e);
                }
            }
            Ok(_) => {}
            Err(e) => crate::log::warn_err("未使用タグの整理に失敗しました。", &e),
        }
    }

    /// タグ名を一括変更する。
    pub fn rename_tag(&mut self, old_name: &str, new_name: &str) -> DbResult<()> {
        let new_name = new_name.trim();
        if new_name.is_empty() || old_name == new_name {
            return Ok(());
        }
        self.begin_batch();
        let result = (|| {
            let targets: Vec<Shortcut> =
                self.shortcuts.iter().filter(|s| s.tags.iter().any(|t| eq_ignore_case(t, old_name))).cloned().collect();
            for mut s in targets {
                let mut tags: Vec<String> = Vec::new();
                for t in &s.tags {
                    let replaced = if eq_ignore_case(t, old_name) { new_name.to_string() } else { t.clone() };
                    if !tags.iter().any(|x| eq_ignore_case(x, &replaced)) {
                        tags.push(replaced);
                    }
                }
                s.tags = tags;
                self.update_shortcut(&s)?;
            }
            Ok(())
        })();
        self.end_batch();
        result
    }

    pub fn delete_tag(&mut self, name: &str) -> DbResult<()> {
        self.begin_batch();
        let result = (|| {
            let targets: Vec<Shortcut> =
                self.shortcuts.iter().filter(|s| s.tags.iter().any(|t| eq_ignore_case(t, name))).cloned().collect();
            for mut s in targets {
                s.tags.retain(|t| !eq_ignore_case(t, name));
                self.update_shortcut(&s)?;
            }
            Ok(())
        })();
        self.end_batch();
        result
    }

    // ------------------------------------------------------------------ 設定

    pub fn load_settings_raw(&self) -> DbResult<HashMap<String, String>> {
        let mut stmt = self.conn().prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
        let mut map = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v.unwrap_or_default());
        }
        Ok(map)
    }

    pub fn save_settings_raw(&self, values: &[(String, String)]) -> DbResult<()> {
        let tx = self.conn().unchecked_transaction()?;
        for (k, v) in values {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------ 取り込み用

    /// ID・作成日時・並び順をそのまま保った状態でフォルダを書き込む（インポート専用）。
    /// メモリ側は更新しないので、呼び出し後に reload すること。
    pub fn insert_folder_raw(&self, folder: &Folder) -> DbResult<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO folders (id, name, parent_id, sort_order, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                folder.id,
                folder.name,
                folder.parent_id,
                folder.sort_order,
                format_time(folder.created_at),
                format_time(folder.updated_at)
            ],
        )?;
        Ok(())
    }

    /// ID・作成日時・タグ・使用履歴をそのまま保った状態でショートカットを書き込む（インポート専用）。
    pub fn insert_shortcut_raw(&mut self, item: &Shortcut) -> DbResult<()> {
        let conn = self.db.conn();
        conn.execute(
            "INSERT OR REPLACE INTO shortcuts (id, folder_id, title, target, target_type, icon_path, note, \
                                              sort_order, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                item.id,
                item.folder_id,
                item.title,
                item.target,
                item.target_type as i64,
                item.icon_path,
                item.note,
                item.sort_order,
                format_time(item.created_at),
                format_time(item.updated_at)
            ],
        )?;
        Self::sync_tags(conn, &mut self.tags, item)?;
        if item.usage_count > 0 || item.last_used_at.is_some() {
            conn.execute(
                "INSERT OR REPLACE INTO shortcut_usage (shortcut_id, last_used_at, usage_count) VALUES (?1, ?2, ?3)",
                params![item.id, item.last_used_at.map(format_time), item.usage_count],
            )?;
        }
        Ok(())
    }

    /// フォルダ・ショートカット・タグ・使用履歴をすべて削除する（置き換えインポート用）。
    pub fn clear_all_data(&mut self, include_settings: bool) -> DbResult<()> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM shortcut_tags; DELETE FROM shortcut_usage; DELETE FROM shortcuts; \
             DELETE FROM folders; DELETE FROM tags;",
        )?;
        if include_settings {
            tx.execute("DELETE FROM settings", [])?;
        }
        tx.commit()?;
        self.tags.clear();
        Ok(())
    }

    /// 取り込みで大量に書き込むときのトランザクション（commit / rollback を必ず呼ぶこと）。
    pub fn begin_raw_transaction(&self) -> DbResult<()> {
        self.conn().execute_batch("BEGIN")?;
        Ok(())
    }

    pub fn commit_raw_transaction(&self) -> DbResult<()> {
        self.conn().execute_batch("COMMIT")?;
        Ok(())
    }

    pub fn rollback_raw_transaction(&self) {
        let _ = self.conn().execute_batch("ROLLBACK");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("fsc-rs-test-{}", new_id()));
        let db = Database::open(&dir.join("store.sqlite")).unwrap();
        (Store::open(db).unwrap(), dir)
    }

    fn web(title: &str, url: &str) -> Shortcut {
        Shortcut { title: title.into(), target: url.into(), target_type: TargetType::Web, ..Shortcut::new() }
    }

    #[test]
    fn folders_tags_and_shortcuts_persist() {
        let (mut store, dir) = temp_store();
        let work = store.create_folder("仕事", None).unwrap();
        let dev = store.create_folder("開発", Some(&work)).unwrap();
        let mut item = web("GitHub", "https://github.com/");
        item.folder_id = Some(dev.clone());
        item.tags = vec!["code".into(), "Git".into()];
        let id = store.add_shortcut(item).unwrap();
        store.record_usage(&id).unwrap();

        assert_eq!(store.folder(&dev).unwrap().full_path, "仕事 > 開発");
        let path = store.database().file_path().to_path_buf();
        drop(store);

        let store = Store::open(Database::open(&path).unwrap()).unwrap();
        let s = store.find_shortcut(&id).unwrap();
        assert_eq!(s.tags, vec!["code".to_string(), "Git".to_string()]);
        assert_eq!(s.usage_count, 1);
        assert_eq!(s.folder_path_text, "仕事 > 開発");
        assert_eq!(store.tag_count(), 2);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_folder_keeps_or_deletes_shortcuts() {
        let (mut store, dir) = temp_store();
        let a = store.create_folder("A", None).unwrap();
        let b = store.create_folder("B", Some(&a)).unwrap();
        let mut item = web("x", "https://x.example/");
        item.folder_id = Some(b.clone());
        let id = store.add_shortcut(item).unwrap();

        store.delete_folder(&a, false).unwrap();
        assert!(store.folder(&b).is_none(), "サブフォルダも消える");
        assert_eq!(store.find_shortcut(&id).unwrap().folder_id, None, "未分類へ移る");

        let c = store.create_folder("C", None).unwrap();
        store.move_shortcut(&id, Some(&c)).unwrap();
        store.delete_folder(&c, true).unwrap();
        assert!(store.find_shortcut(&id).is_none(), "一緒に削除される");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_cannot_move_into_itself() {
        let (mut store, dir) = temp_store();
        let a = store.create_folder("A", None).unwrap();
        let b = store.create_folder("B", Some(&a)).unwrap();
        assert!(!store.move_folder(&a, Some(&b)).unwrap());
        assert!(store.move_folder(&b, None).unwrap());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_are_case_insensitive_and_orphans_are_removed() {
        let (mut store, dir) = temp_store();
        let mut a = web("a", "https://a.example/");
        a.tags = vec!["Dev".into()];
        let mut b = web("b", "https://b.example/");
        b.tags = vec!["dev".into()];
        let ida = store.add_shortcut(a).unwrap();
        store.add_shortcut(b).unwrap();
        assert_eq!(store.tag_count(), 1);
        assert_eq!(store.tag_usage()[0].1, 2);

        store.rename_tag("DEV", "開発").unwrap();
        assert_eq!(store.find_shortcut(&ida).unwrap().tags, vec!["開発".to_string()]);
        store.delete_tag("開発").unwrap();
        assert_eq!(store.tag_count(), 0);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// C# 版が書いた DB を読み書きできるかの確認（手動で実行する。FSC_COMPAT_DB に DB のパスを渡す）。
#[cfg(test)]
mod compat {
    use super::*;

    #[test]
    #[ignore]
    fn reads_and_writes_csharp_database() {
        let Ok(path) = std::env::var("FSC_COMPAT_DB") else { return };
        let mut store = Store::open(Database::open(std::path::Path::new(&path)).unwrap()).unwrap();
        println!(
            "読み込み: フォルダ {} / ショートカット {} / タグ {} / スキーマ {}",
            store.folder_count(),
            store.shortcut_count(),
            store.tag_count(),
            store.database().schema_version().unwrap()
        );
        for s in store.shortcuts() {
            if let Some(f) = &s.folder_id {
                assert!(store.folder(f).is_some(), "フォルダが見つからない項目がある");
            }
        }
        let settings = crate::settings::load(&store);
        println!(
            "設定: テーマ {:?} / ホットキー {} / 表示 {:?}",
            settings.theme,
            crate::keys::describe(settings.hotkey_modifiers, settings.hotkey_key),
            settings.view_mode
        );

        // Rust 版から書き込む（C# 版で読めるかを後で確かめる）
        let mut item = Shortcut::new();
        item.title = "Rust互換テスト".into();
        item.target = "https://example.com/rust-compat".into();
        item.target_type = TargetType::Web;
        item.tags = vec!["Rust版".into()];
        let id = store.add_shortcut(item).unwrap();
        store.record_usage(&id).unwrap();
        let folder = store.create_folder("Rust版で作ったフォルダ", None).unwrap();
        store.move_shortcut(&id, Some(&folder)).unwrap();
        let mut s2 = settings.clone();
        s2.main_window_width = 1111.0;
        crate::settings::save(&store, &s2).unwrap();
        println!("書き込み: {id}");
    }
}
