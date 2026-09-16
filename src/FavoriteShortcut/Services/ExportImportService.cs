using System.IO;
using System.IO.Compression;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

public enum ImportMode
{
    /// <summary>現在のデータを残したまま追加する。</summary>
    Merge = 0,

    /// <summary>現在のデータをすべて置き換える。</summary>
    Replace = 1,
}

public readonly record struct ImportSummary(
    int Folders, int Shortcuts, int SkippedShortcuts, string? BackupPath);

/// <summary>
/// エクスポート / インポート（§24〜§28）。
///
/// エクスポートは ZIP 1 つにまとめる:
///   database.sqlite  … VACUUM INTO で作った整合性のとれたコピー
///   icons/           … favicon キャッシュとカスタムアイコン
///   settings.json    … 設定（人が読める形）
///   manifest.json    … アプリ名/バージョン/スキーマ版/件数
///
/// インポートは必ず先に現在のDBをバックアップしてから行うので、
/// 失敗しても元のデータに戻せる（§47）。
/// </summary>
public sealed class ExportImportService
{
    private const string ManifestEntry = "manifest.json";
    private const string DatabaseEntry = "database.sqlite";
    private const string SettingsEntry = "settings.json";
    private const string IconsPrefix = "icons/";

    private readonly AppStore _store;
    private readonly SettingsService _settings;

    public ExportImportService(AppStore store, SettingsService settings)
    {
        _store = store;
        _settings = settings;
    }

    public static string SuggestedFileName =>
        $"favorite-shortcuts-{DateTime.Now:yyyyMMdd-HHmmss}.zip";

    // ---------------------------------------------------------------- export

