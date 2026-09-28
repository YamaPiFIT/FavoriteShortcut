using System.ComponentModel;
using FavoriteShortcut.Services;

namespace FavoriteShortcut.Models;

public enum SpecialFolderKind
{
    All = 0,
    Uncategorized = 1,
}

/// <summary>
/// フォルダツリーの先頭に置く特別な項目（「すべて」「未分類」）。
/// 実フォルダではないので DB には存在しない。
///
/// 表示名は対訳表のキーで持ち、言語を切り替えたら <see cref="Refresh"/> で表示を更新する。
/// </summary>
public sealed class SpecialFolderNode : INotifyPropertyChanged
{
    private readonly string _nameKey;

    public SpecialFolderNode(SpecialFolderKind kind, string nameKey, string glyph)
    {
        Kind = kind;
        _nameKey = nameKey;
        Glyph = glyph;
    }

    public SpecialFolderKind Kind { get; }

    public string Name => Loc.T(_nameKey);

    /// <summary>ツリーに表示する記号。</summary>
    public string Glyph { get; }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>言語を切り替えたあとに、表示名を更新させる。</summary>
    public void Refresh() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(Name)));
}
