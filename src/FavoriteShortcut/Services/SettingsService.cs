using System.Globalization;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

/// <summary>
/// 設定の読み書き。DB の settings テーブル（key/value）に保存するので、
/// エクスポート/インポートで設定も一緒に持ち運べる（§25）。
/// 未知のキーや壊れた値があっても既定値で起動できるようにしている。
/// </summary>
public sealed class SettingsService
{
    private readonly AppStore _store;

    public SettingsService(AppStore store)
    {
        _store = store;
        Current = Load();
    }

    public AppSettings Current { get; private set; }

    public event EventHandler<AppSettings>? Changed;

    private AppSettings Load()
    {
        var raw = _store.LoadSettingsRaw();
        var s = new AppSettings();

        s.ViewMode = GetEnum(raw, "view_mode", s.ViewMode);
        s.IconSize = Math.Clamp(GetInt(raw, "icon_size", s.IconSize), 16, 96);
        s.Theme = GetEnum(raw, "theme", s.Theme);
        s.HotKeyModifiers = GetInt(raw, "hotkey_modifiers", s.HotKeyModifiers);
        s.HotKeyKey = GetInt(raw, "hotkey_key", s.HotKeyKey);
        s.MinimizeToTray = GetBool(raw, "minimize_to_tray", s.MinimizeToTray);
        s.StartMinimized = GetBool(raw, "start_minimized", s.StartMinimized);
        s.RunAtStartup = GetBool(raw, "run_at_startup", s.RunAtStartup);
        s.UseFaviconFallbackService = GetBool(raw, "favicon_fallback", s.UseFaviconFallbackService);
        s.ShowRecentInLauncher = GetBool(raw, "launcher_show_recent", s.ShowRecentInLauncher);
        s.LauncherMaxResults = Math.Clamp(GetInt(raw, "launcher_max_results", s.LauncherMaxResults), 3, 50);
        s.RestoreLastFolder = GetBool(raw, "restore_last_folder", s.RestoreLastFolder);
        s.LastFolderId = raw.GetValueOrDefault("last_folder_id") is { Length: > 0 } id ? id : null;
        s.MainWindowWidth = GetDouble(raw, "window_width", s.MainWindowWidth);
        s.MainWindowHeight = GetDouble(raw, "window_height", s.MainWindowHeight);
        s.FolderPaneWidth = GetDouble(raw, "folder_pane_width", s.FolderPaneWidth);

        return s;
    }

    public void Save(AppSettings settings)
    {
        var raw = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["view_mode"] = ((int)settings.ViewMode).ToString(CultureInfo.InvariantCulture),
            ["icon_size"] = settings.IconSize.ToString(CultureInfo.InvariantCulture),
            ["theme"] = ((int)settings.Theme).ToString(CultureInfo.InvariantCulture),
            ["hotkey_modifiers"] = settings.HotKeyModifiers.ToString(CultureInfo.InvariantCulture),
            ["hotkey_key"] = settings.HotKeyKey.ToString(CultureInfo.InvariantCulture),
            ["minimize_to_tray"] = settings.MinimizeToTray ? "1" : "0",
            ["start_minimized"] = settings.StartMinimized ? "1" : "0",
            ["run_at_startup"] = settings.RunAtStartup ? "1" : "0",
            ["favicon_fallback"] = settings.UseFaviconFallbackService ? "1" : "0",
            ["launcher_show_recent"] = settings.ShowRecentInLauncher ? "1" : "0",
            ["launcher_max_results"] = settings.LauncherMaxResults.ToString(CultureInfo.InvariantCulture),
            ["restore_last_folder"] = settings.RestoreLastFolder ? "1" : "0",
            ["last_folder_id"] = settings.LastFolderId ?? string.Empty,
            ["window_width"] = settings.MainWindowWidth.ToString(CultureInfo.InvariantCulture),
            ["window_height"] = settings.MainWindowHeight.ToString(CultureInfo.InvariantCulture),
            ["folder_pane_width"] = settings.FolderPaneWidth.ToString(CultureInfo.InvariantCulture),
        };

        _store.SaveSettingsRaw(raw);
        Current = settings;
        Changed?.Invoke(this, settings);
    }

    /// <summary>インポート後など、DB の内容から読み直す。</summary>
    public void Reload()
    {
        Current = Load();
        Changed?.Invoke(this, Current);
    }

    private static int GetInt(IReadOnlyDictionary<string, string> raw, string key, int fallback) =>
        raw.TryGetValue(key, out var v) && int.TryParse(v, NumberStyles.Integer, CultureInfo.InvariantCulture, out var i)
            ? i : fallback;

    private static double GetDouble(IReadOnlyDictionary<string, string> raw, string key, double fallback) =>
        raw.TryGetValue(key, out var v) && double.TryParse(v, NumberStyles.Float, CultureInfo.InvariantCulture, out var d)
            && d > 0 ? d : fallback;

    private static bool GetBool(IReadOnlyDictionary<string, string> raw, string key, bool fallback) =>
        raw.TryGetValue(key, out var v) ? v == "1" || string.Equals(v, "true", StringComparison.OrdinalIgnoreCase) : fallback;

    private static TEnum GetEnum<TEnum>(IReadOnlyDictionary<string, string> raw, string key, TEnum fallback)
        where TEnum : struct, Enum
    {
        if (!raw.TryGetValue(key, out var v)) return fallback;
        if (int.TryParse(v, out var i) && Enum.IsDefined(typeof(TEnum), i)) return (TEnum)(object)i;
        return Enum.TryParse<TEnum>(v, true, out var parsed) ? parsed : fallback;
    }
}