    public void Export(string zipPath)
    {
        var temp = Path.Combine(Path.GetTempPath(), "fsc-export-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(temp);

        try
        {
            var dbCopy = Path.Combine(temp, DatabaseEntry);
            CopyDatabase(dbCopy);

            var manifest = new ExportManifest
            {
                Application = "お気に入りショートカット",
                FormatVersion = 1,
                SchemaVersion = Database.CurrentSchemaVersion,
                AppVersion = typeof(ExportImportService).Assembly.GetName().Version?.ToString() ?? "1.0.0",
                ExportedAt = DateTime.Now.ToString("o"),
                FolderCount = _store.AllFolders.Count,
                ShortcutCount = _store.Shortcuts.Count,
                TagCount = _store.AllTagNames.Count(),
            };

            File.WriteAllText(Path.Combine(temp, ManifestEntry),
                JsonSerializer.Serialize(manifest, JsonOptions), Encoding.UTF8);

            File.WriteAllText(Path.Combine(temp, SettingsEntry),
                JsonSerializer.Serialize(_settings.Current, JsonOptions), Encoding.UTF8);

            // アイコンは「実際に使われているもの」だけを入れる（ZIP が無駄に膨らまないように）
            var used = _store.Shortcuts
                .Select(s => s.IconPath)
                .Where(p => !string.IsNullOrEmpty(p))
                .ToHashSet(StringComparer.OrdinalIgnoreCase)!;

            var iconsDir = Path.Combine(temp, "icons");
            Directory.CreateDirectory(iconsDir);
            if (Directory.Exists(AppPaths.IconDirectory))
            {
                foreach (var file in Directory.EnumerateFiles(AppPaths.IconDirectory))
                {
                    var name = Path.GetFileName(file);
                    if (!used.Contains(name)) continue;
                    File.Copy(file, Path.Combine(iconsDir, name), overwrite: true);
                }
            }

            if (File.Exists(zipPath)) File.Delete(zipPath);
            ZipFile.CreateFromDirectory(temp, zipPath, CompressionLevel.Optimal, includeBaseDirectory: false);

            AppLog.Info($"エクスポートしました: {zipPath}");
        }
        finally
        {
            TryDeleteDirectory(temp);
        }
    }

    /// <summary>
    /// VACUUM INTO で整合性のとれたコピーを作る。
    /// WAL に残っている変更も含まれるので、ファイルを直接コピーするより安全。
    /// </summary>
    private void CopyDatabase(string destination)
    {
        _store.Database.Checkpoint();

        if (File.Exists(destination)) File.Delete(destination);

        try
        {
            using var cmd = _store.Database.CreateCommand("VACUUM INTO $path");
            cmd.Parameters.AddWithValue("$path", destination);
            cmd.ExecuteNonQuery();
        }
        catch (Exception ex)
        {
            // 古い SQLite などで VACUUM INTO が使えない場合はファイルコピーにフォールバック
            AppLog.Warn("VACUUM INTO に失敗したためファイルコピーで代替します。", ex);
            File.Copy(_store.Database.FilePath, destination, overwrite: true);
        }
    }

    // ---------------------------------------------------------------- import

    /// <summary>ZIP の中身を読んで、内容の概要を返す（インポート前の確認表示用）。</summary>
    public ExportManifest? PeekManifest(string zipPath)
    {
        try
        {
            using var archive = ZipFile.OpenRead(zipPath);
            var entry = archive.GetEntry(ManifestEntry);
            if (entry is null) return null;
            using var stream = entry.Open();
            return JsonSerializer.Deserialize<ExportManifest>(stream, JsonOptions);
        }
        catch (Exception ex)
        {
            AppLog.Warn($"マニフェストを読めませんでした: {zipPath}", ex);
            return null;
        }
    }

    public ImportSummary Import(string zipPath, ImportMode mode)
    {
        var temp = Path.Combine(Path.GetTempPath(), "fsc-import-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(temp);

        try
        {
            ExtractSafely(zipPath, temp);

            var importedDbPath = Path.Combine(temp, DatabaseEntry);
            if (!File.Exists(importedDbPath))
                throw new InvalidDataException("このファイルにはデータベースが含まれていません。");

            // 何かあっても戻せるよう、先に現在のDBを退避する
            var backupPath = CreateBackup("import-before");

            using var importedDb = new Database(importedDbPath);
            using var imported = new AppStore(importedDb);

            int folders, shortcuts, skipped;
            if (mode == ImportMode.Replace)
                (folders, shortcuts, skipped) = ImportReplace(imported, temp);
            else
                (folders, shortcuts, skipped) = ImportMerge(imported, temp);

            _store.Reload();
            _settings.Reload();

            AppLog.Info($"インポート完了 ({mode}): フォルダ {folders} / ショートカット {shortcuts} / スキップ {skipped}");
            return new ImportSummary(folders, shortcuts, skipped, backupPath);
        }
        finally
        {
            TryDeleteDirectory(temp);
        }
    }

    private (int Folders, int Shortcuts, int Skipped) ImportReplace(AppStore imported, string tempDir)
    {
        CopyIcons(tempDir);

        // ClearAllData / SaveSettingsRaw はそれぞれ自前でトランザクションを張るため、
        // ここで外側のトランザクションに入れ子にしないこと（SQLite は入れ子に対応していない）。
        _store.ClearAllData(includeSettings: true);

        using (var tx = _store.BeginTransaction())
        {
            // 親より先に子を入れると外部キー制約に触れるので、親から順に入れる
            foreach (var folder in OrderByDepth(imported))
                _store.InsertFolderRaw(folder);

            foreach (var shortcut in imported.Shortcuts)
                _store.InsertShortcutRaw(shortcut);

            tx.Commit();
        }

        _store.SaveSettingsRaw(imported.LoadSettingsRaw());

        return (imported.AllFolders.Count, imported.Shortcuts.Count, 0);
    }

    private (int Folders, int Shortcuts, int Skipped) ImportMerge(AppStore imported, string tempDir)
    {
        CopyIcons(tempDir);

        // 取り込み元のフォルダID -> 取り込み先のフォルダID
        var folderMap = new Dictionary<string, string>(StringComparer.Ordinal);

        // 既存フォルダを「フルパス」で引けるようにしておき、同じ構成なら作り直さない
        var existingByPath = new Dictionary<string, FolderItem>(StringComparer.CurrentCultureIgnoreCase);
        foreach (var f in _store.AllFolders) existingByPath[f.FullPath] = f;

        var addedFolders = 0;
        foreach (var folder in OrderByDepth(imported))
        {
            var path = BuildPath(imported, folder);
            if (existingByPath.TryGetValue(path, out var existing))
            {
                folderMap[folder.Id] = existing.Id;
                continue;
            }

            var parentId = folder.ParentId is not null && folderMap.TryGetValue(folder.ParentId, out var mapped)
                ? mapped
                : null;

            var created = _store.CreateFolder(folder.Name, parentId);
            folderMap[folder.Id] = created.Id;
            existingByPath[created.FullPath] = created;
            addedFolders++;
        }

        // 既存と同じ内容（フォルダ・タイトル・対象が一致）のショートカットは追加しない
        var existingKeys = _store.Shortcuts
            .Select(s => DuplicateKey(s.FolderPathText, s.Title, s.Target))
            .ToHashSet(StringComparer.OrdinalIgnoreCase);

        var addedShortcuts = 0;
        var skipped = 0;

        foreach (var source in imported.Shortcuts)
        {
            var folderPath = source.FolderId is not null && imported.FindFolder(source.FolderId) is { } f
                ? BuildPath(imported, f)
                : "未分類";

            var key = DuplicateKey(folderPath, source.Title, source.Target);
            if (!existingKeys.Add(key)) { skipped++; continue; }

            var copy = source.Clone();
            copy.Id = Guid.NewGuid().ToString("N");
            copy.FolderId = source.FolderId is not null && folderMap.TryGetValue(source.FolderId, out var fid)
                ? fid
                : null;

            _store.AddShortcut(copy);

            // 使用履歴も引き継ぐ（AddShortcut は履歴を書かないのでここで反映）
            if (source.UsageCount > 0 || source.LastUsedAt is not null)
            {
                copy.UsageCount = source.UsageCount;
                copy.LastUsedAt = source.LastUsedAt;
                _store.InsertShortcutRaw(copy);
            }

            addedShortcuts++;
        }

        return (addedFolders, addedShortcuts, skipped);
    }

    private static string DuplicateKey(string folderPath, string title, string target) =>
        $"{folderPath}{title}{target}";

    /// <summary>親フォルダが必ず先に来る順序で列挙する。</summary>
    private static List<FolderItem> OrderByDepth(AppStore store)
    {
        var byId = store.AllFolders.ToDictionary(f => f.Id, StringComparer.Ordinal);
        int Depth(FolderItem f)
        {
            var depth = 0;
            var current = f;
            while (current?.ParentId is not null && byId.TryGetValue(current.ParentId, out var parent) && depth < 1000)
            {
                depth++;
                current = parent;
            }
            return depth;
        }

        return store.AllFolders.OrderBy(Depth).ThenBy(f => f.SortOrder).ToList();
    }

    private static string BuildPath(AppStore store, FolderItem folder)
    {
        var parts = new List<string>();
        var current = folder;
        var guard = 0;
        while (current is not null && guard++ < 1000)
        {
            parts.Add(current.Name);
            current = store.FindFolder(current.ParentId);
        }
        parts.Reverse();
        return string.Join(" > ", parts);
    }

    private static void CopyIcons(string tempDir)
    {
        var source = Path.Combine(tempDir, "icons");
        if (!Directory.Exists(source)) return;

        Directory.CreateDirectory(AppPaths.IconDirectory);
        foreach (var file in Directory.EnumerateFiles(source))
        {
            try
            {
                File.Copy(file, Path.Combine(AppPaths.IconDirectory, Path.GetFileName(file)), overwrite: true);
            }
            catch (Exception ex)
            {
                AppLog.Warn($"アイコンのコピーに失敗: {file}", ex);
            }
        }
    }

    /// <summary>
    /// ZIP を展開する。展開先から外に出るパス（Zip Slip）を持つエントリは無視する。
    /// </summary>
    private static void ExtractSafely(string zipPath, string destination)
    {
        var root = Path.GetFullPath(destination + Path.DirectorySeparatorChar);
        using var archive = ZipFile.OpenRead(zipPath);

        foreach (var entry in archive.Entries)
        {
            if (entry.FullName.EndsWith('/') || entry.FullName.EndsWith('\\')) continue;

            // manifest / database / settings / icons 配下だけを受け入れる
            var name = entry.FullName.Replace('\\', '/');
            var allowed = name is ManifestEntry or DatabaseEntry or SettingsEntry
                          || name.StartsWith(IconsPrefix, StringComparison.OrdinalIgnoreCase);
            if (!allowed) continue;

            var targetPath = Path.GetFullPath(Path.Combine(destination, name));
            if (!targetPath.StartsWith(root, StringComparison.OrdinalIgnoreCase)) continue;

            Directory.CreateDirectory(Path.GetDirectoryName(targetPath)!);
            entry.ExtractToFile(targetPath, overwrite: true);
        }
    }

    // ---------------------------------------------------------------- backup

    /// <summary>現在のDBを backup/ にコピーする。戻り値は作成したファイルのパス。</summary>
    public string CreateBackup(string reason)
    {
        Directory.CreateDirectory(AppPaths.BackupDirectory);
        var name = $"{DateTime.Now:yyyyMMdd-HHmmss}-{reason}.sqlite";
        var path = Path.Combine(AppPaths.BackupDirectory, name);

        CopyDatabase(path);
        PruneBackups(keep: 10);
        return path;
    }

    private static void PruneBackups(int keep)
    {
        try
        {
            var files = Directory.EnumerateFiles(AppPaths.BackupDirectory, "*.sqlite")
                .OrderByDescending(File.GetLastWriteTimeUtc)
                .Skip(keep)
                .ToList();
            foreach (var file in files) File.Delete(file);
        }
        catch (Exception ex)
        {
            AppLog.Warn("古いバックアップの整理に失敗しました。", ex);
        }
    }

    private static void TryDeleteDirectory(string path)
    {
        try { if (Directory.Exists(path)) Directory.Delete(path, recursive: true); }
        catch (Exception ex) { AppLog.Warn($"一時フォルダを削除できませんでした: {path}", ex); }
    }

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };
}

public sealed class ExportManifest
{
    public string Application { get; set; } = string.Empty;
    public int FormatVersion { get; set; }
    public int SchemaVersion { get; set; }
    public string AppVersion { get; set; } = string.Empty;
    public string ExportedAt { get; set; } = string.Empty;
    public int FolderCount { get; set; }
    public int ShortcutCount { get; set; }
    public int TagCount { get; set; }
}
