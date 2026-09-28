using FavoriteShortcut.Services;
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
        TargetType.Web => Loc.T("Str.Type.Web"),
        TargetType.Folder => Loc.T("Str.Type.Folder"),
        TargetType.File => Loc.T("Str.Type.File"),
        TargetType.Application => Loc.T("Str.Type.Application"),
        _ => Loc.T("Str.Type.Unknown"),
    };
}
