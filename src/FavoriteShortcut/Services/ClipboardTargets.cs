using System.Text.RegularExpressions;
using System.Windows;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

/// <summary>
/// クリップボードから、ショートカットとして登録できそうなものを取り出す。
///
///   エクスプローラーでコピーしたファイル / フォルダ → そのパス（複数可）
///   文字列 → 1 行目が URL かパスならそれ（普通の文章は対象にしない）
/// </summary>
public static class ClipboardTargets
{
    private const int MaxLength = 2048;

    /// <summary>ドライブレター付きのパス、または UNC パス。</summary>
    private static readonly Regex PathLike = new(@"^([a-zA-Z]:[\\/]|\\\\)", RegexOptions.Compiled);

    public static IReadOnlyList<string> Read()
    {
        try
        {
            if (Clipboard.ContainsFileDropList())
            {
                return Clipboard.GetFileDropList().Cast<string>()
                    .Where(path => !string.IsNullOrWhiteSpace(path))
                    .ToList();
            }

            if (Clipboard.ContainsText() && FromText(Clipboard.GetText()) is { } target)
                return new[] { target };
        }
        catch (Exception ex)
        {
            // 他のアプリがクリップボードを使用中だと読めないことがある
            AppLog.Warn("クリップボードを読めませんでした。", ex);
        }

        return Array.Empty<string>();
    }

    /// <summary>文字列の 1 行目が URL かパスなら、それを返す。</summary>
    public static string? FromText(string? text)
    {
        var line = (text ?? string.Empty).Replace("\r", string.Empty).Split('\n')[0].Trim();

        // エクスプローラーの「パスのコピー」は前後に引用符が付く
        if (line.Length >= 2 && line[0] == '"' && line[^1] == '"') line = line[1..^1].Trim();
        if (line.Length is 0 or > MaxLength) return null;

        // パス（C:\... / \\server\...）は空白を含んでよい
        if (PathLike.IsMatch(line)) return line;

        // それ以外は空白を含まないものだけ（普通の文章を URL と取り違えないように）
        if (line.Any(char.IsWhiteSpace)) return null;

        var type = TargetResolver.Detect(line);
        if (type == TargetType.Unknown) return null;

        // "memo:abc" のように「〇〇:」で始まるだけの文字列は URL とみなさない
        if (type == TargetType.Web && line.Contains(':') && !line.Contains("://")) return null;

        return line;
    }
}
