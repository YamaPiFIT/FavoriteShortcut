using System.IO;
using System.Text.Json;

namespace FavoriteShortcut.Data;

/// <summary>
/// アプリが使用するフォルダ/ファイルの場所を一元管理する。
///
/// 既定は「EXE と同じ場所の Data フォルダ」。
/// EXE ごとフォルダをコピーすれば登録内容も一緒に移動するので、
/// USB メモリや別PCへの持ち運びがそのままできる。
///
/// 解決の優先順位:
///   1. 環境変数 FAVORITESHORTCUT_DATA_DIR
///   2. EXE と同じ場所の location.json（設定画面で変更したとき）
///   3. %APPDATA% の location.json（EXE 側に書けない環境で変更したとき）
///   4. EXE と同じ場所の Data フォルダ  ← 既定
///   5. 4 に書き込めない場合のみ %APPDATA%\お気に入りショートカット\
///
/// 5 は Program Files 配下など、書き込みが許可されていない場所に
/// インストールされた場合の退避先。
/// </summary>
public static class AppPaths
{
    public const string AppFolderName = "お気に入りショートカット";

    /// <summary>
    /// データ保存場所を上書きする環境変数。
    /// 検証用や、1台のPCで複数のデータを使い分けたいときに使う。
    /// </summary>
    public const string DataDirectoryEnvironmentVariable = "FAVORITESHORTCUT_DATA_DIR";

    private const string LocationConfigFileName = "location.json";

    private static string? _dataDirectory;

    public static string ExeDirectory { get; } =
        Path.GetDirectoryName(Environment.ProcessPath ?? AppContext.BaseDirectory) ?? AppContext.BaseDirectory;

    /// <summary>既定のデータ保存場所（EXE と同じ場所の Data）。</summary>
    public static string DefaultDataDirectory { get; } = Path.Combine(ExeDirectory, "Data");

    /// <summary>EXE の場所に書き込めない場合の退避先。</summary>
    public static string FallbackDataDirectory { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), AppFolderName);

    private static string ExeLocationConfigFile => Path.Combine(ExeDirectory, LocationConfigFileName);
    private static string FallbackLocationConfigFile => Path.Combine(FallbackDataDirectory, LocationConfigFileName);

    /// <summary>
    /// EXE の場所に書き込めず、%APPDATA% へ退避した場合 true。
    /// 設定画面で理由を説明するために使う。
    /// </summary>
    public static bool UsingFallbackLocation { get; private set; }

    /// <summary>DB・アイコン・ログを格納する実際のフォルダ。</summary>
    public static string DataDirectory
    {
        get
        {
            if (_dataDirectory is null)
            {
                _dataDirectory = ResolveDataDirectory();
                Directory.CreateDirectory(_dataDirectory);
                Directory.CreateDirectory(Path.Combine(_dataDirectory, "icons"));
                Directory.CreateDirectory(Path.Combine(_dataDirectory, "backup"));
            }
            return _dataDirectory;
        }
    }

    public static string DatabaseFile => Path.Combine(DataDirectory, "favorite-shortcuts.sqlite");
    public static string IconDirectory => Path.Combine(DataDirectory, "icons");
    public static string BackupDirectory => Path.Combine(DataDirectory, "backup");
    public static string LogFile => Path.Combine(DataDirectory, "app.log");

    private static string ResolveDataDirectory()
    {
        var fromEnvironment = Environment.GetEnvironmentVariable(DataDirectoryEnvironmentVariable);
        if (!string.IsNullOrWhiteSpace(fromEnvironment))
            return Path.GetFullPath(fromEnvironment);

        // 設定画面で保存場所を変更した場合。EXE 隣の設定を優先するので、
        // フォルダごとコピーすれば変更内容も一緒に移動する。
        foreach (var configFile in new[] { ExeLocationConfigFile, FallbackLocationConfigFile })
        {
            if (TryReadLocationConfig(configFile) is { } configured) return configured;
        }

        if (CanWriteTo(DefaultDataDirectory)) return DefaultDataDirectory;

        UsingFallbackLocation = true;
        return FallbackDataDirectory;
    }

    private static string? TryReadLocationConfig(string configFile)
    {
        try
        {
            if (!File.Exists(configFile)) return null;

            var cfg = JsonSerializer.Deserialize<LocationConfig>(File.ReadAllText(configFile));
            var dir = cfg?.DataDirectory;
            return string.IsNullOrWhiteSpace(dir) ? null : Path.GetFullPath(dir);
        }
        catch
        {
            // 設定ファイルが壊れていても既定の場所で起動できるようにする
            return null;
        }
    }

    /// <summary>実際にファイルを作って、その場所に書き込めるかどうかを確かめる。</summary>
    private static bool CanWriteTo(string directory)
    {
        try
        {
            Directory.CreateDirectory(directory);
            var probe = Path.Combine(directory, ".write-test-" + Guid.NewGuid().ToString("N"));
            File.WriteAllText(probe, string.Empty);
            File.Delete(probe);
            return true;
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// データ保存場所を変更する（次回起動から有効）。
    /// null を渡すと既定（EXE 隣の Data）に戻る。
    /// </summary>
    public static void SaveDataDirectoryOverride(string? directory)
    {
        // EXE の隣に書ければそちらへ。書けない場合のみ %APPDATA% 側に記録する。
        var target = CanWriteTo(ExeDirectory) ? ExeLocationConfigFile : FallbackLocationConfigFile;
        if (target == FallbackLocationConfigFile) Directory.CreateDirectory(FallbackDataDirectory);

        if (string.IsNullOrWhiteSpace(directory))
        {
            DeleteIfExists(ExeLocationConfigFile);
            DeleteIfExists(FallbackLocationConfigFile);
            return;
        }

        var json = JsonSerializer.Serialize(new LocationConfig { DataDirectory = directory },
            new JsonSerializerOptions { WriteIndented = true });
        File.WriteAllText(target, json);

        // 片方だけが残って解決順序で食い違わないようにする
        if (target == ExeLocationConfigFile) DeleteIfExists(FallbackLocationConfigFile);
    }

    private static void DeleteIfExists(string path)
    {
        try { if (File.Exists(path)) File.Delete(path); }
        catch { /* 消せなくても致命的ではない */ }
    }

    private sealed class LocationConfig
    {
        public string? DataDirectory { get; set; }
    }
}
