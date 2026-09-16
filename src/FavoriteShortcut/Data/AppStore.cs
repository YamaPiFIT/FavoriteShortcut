using System.Collections.ObjectModel;
using System.Globalization;
using FavoriteShortcut.Models;
using Microsoft.Data.Sqlite;

namespace FavoriteShortcut.Data;

/// <summary>
/// SQLite への永続化と、メモリ上のキャッシュをまとめて扱うストア。
///
/// 数千件規模でも検索を高速にするため、起動時に全件をメモリへ読み込み、
/// 検索・並べ替えはメモリ上で行う。書き込みは常に SQLite へ即時反映する。
/// </summary>
public sealed class AppStore : IDisposable
{
    private readonly Database _db;
    private readonly Dictionary<string, FolderItem> _folders = new(StringComparer.Ordinal);
    private readonly Dictionary<string, ShortcutItem> _shortcuts = new(StringComparer.Ordinal);

    /// <summary>タグ名 -> タグID（大文字小文字を区別しない）。</summary>
    private readonly Dictionary<string, string> _tagIds = new(StringComparer.OrdinalIgnoreCase);

    public AppStore(Database db)
    {
        _db = db;
        Load();
    }

    public Database Database => _db;

    /// <summary>ツリーの最上位フォルダ。</summary>
    public ObservableCollection<FolderItem> RootFolders { get; } = new();

    /// <summary>全ショートカット（表示順は各画面で決める）。</summary>
    public ObservableCollection<ShortcutItem> Shortcuts { get; } = new();

    public IReadOnlyCollection<FolderItem> AllFolders => _folders.Values;

    /// <summary>構成が変わったときに上がる。検索インデックスの無効化などに使う。</summary>
    public event EventHandler? DataChanged;

    public int Revision { get; private set; }

    private void RaiseChanged()
    {
        Revision++;
        DataChanged?.Invoke(this, EventArgs.Empty);
    }

    // ------------------------------------------------------------------ load

    private void Load()
    {
        LoadFolders();
        LoadTags();
        LoadShortcuts();
        RebuildFolderTree();
        RaiseChanged();
    }

    private void LoadFolders()
    {
        _folders.Clear();
        using var cmd = _db.CreateCommand(
            "SELECT id, name, parent_id, sort_order, created_at, updated_at FROM folders");
        using var r = cmd.ExecuteReader();
        while (r.Read())
        {
            var f = new FolderItem
            {
                Id = r.GetString(0),
                Name = r.GetString(1),
                ParentId = r.IsDBNull(2) ? null : r.GetString(2),
                SortOrder = r.GetInt32(3),
                CreatedAt = ParseTime(r.GetString(4)),
                UpdatedAt = ParseTime(r.GetString(5)),
            };
            _folders[f.Id] = f;
        }
    }

    private void LoadTags()
    {
        _tagIds.Clear();
        using var cmd = _db.CreateCommand("SELECT id, name FROM tags");
        using var r = cmd.ExecuteReader();
        while (r.Read())
            _tagIds[r.GetString(1)] = r.GetString(0);
    }

