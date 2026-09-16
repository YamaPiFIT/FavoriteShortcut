namespace FavoriteShortcut.Models;

public enum SpecialFolderKind
{
    All = 0,
    Uncategorized = 1,
}

/// <summary>
/// フォルダツリーの先頭に置く特別な項目（「すべて」「未分類」）。
/// 実フォルダではないので DB には存在しない。
/// </summary>
public sealed class SpecialFolderNode
{
    public SpecialFolderNode(SpecialFolderKind kind, string name, string glyph)
    {
        Kind = kind;
        Name = name;
        Glyph = glyph;
    }

    public SpecialFolderKind Kind { get; }
    public string Name { get; }

    /// <summary>ツリーに表示する記号。</summary>
    public string Glyph { get; }
}
