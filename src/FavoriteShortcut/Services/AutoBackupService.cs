using System.Globalization;
using System.IO;
using System.Text.Json;
using System.Windows.Threading;
using FavoriteShortcut.Data;
using Microsoft.Data.Sqlite;

namespace FavoriteShortcut.Services;

/// <summary>
/// 1 日 1 回の自動バックアップ（§47 データ保護）。
///
/// 起動して 1 分後と、起動したままなら 30 分ごとに「今日まだ作っていないか」を確かめ、
/// 前回の自動バックアップから登録内容（フォルダ・ショートカット・タグ）が変わっていれば作る。
/// 変わっていない日は作らないので、使わない日が続いても古い世代が押し出されない。
///
/// コピーは画面とは別の接続で VACUUM INTO を使い、裏のスレッドで行う（画面を止めない）。
/// 自動の分は最新 <see cref="KeepCount"/> 回分を残し、手動・取り込み前のバックアップとは別に数える。
/// </summary>
public sealed class AutoBackupService : IDisposable
{
    /// <summary>自動バックアップを残す数。</summary>
    public const int KeepCount = 14;

    private const string FileSuffix = "-auto.sqlite";
    private const string StateFileName = "auto-backup.json";

    private readonly AppStore _store;
    private readonly SettingsService _settings;
    private DispatcherTimer? _timer;
    private bool _running;

    public AutoBackupService(AppStore store, SettingsService settings)
    {
        _store = store;
        _settings = settings;
    }

    public void Start()
    {
        if (_timer is not null) return;

        // 起動直後は画面の表示を優先し、1 分後に最初の確認をする
        _timer = new DispatcherTimer { Interval = TimeSpan.FromMinutes(1) };
        _timer.Tick += async (_, _) =>
        {
            _timer.Interval = TimeSpan.FromMinutes(30);
            await RunIfDueAsync();
        };
        _timer.Start();
    }

    public void Dispose() => _timer?.Stop();

    /// <summary>自動バックアップのファイルか（手動・取り込み前のものと分けて数えるため）。</summary>
    public static bool IsAutoBackup(string path) =>
        Path.GetFileName(path).EndsWith(FileSuffix, StringComparison.OrdinalIgnoreCase);

    /// <summary>最後に自動バックアップを作った日時（まだなければ null）。設定画面の表示用。</summary>
    public static DateTime? LastBackupTime => LoadState()?.CreatedAt;

    private async Task RunIfDueAsync()
    {
        if (_running) return;
        _running = true;
        try
        {
            var now = DateTime.Now;
            if (GetDueFingerprint(now) is not { } fingerprint) return;

            var databasePath = _store.Database.FilePath;
            var path = await Task.Run(() => CreateBackup(databasePath, fingerprint, now));
            AppLog.Info($"自動バックアップを作成しました: {path}");
        }
        catch (Exception ex)
        {
            AppLog.Warn("自動バックアップに失敗しました。", ex);
        }
        finally
        {
            _running = false;
        }
    }

    /// <summary>
    /// 今バックアップを作るべきなら、そのときの登録内容の指紋を返す（作らなくてよければ null）。
    /// DB を読むので UI スレッドから呼ぶ。
    /// </summary>
    internal string? GetDueFingerprint(DateTime now)
    {
        if (!_settings.Current.AutoBackup) return null;

        var state = LoadState();
        if (state is not null && state.CreatedAt.Date == now.Date) return null; // 今日は作成済み

        var fingerprint = ComputeFingerprint();
        return state is not null && state.Fingerprint == fingerprint ? null : fingerprint;
    }

    /// <summary>
    /// 登録内容の指紋。件数と最終更新日時から作るので、追加・変更・削除のどれでも変わる。
    /// 起動回数などの使用履歴は含めない（開いただけの日にはバックアップを作らない）。
    /// </summary>
    private string ComputeFingerprint()
    {
        using var cmd = _store.Database.CreateCommand(
            "SELECT (SELECT count(*) FROM folders) || '|' || ifnull((SELECT max(updated_at) FROM folders), '') " +
            "    || '|' || (SELECT count(*) FROM shortcuts) || '|' || ifnull((SELECT max(updated_at) FROM shortcuts), '') " +
            "    || '|' || (SELECT count(*) FROM tags) || '|' || (SELECT count(*) FROM shortcut_tags)");
        return Convert.ToString(cmd.ExecuteScalar(), CultureInfo.InvariantCulture) ?? string.Empty;
    }

    /// <summary>
    /// バックアップを作り、記録を残して古いものを整理する。戻り値は作ったファイル。
    /// 裏のスレッドから呼んでよい。
    /// </summary>
    internal static string CreateBackup(string databasePath, string fingerprint, DateTime now)
    {
        Directory.CreateDirectory(AppPaths.BackupDirectory);
        var path = Path.Combine(AppPaths.BackupDirectory,
            now.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture) + FileSuffix);
        var temp = path + ".tmp";

        // 画面が使っている接続とは別の接続でコピーする。WAL なので、画面側の書き込みと
        // 同時でも、ある時点の整合性のとれた内容がコピーされる。
        // 途中で終了しても壊れたバックアップが残らないよう、一時ファイルに作ってから名前を変える。
        if (File.Exists(temp)) File.Delete(temp);

        var builder = new SqliteConnectionStringBuilder
        {
            DataSource = databasePath,
            Mode = SqliteOpenMode.ReadWrite,
            Pooling = false,
        };
        using (var connection = new SqliteConnection(builder.ToString()))
        {
            connection.Open();
            using var cmd = connection.CreateCommand();
            cmd.CommandText = "VACUUM INTO $path";
            cmd.Parameters.AddWithValue("$path", temp);
            cmd.ExecuteNonQuery();
        }

        File.Move(temp, path, overwrite: true);
        SaveState(new BackupState(now, fingerprint));
        Prune();
        return path;
    }

    private static void Prune()
    {
        var old = Directory.EnumerateFiles(AppPaths.BackupDirectory, "*" + FileSuffix)
            .OrderByDescending(Path.GetFileName, StringComparer.Ordinal)
            .Skip(KeepCount)
            .ToList();

        foreach (var file in old)
        {
            try { File.Delete(file); }
            catch (Exception ex) { AppLog.Warn($"古い自動バックアップを削除できませんでした: {file}", ex); }
        }
    }

    // ---------------------------------------------------------------- 記録

    private sealed record BackupState(DateTime CreatedAt, string Fingerprint);

    private static string StatePath => Path.Combine(AppPaths.BackupDirectory, StateFileName);

    private static BackupState? LoadState()
    {
        try
        {
            return File.Exists(StatePath)
                ? JsonSerializer.Deserialize<BackupState>(File.ReadAllText(StatePath))
                : null;
        }
        catch (Exception ex)
        {
            AppLog.Warn("自動バックアップの記録を読めませんでした。", ex);
            return null;
        }
    }

    private static void SaveState(BackupState state) =>
        File.WriteAllText(StatePath, JsonSerializer.Serialize(state));
}