    private void LoadShortcuts()
    {
        _shortcuts.Clear();
        Shortcuts.Clear();

        using (var cmd = _db.CreateCommand(
            "SELECT s.id, s.folder_id, s.title, s.target, s.target_type, s.icon_path, s.note, " +
            "       s.sort_order, s.created_at, s.updated_at, u.usage_count, u.last_used_at " +
            "FROM shortcuts s LEFT JOIN shortcut_usage u ON u.shortcut_id = s.id"))
        using (var r = cmd.ExecuteReader())
        {
            while (r.Read())
            {
                var s = new ShortcutItem
                {
                    Id = r.GetString(0),
                    FolderId = r.IsDBNull(1) ? null : r.GetString(1),
                    Title = r.GetString(2),
                    Target = r.GetString(3),
                    TargetType = (TargetType)r.GetInt32(4),
                    IconPath = r.IsDBNull(5) ? null : r.GetString(5),
                    Note = r.IsDBNull(6) ? null : r.GetString(6),
                    SortOrder = r.GetInt32(7),
                    CreatedAt = ParseTime(r.GetString(8)),
                    UpdatedAt = ParseTime(r.GetString(9)),
                    UsageCount = r.IsDBNull(10) ? 0 : r.GetInt32(10),
                    LastUsedAt = r.IsDBNull(11) ? null : ParseTime(r.GetString(11)),
                };
                _shortcuts[s.Id] = s;
            }
        }

        // タグはまとめて読み込む（N+1 クエリを避ける）
        var tagNameById = _tagIds.ToDictionary(kv => kv.Value, kv => kv.Key, StringComparer.Ordinal);
        var perShortcut = new Dictionary<string, List<string>>(StringComparer.Ordinal);
        using (var cmd = _db.CreateCommand(
            "SELECT shortcut_id, tag_id FROM shortcut_tags ORDER BY shortcut_id, sort_order"))
        using (var r = cmd.ExecuteReader())
        {
            while (r.Read())
            {
                var sid = r.GetString(0);
                if (!tagNameById.TryGetValue(r.GetString(1), out var name)) continue;
                if (!perShortcut.TryGetValue(sid, out var list))
                    perShortcut[sid] = list = new List<string>();
                list.Add(name);
            }
        }

        foreach (var s in _shortcuts.Values)
        {
            if (perShortcut.TryGetValue(s.Id, out var tags)) s.Tags = tags;
            Shortcuts.Add(s);
        }
    }

    // ---------------------------------------------------------------- folders

    public FolderItem? FindFolder(string? id) =>
        id is not null && _folders.TryGetValue(id, out var f) ? f : null;

    public ShortcutItem? FindShortcut(string id) =>
        _shortcuts.TryGetValue(id, out var s) ? s : null;

