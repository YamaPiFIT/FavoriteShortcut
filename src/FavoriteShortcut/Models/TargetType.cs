namespace FavoriteShortcut.Models;

/// <summary>
/// ショートカットが指す対象の種類。
/// DB には整数で保存するため、既存の値は変更しないこと。
/// </summary>
public enum TargetType
{
    Unknown = 0,
    Web = 1,
    Folder = 2,
    File = 3,
    Application = 4,
}

public static class TargetTypeExtensions
{
    public static string ToDisplayName(this TargetType type) => type switch
    {
        TargetType.Web => "Web サイト",
        TargetType.Folder => "フォルダ",
        TargetType.File => "ファイル",
        TargetType.Application => "アプリケーション",
        _ => "不明",
    };
}
