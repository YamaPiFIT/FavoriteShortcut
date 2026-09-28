using System.Globalization;
using System.Windows;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

/// <summary>
/// 表示言語の切り替えと、画面に出す文字列の取得（日本語 / 英語）。
///
/// 文字列の本体は <see cref="Strings"/> の対訳表にある。
/// 言語を適用すると、その表から WPF のリソース辞書を作ってアプリに差し込むので、
/// XAML 側は {DynamicResource Str.xxx} と書くだけで、再起動なしに表示が切り替わる。
/// コードからは <see cref="T(string)"/> で取得する。
///
/// WPF のリソースを経由せずに対訳表を直接引くので、アプリ本体の外（テストやデータ層）や
/// 別スレッドから呼んでも安全に動く。
/// </summary>
public static class Loc
{
    /// <summary>
    /// 起動時点の Windows の表示言語。
    /// 「自動」はこれを基準に判定する（アプリ側でカルチャを書き換えても影響を受けない）。
    /// </summary>
    private static readonly CultureInfo SystemCulture = CultureInfo.CurrentUICulture;

    private static volatile IReadOnlyDictionary<string, string> _current = Strings.Japanese;
    private static ResourceDictionary? _resourceSlot;
    private static readonly HashSet<string> WarnedKeys = new(StringComparer.Ordinal);

    /// <summary>実際に表示している言語（「自動」を解決したあとの値）。</summary>
    public static AppLanguage Current { get; private set; } = AppLanguage.Japanese;

    /// <summary>言語が切り替わったときに発生する。コードで組み立てた表示を作り直すために使う。</summary>
    public static event EventHandler? LanguageChanged;

    /// <summary>キーに対応する文字列。見つからない場合はキーをそのまま返す（表示が消えないように）。</summary>
    public static string T(string key)
    {
        if (_current.TryGetValue(key, out var value)) return value;

        lock (WarnedKeys)
        {
            // 同じキーで何度もログを書かない
            if (WarnedKeys.Add(key)) AppLog.Warn($"文字列リソースが見つかりません: {key}");
        }
        return key;
    }

    /// <summary>{0} などのプレースホルダを埋めた文字列。</summary>
    public static string T(string key, params object?[] args)
    {
        var format = T(key);
        try
        {
            return string.Format(CultureInfo.CurrentCulture, format, args);
        }
        catch (FormatException ex)
        {
            AppLog.Warn($"文字列リソースの書式が不正です: {key}", ex);
            return format;
        }
    }

    /// <summary>「自動」を、実際に使用する言語へ解決する。</summary>
    public static AppLanguage Resolve(AppLanguage language) => ResolveFor(language, SystemCulture);

    /// <summary>
    /// 指定したカルチャを Windows の表示言語とみなして解決する。
    /// 日本語環境なら日本語、それ以外はすべて英語にする。
    /// </summary>
    public static AppLanguage ResolveFor(AppLanguage language, CultureInfo culture)
    {
        if (language != AppLanguage.Auto) return language;

        return culture.TwoLetterISOLanguageName.Equals("ja", StringComparison.OrdinalIgnoreCase)
            ? AppLanguage.Japanese
            : AppLanguage.English;
    }

    /// <summary>
    /// 言語を適用する。WPF アプリとして動いている場合は画面の文字列も即座に入れ替える。
    /// UI スレッドから呼ぶこと。
    /// </summary>
    public static void Apply(AppLanguage setting)
    {
        var resolved = Resolve(setting);
        var changed = resolved != Current || _resourceSlot is null;

        _current = resolved == AppLanguage.English ? Strings.English : Strings.Japanese;
        Current = resolved;

        if (Application.Current is { } app)
        {
            var dictionary = new ResourceDictionary();
            foreach (var (key, value) in _current) dictionary[key] = value;

            // 前回差し込んだ辞書だけを入れ替える（テーマの辞書などには触れない）
            var merged = app.Resources.MergedDictionaries;
            if (_resourceSlot is not null) merged.Remove(_resourceSlot);
            merged.Add(dictionary);
            _resourceSlot = dictionary;
        }

        if (changed) LanguageChanged?.Invoke(null, EventArgs.Empty);
    }

    /// <summary>設定画面の補足表示用: 「自動」で何語になるかの説明。</summary>
    public static string DescribeAutoDetection()
    {
        var resolved = Resolve(AppLanguage.Auto);
        return T("Str.Settings.LanguageAutoNote",
            SystemCulture.NativeName,
            T(resolved == AppLanguage.Japanese
                ? "Str.Settings.LanguageNameJapanese"
                : "Str.Settings.LanguageNameEnglish"));
    }
}