    public FolderItem CreateFolder(string name, string? parentId)
    {
        var now = DateTime.UtcNow;
        var folder = new FolderItem
        {
            Name = name.Trim(),
            ParentId = parentId,
            SortOrder = NextFolderSortOrder(parentId),
            CreatedAt = now,
            UpdatedAt = now,
        };

        using var cmd = _db.CreateCommand(
            "INSERT INTO folders (id, name, parent_id, sort_order, created_at, updated_at) " +
            "VALUES ($id, $name, $parent, $order, $created, $updated)");
        cmd.Parameters.AddWithValue("$id", folder.Id);
        cmd.Parameters.AddWithValue("$name", folder.Name);
        cmd.Parameters.AddWithValue("$parent", (object?)folder.ParentId ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$order", folder.SortOrder);
        cmd.Parameters.AddWithValue("$created", FormatTime(folder.CreatedAt));
        cmd.Parameters.AddWithValue("$updated", FormatTime(folder.UpdatedAt));
        cmd.ExecuteNonQuery();

        _folders[folder.Id] = folder;
        RebuildFolderTree();
        RaiseChanged();
        return folder;
    }

    public void RenameFolder(FolderItem folder, string newName)
    {
        newName = newName.Trim();
        if (newName.Length == 0 || newName == folder.Name) return;

        folder.Name = newName;
        folder.UpdatedAt = DateTime.UtcNow;

        using var cmd = _db.CreateCommand(
            "UPDATE folders SET name=$name, updated_at=$updated WHERE id=$id");
        cmd.Parameters.AddWithValue("$name", folder.Name);
        cmd.Parameters.AddWithValue("$updated", FormatTime(folder.UpdatedAt));
        cmd.Parameters.AddWithValue("$id", folder.Id);
        cmd.ExecuteNonQuery();

        RebuildFolderTree();
        RaiseChanged();
    }

    /// <summary>フォルダを別の親へ移動する。循環参照になる場合は false を返す。</summary>
    public bool MoveFolder(FolderItem folder, string? newParentId)
    {
        if (folder.Id == newParentId) return false;
        if (newParentId is not null && IsDescendantOf(newParentId, folder.Id)) return false;
        if (folder.ParentId == newParentId) return true;

        folder.ParentId = newParentId;
        folder.SortOrder = NextFolderSortOrder(newParentId);
        folder.UpdatedAt = DateTime.UtcNow;

        using var cmd = _db.CreateCommand(
            "UPDATE folders SET parent_id=$parent, sort_order=$order, updated_at=$updated WHERE id=$id");
        cmd.Parameters.AddWithValue("$parent", (object?)newParentId ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$order", folder.SortOrder);
        cmd.Parameters.AddWithValue("$updated", FormatTime(folder.UpdatedAt));
        cmd.Parameters.AddWithValue("$id", folder.Id);
        cmd.ExecuteNonQuery();

        RebuildFolderTree();
        RaiseChanged();
        return true;
    }

    /// <summary>親方向へたどり、ancestorId が祖先（自身を含む）かどうかを判定する。</summary>
    public bool IsDescendantOf(string folderId, string ancestorId)
    {
        var current = FindFolder(folderId);
        var guard = 0;
        while (current is not null && guard++ < 1000)
        {
            if (current.Id == ancestorId) return true;
            current = FindFolder(current.ParentId);
        }
        return false;
    }

    /// <summary>
    /// フォルダを削除する。サブフォルダも削除される。
    /// deleteShortcuts が false の場合、含まれるショートカットは未分類へ移動する。
    /// </summary>
    public void DeleteFolder(FolderItem folder, bool deleteShortcuts)
    {
        var ids = CollectFolderAndDescendants(folder).Select(f => f.Id).ToHashSet(StringComparer.Ordinal);

        using (var tx = _db.BeginTransaction())
        {
            if (deleteShortcuts)
            {
                foreach (var s in _shortcuts.Values
                             .Where(s => s.FolderId is not null && ids.Contains(s.FolderId)).ToList())
                {
                    DeleteShortcutCore(s);
                    _shortcuts.Remove(s.Id);
                    Shortcuts.Remove(s);
                }
            }

            // サブフォルダは ON DELETE CASCADE、残ったショートカットは ON DELETE SET NULL で処理される
            using var cmd = _db.CreateCommand("DELETE FROM folders WHERE id=$id");
            cmd.Parameters.AddWithValue("$id", folder.Id);
            cmd.ExecuteNonQuery();
            tx.Commit();
        }

        foreach (var id in ids) _folders.Remove(id);
        foreach (var s in _shortcuts.Values)
            if (s.FolderId is not null && ids.Contains(s.FolderId)) s.FolderId = null;

        CleanUpOrphanTags();
        RebuildFolderTree();
        RaiseChanged();
    }

    public List<FolderItem> CollectFolderAndDescendants(FolderItem folder)
    {
        var result = new List<FolderItem> { folder };
        var queue = new Queue<FolderItem>();
        queue.Enqueue(folder);
        while (queue.Count > 0)
        {
            var current = queue.Dequeue();
            foreach (var child in _folders.Values.Where(f => f.ParentId == current.Id))
            {
                result.Add(child);
                queue.Enqueue(child);
            }
        }
        return result;
    }

    /// <summary>ドラッグ＆ドロップによるフォルダの並べ替え／移動。</summary>
    public void ReorderFolder(FolderItem folder, string? newParentId, int newIndex)
    {
        if (newParentId is not null && (folder.Id == newParentId || IsDescendantOf(newParentId, folder.Id)))
            return;

        var siblings = _folders.Values
            .Where(f => f.ParentId == newParentId && f.Id != folder.Id)
            .OrderBy(f => f.SortOrder).ThenBy(f => f.Name, StringComparer.CurrentCulture)
            .ToList();

        newIndex = Math.Clamp(newIndex, 0, siblings.Count);
        siblings.Insert(newIndex, folder);
        folder.ParentId = newParentId;

        using (var tx = _db.BeginTransaction())
        {
            for (var i = 0; i < siblings.Count; i++)
            {
                siblings[i].SortOrder = i;
                using var cmd = _db.CreateCommand(
                    "UPDATE folders SET parent_id=$parent, sort_order=$order, updated_at=$updated WHERE id=$id");
                cmd.Parameters.AddWithValue("$parent", (object?)siblings[i].ParentId ?? DBNull.Value);
                cmd.Parameters.AddWithValue("$order", i);
                cmd.Parameters.AddWithValue("$updated", FormatTime(DateTime.UtcNow));
                cmd.Parameters.AddWithValue("$id", siblings[i].Id);
                cmd.ExecuteNonQuery();
            }
            tx.Commit();
        }

        RebuildFolderTree();
        RaiseChanged();
    }

    private int NextFolderSortOrder(string? parentId)
    {
        var siblings = _folders.Values.Where(f => f.ParentId == parentId).ToList();
        return siblings.Count == 0 ? 0 : siblings.Max(f => f.SortOrder) + 1;
    }

    /// <summary>親子関係と表示用フルパスを作り直す。</summary>
    public void RebuildFolderTree()
    {
        foreach (var f in _folders.Values) f.Children.Clear();

        var byParent = _folders.Values
            .GroupBy(f => f.ParentId ?? string.Empty)
            .ToDictionary(g => g.Key, g => g
                .OrderBy(f => f.SortOrder)
                .ThenBy(f => f.Name, StringComparer.CurrentCulture)
                .ToList(), StringComparer.Ordinal);

        RootFolders.Clear();
        if (byParent.TryGetValue(string.Empty, out var roots))
            foreach (var f in roots) RootFolders.Add(f);

        foreach (var f in _folders.Values)
            if (byParent.TryGetValue(f.Id, out var children))
                foreach (var c in children) f.Children.Add(c);

        foreach (var f in _folders.Values) f.FullPath = BuildFullPath(f);
        foreach (var s in _shortcuts.Values)
            s.FolderPathText = s.FolderId is not null && _folders.TryGetValue(s.FolderId, out var sf)
                ? sf.FullPath
                : "未分類";
    }

    private string BuildFullPath(FolderItem folder)
    {
        var parts = new List<string>();
        var current = folder;
        var guard = 0;
        while (current is not null && guard++ < 1000)
        {
            parts.Add(current.Name);
            current = FindFolder(current.ParentId);
        }
        parts.Reverse();
        return string.Join(" > ", parts);
    }

    // -------------------------------------------------------------- shortcuts

    public void AddShortcut(ShortcutItem item)
    {
        item.CreatedAt = item.UpdatedAt = DateTime.UtcNow;
        item.SortOrder = NextShortcutSortOrder(item.FolderId);

        using (var tx = _db.BeginTransaction())
        {
            using (var cmd = _db.CreateCommand(
                "INSERT INTO shortcuts (id, folder_id, title, target, target_type, icon_path, note, " +
                "                       sort_order, created_at, updated_at) " +
                "VALUES ($id, $folder, $title, $target, $type, $icon, $note, $order, $created, $updated)"))
            {
                BindShortcut(cmd, item);
                cmd.ExecuteNonQuery();
            }
            SyncTags(item);
            tx.Commit();
        }

        _shortcuts[item.Id] = item;
        Shortcuts.Add(item);
        item.FolderPathText = FindFolder(item.FolderId)?.FullPath ?? "未分類";
        item.SearchIndex = null;
        RaiseChanged();
    }

    public void UpdateShortcut(ShortcutItem item)
    {
        item.UpdatedAt = DateTime.UtcNow;

        using (var tx = _db.BeginTransaction())
        {
            using (var cmd = _db.CreateCommand(
                "UPDATE shortcuts SET folder_id=$folder, title=$title, target=$target, target_type=$type, " +
                "       icon_path=$icon, note=$note, sort_order=$order, updated_at=$updated " +
                "WHERE id=$id"))
            {
                BindShortcut(cmd, item);
                cmd.ExecuteNonQuery();
            }
            SyncTags(item);
            tx.Commit();
        }

        CleanUpOrphanTags();
        item.FolderPathText = FindFolder(item.FolderId)?.FullPath ?? "未分類";
        item.SearchIndex = null;
        RaiseChanged();
    }

    public void DeleteShortcut(ShortcutItem item)
    {
        using (var tx = _db.BeginTransaction())
        {
            DeleteShortcutCore(item);
            tx.Commit();
        }

        _shortcuts.Remove(item.Id);
        Shortcuts.Remove(item);
        CleanUpOrphanTags();
        RaiseChanged();
    }

    private void DeleteShortcutCore(ShortcutItem item)
    {
        using var cmd = _db.CreateCommand("DELETE FROM shortcuts WHERE id=$id");
        cmd.Parameters.AddWithValue("$id", item.Id);
        cmd.ExecuteNonQuery();
    }

    public void MoveShortcut(ShortcutItem item, string? folderId)
    {
        if (item.FolderId == folderId) return;
        item.FolderId = folderId;
        item.SortOrder = NextShortcutSortOrder(folderId);
        item.UpdatedAt = DateTime.UtcNow;

        using var cmd = _db.CreateCommand(
            "UPDATE shortcuts SET folder_id=$folder, sort_order=$order, updated_at=$updated WHERE id=$id");
        cmd.Parameters.AddWithValue("$folder", (object?)folderId ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$order", item.SortOrder);
        cmd.Parameters.AddWithValue("$updated", FormatTime(item.UpdatedAt));
        cmd.Parameters.AddWithValue("$id", item.Id);
        cmd.ExecuteNonQuery();

        item.FolderPathText = FindFolder(folderId)?.FullPath ?? "未分類";
        item.SearchIndex = null;
        RaiseChanged();
    }

    /// <summary>
    /// アイコンのファイル名だけを更新する。
    /// favicon は表示のたびに裏で取得されるため、通常の更新（タグの再登録や
    /// DataChanged の発火）まで行うと一覧の再構築が連鎖してしまう。
    /// </summary>
    public void UpdateIconPath(ShortcutItem item, string? iconPath)
    {
        if (item.IconPath == iconPath) return;

        item.IconPath = iconPath;
        item.UpdatedAt = DateTime.UtcNow;

        using var cmd = _db.CreateCommand(
            "UPDATE shortcuts SET icon_path=$icon, updated_at=$updated WHERE id=$id");
        cmd.Parameters.AddWithValue("$icon", (object?)iconPath ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$updated", FormatTime(item.UpdatedAt));
        cmd.Parameters.AddWithValue("$id", item.Id);
        cmd.ExecuteNonQuery();
    }

    /// <summary>ショートカットを起動したときに使用履歴を記録する（§56, §57）。</summary>
    public void RecordUsage(ShortcutItem item)
    {
        item.UsageCount++;
        item.LastUsedAt = DateTime.UtcNow;

        using var cmd = _db.CreateCommand(
            "INSERT INTO shortcut_usage (shortcut_id, last_used_at, usage_count) VALUES ($id, $last, 1) " +
            "ON CONFLICT(shortcut_id) DO UPDATE " +
            "SET last_used_at = excluded.last_used_at, usage_count = shortcut_usage.usage_count + 1");
        cmd.Parameters.AddWithValue("$id", item.Id);
        cmd.Parameters.AddWithValue("$last", FormatTime(item.LastUsedAt.Value));
        cmd.ExecuteNonQuery();
    }

    private int NextShortcutSortOrder(string? folderId)
    {
        var siblings = _shortcuts.Values.Where(s => s.FolderId == folderId).ToList();
        return siblings.Count == 0 ? 0 : siblings.Max(s => s.SortOrder) + 1;
    }

    private void BindShortcut(SqliteCommand cmd, ShortcutItem item)
    {
        cmd.Parameters.AddWithValue("$id", item.Id);
        cmd.Parameters.AddWithValue("$folder", (object?)item.FolderId ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$title", item.Title ?? string.Empty);
        cmd.Parameters.AddWithValue("$target", item.Target ?? string.Empty);
        cmd.Parameters.AddWithValue("$type", (int)item.TargetType);
        cmd.Parameters.AddWithValue("$icon", (object?)item.IconPath ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$note", (object?)item.Note ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$order", item.SortOrder);
        cmd.Parameters.AddWithValue("$created", FormatTime(item.CreatedAt));
        cmd.Parameters.AddWithValue("$updated", FormatTime(item.UpdatedAt));
    }

    // ------------------------------------------------------------------- tags

    public IEnumerable<string> AllTagNames => _tagIds.Keys;

    /// <summary>使用件数つきのタグ一覧（多い順）。</summary>
    public List<(string Name, int Count)> GetTagUsage()
    {
        var counts = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        foreach (var s in _shortcuts.Values)
            foreach (var t in s.Tags)
                counts[t] = counts.GetValueOrDefault(t) + 1;

        return counts
            .Select(kv => (Name: kv.Key, Count: kv.Value))
            .OrderByDescending(x => x.Count)
            .ThenBy(x => x.Name, StringComparer.CurrentCulture)
            .ToList();
    }

    private string EnsureTag(string name)
    {
        if (_tagIds.TryGetValue(name, out var existing)) return existing;

        var id = Guid.NewGuid().ToString("N");
        using var cmd = _db.CreateCommand("INSERT INTO tags (id, name) VALUES ($id, $name)");
        cmd.Parameters.AddWithValue("$id", id);
        cmd.Parameters.AddWithValue("$name", name);
        cmd.ExecuteNonQuery();
        _tagIds[name] = id;
        return id;
    }

    private void SyncTags(ShortcutItem item)
    {
        using (var del = _db.CreateCommand("DELETE FROM shortcut_tags WHERE shortcut_id=$id"))
        {
            del.Parameters.AddWithValue("$id", item.Id);
            del.ExecuteNonQuery();
        }

        var order = 0;
        foreach (var raw in item.Tags)
        {
            var name = raw.Trim();
            if (name.Length == 0) continue;
            var tagId = EnsureTag(name);
            using var cmd = _db.CreateCommand(
                "INSERT OR IGNORE INTO shortcut_tags (shortcut_id, tag_id, sort_order) VALUES ($s, $t, $o)");
            cmd.Parameters.AddWithValue("$s", item.Id);
            cmd.Parameters.AddWithValue("$t", tagId);
            cmd.Parameters.AddWithValue("$o", order++);
            cmd.ExecuteNonQuery();
        }
    }

    /// <summary>どのショートカットからも参照されなくなったタグを削除する。</summary>
    public void CleanUpOrphanTags()
    {
        using var cmd = _db.CreateCommand(
            "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM shortcut_tags)");
        if (cmd.ExecuteNonQuery() > 0) LoadTags();
    }

    /// <summary>タグ名を一括変更する。</summary>
    public void RenameTag(string oldName, string newName)
    {
        newName = newName.Trim();
        if (newName.Length == 0 || string.Equals(oldName, newName, StringComparison.Ordinal)) return;

        foreach (var s in _shortcuts.Values.ToList())
        {
            if (!s.Tags.Any(t => string.Equals(t, oldName, StringComparison.OrdinalIgnoreCase))) continue;
            var tags = new List<string>();
            foreach (var t in s.Tags)
            {
                var replaced = string.Equals(t, oldName, StringComparison.OrdinalIgnoreCase) ? newName : t;
                if (!tags.Any(x => string.Equals(x, replaced, StringComparison.OrdinalIgnoreCase)))
                    tags.Add(replaced);
            }
            s.Tags = tags;
            UpdateShortcut(s);
        }
    }

    public void DeleteTag(string name)
    {
        foreach (var s in _shortcuts.Values.ToList())
        {
            if (!s.Tags.Any(t => string.Equals(t, name, StringComparison.OrdinalIgnoreCase))) continue;
            s.Tags = s.Tags.Where(t => !string.Equals(t, name, StringComparison.OrdinalIgnoreCase)).ToList();
            UpdateShortcut(s);
        }
    }

    // --------------------------------------------------------------- settings

    public Dictionary<string, string> LoadSettingsRaw()
    {
        var result = new Dictionary<string, string>(StringComparer.Ordinal);
        using var cmd = _db.CreateCommand("SELECT key, value FROM settings");
        using var r = cmd.ExecuteReader();
        while (r.Read())
            result[r.GetString(0)] = r.IsDBNull(1) ? string.Empty : r.GetString(1);
        return result;
    }

    public void SaveSettingsRaw(IReadOnlyDictionary<string, string> values)
    {
        using var tx = _db.BeginTransaction();
        foreach (var kv in values)
        {
            using var cmd = _db.CreateCommand(
                "INSERT INTO settings (key, value) VALUES ($k, $v) " +
                "ON CONFLICT(key) DO UPDATE SET value = excluded.value");
            cmd.Parameters.AddWithValue("$k", kv.Key);
            cmd.Parameters.AddWithValue("$v", kv.Value);
            cmd.ExecuteNonQuery();
        }
        tx.Commit();
    }

    /// <summary>インポート後などに、DB からすべて読み込み直す。</summary>
    public void Reload() => Load();

    // ----------------------------------------------------------- import 用 API

    /// <summary>
    /// ID・作成日時・並び順をそのまま保った状態でフォルダを書き込む（インポート専用）。
    /// メモリ側は更新しないので、呼び出し後に <see cref="Reload"/> すること。
    /// </summary>
    public void InsertFolderRaw(FolderItem folder)
    {
        using var cmd = _db.CreateCommand(
            "INSERT OR REPLACE INTO folders (id, name, parent_id, sort_order, created_at, updated_at) " +
            "VALUES ($id, $name, $parent, $order, $created, $updated)");
        cmd.Parameters.AddWithValue("$id", folder.Id);
        cmd.Parameters.AddWithValue("$name", folder.Name);
        cmd.Parameters.AddWithValue("$parent", (object?)folder.ParentId ?? DBNull.Value);
        cmd.Parameters.AddWithValue("$order", folder.SortOrder);
        cmd.Parameters.AddWithValue("$created", FormatTime(folder.CreatedAt));
        cmd.Parameters.AddWithValue("$updated", FormatTime(folder.UpdatedAt));
        cmd.ExecuteNonQuery();
    }

    /// <summary>
    /// ID・作成日時・タグ・使用履歴をそのまま保った状態でショートカットを書き込む（インポート専用）。
    /// メモリ側は更新しないので、呼び出し後に <see cref="Reload"/> すること。
    /// </summary>
    public void InsertShortcutRaw(ShortcutItem item)
    {
        using (var cmd = _db.CreateCommand(
            "INSERT OR REPLACE INTO shortcuts (id, folder_id, title, target, target_type, icon_path, note, " +
            "                                  sort_order, created_at, updated_at) " +
            "VALUES ($id, $folder, $title, $target, $type, $icon, $note, $order, $created, $updated)"))
        {
            BindShortcut(cmd, item);
            cmd.ExecuteNonQuery();
        }

        SyncTags(item);

        if (item.UsageCount > 0 || item.LastUsedAt is not null)
        {
            using var usage = _db.CreateCommand(
                "INSERT OR REPLACE INTO shortcut_usage (shortcut_id, last_used_at, usage_count) " +
                "VALUES ($id, $last, $count)");
            usage.Parameters.AddWithValue("$id", item.Id);
            usage.Parameters.AddWithValue("$last",
                item.LastUsedAt is { } l ? FormatTime(l) : (object)DBNull.Value);
            usage.Parameters.AddWithValue("$count", item.UsageCount);
            usage.ExecuteNonQuery();
        }
    }

    /// <summary>フォルダ・ショートカット・タグ・使用履歴をすべて削除する（置き換えインポート用）。</summary>
    public void ClearAllData(bool includeSettings)
    {
        using var tx = _db.BeginTransaction();
        _db.Exec("DELETE FROM shortcut_tags;");
        _db.Exec("DELETE FROM shortcut_usage;");
        _db.Exec("DELETE FROM shortcuts;");
        _db.Exec("DELETE FROM folders;");
        _db.Exec("DELETE FROM tags;");
        if (includeSettings) _db.Exec("DELETE FROM settings;");
        tx.Commit();
    }

    public SqliteTransaction BeginTransaction() => _db.BeginTransaction();

    // ---------------------------------------------------------------- helpers

    internal static string FormatTime(DateTime value) =>
        value.ToUniversalTime().ToString("o", CultureInfo.InvariantCulture);

    /// <summary>
    /// 保存した ISO 8601 文字列を UTC の DateTime に戻す。
    /// 解釈できない値でも例外にせず現在時刻にフォールバックし、
    /// 1 レコードの不整合でアプリ全体が起動しなくなることを避ける。
    /// </summary>
    internal static DateTime ParseTime(string value) =>
        DateTime.TryParse(value, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out var dt)
            ? dt.Kind == DateTimeKind.Unspecified
                ? DateTime.SpecifyKind(dt, DateTimeKind.Utc)
                : dt.ToUniversalTime()
            : DateTime.UtcNow;

    public void Dispose() => _db.Dispose();
}
