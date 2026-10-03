using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows.Media;

namespace FavoriteShortcut.Models;

/// <summary>1 件のショートカット。</summary>
public sealed class ShortcutItem : INotifyPropertyChanged
{
    private string _title = string.Empty;
    private string _target = string.Empty;
    private TargetType _targetType = TargetType.Unknown;
    private string? _folderId;
    private string? _iconPath;
    private string? _note;
    private int _sortOrder;
    private List<string> _tags = new();
    private ImageSource? _icon;
    private int _usageCount;
    private DateTime? _lastUsedAt;

    public string Id { get; set; } = Guid.NewGuid().ToString("N");

    public string Title
    {
        get => _title;
        set { if (Set(ref _title, value)) Raise(nameof(DisplayTitle)); }
    }

    /// <summary>URL またはローカル/UNC パス。</summary>
    public string Target
    {
        get => _target;
        set { if (Set(ref _target, value)) Raise(nameof(SubText)); }
    }

    public TargetType TargetType
    {
        get => _targetType;
        set => Set(ref _targetType, value);
    }

    public string? FolderId
    {
        get => _folderId;
        set => Set(ref _folderId, value);
    }

    /// <summary>アイコンフォルダ内の相対ファイル名。null の場合は対象種別から自動解決する。</summary>
    public string? IconPath
    {
        get => _iconPath;
        set => Set(ref _iconPath, value);
    }

    public string? Note
    {
        get => _note;
        set => Set(ref _note, value);
    }

    public int SortOrder
    {
        get => _sortOrder;
        set => Set(ref _sortOrder, value);
    }

    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
    public DateTime UpdatedAt { get; set; } = DateTime.UtcNow;

    /// <summary>タグ名のリスト（表示順）。</summary>
    public List<string> Tags
    {
        get => _tags;
        set
        {
            _tags = value;
            Raise(nameof(Tags));
            Raise(nameof(TagsText));
            Raise(nameof(HasTags));
        }
    }

    public int UsageCount
    {
        get => _usageCount;
        set => Set(ref _usageCount, value);
    }

    public DateTime? LastUsedAt
    {
        get => _lastUsedAt;
        set => Set(ref _lastUsedAt, value);
    }

    // ---- 表示用のみ（DB には保存しない） ----

    public string DisplayTitle => string.IsNullOrWhiteSpace(_title) ? _target : _title;

    public string TagsText => _tags.Count == 0 ? string.Empty : string.Join("  ", _tags);

    public bool HasTags => _tags.Count > 0;

    public string SubText => _target;

    /// <summary>遅延解決されたアイコン。IconService が設定する。</summary>
    public ImageSource? Icon
    {
        get => _icon;
        set => Set(ref _icon, value);
    }

    /// <summary>フォルダのフルパス表示（ストアが更新）。</summary>
    public string FolderPathText
    {
        get => _folderPathText;
        internal set => Set(ref _folderPathText, value);
    }
    private string _folderPathText = string.Empty;

    /// <summary>検索用に正規化済みのキャッシュ。SearchService が管理する。</summary>
    internal SearchIndexEntry? SearchIndex { get; set; }

    /// <summary>URL を解析した結果のキャッシュ（favicon 用）。IconService が管理する。</summary>
    internal WebTargetInfo? WebTargetInfo { get; set; }

    public ShortcutItem Clone() => new()
    {
        Id = Id,
        Title = Title,
        Target = Target,
        TargetType = TargetType,
        FolderId = FolderId,
        IconPath = IconPath,
        Note = Note,
        SortOrder = SortOrder,
        CreatedAt = CreatedAt,
        UpdatedAt = UpdatedAt,
        Tags = new List<string>(Tags),
        UsageCount = UsageCount,
        LastUsedAt = LastUsedAt,
    };

    public event PropertyChangedEventHandler? PropertyChanged;

    private bool Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value)) return false;
        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
        return true;
    }

    private void Raise(string name) => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}

/// <summary>検索用に正規化した文字列のキャッシュ。</summary>
internal sealed class SearchIndexEntry
{
    public string Title = string.Empty;
    public string Target = string.Empty;
    public string Host = string.Empty;
    public string FolderPath = string.Empty;
    public string[] FolderSegments = Array.Empty<string>();
    public string Note = string.Empty;
    public string[] Tags = Array.Empty<string>();
    public int Revision = -1;

    // 索引を作ったときの元の値。文字列は書き換えられないので、
    // 参照が同じなら内容も同じ＝作り直す必要がない（比較はポインタ比較だけで済む）。
    public string? SourceTitle;
    public string? SourceTarget;
    public string? SourceFolderPath;
    public string? SourceNote;
    public List<string>? SourceTags;

    public bool IsBuiltFrom(ShortcutItem item) =>
        ReferenceEquals(SourceTitle, item.Title) &&
        ReferenceEquals(SourceTarget, item.Target) &&
        ReferenceEquals(SourceFolderPath, item.FolderPathText) &&
        ReferenceEquals(SourceNote, item.Note) &&
        ReferenceEquals(SourceTags, item.Tags);
}

/// <summary>Web の URL を解析した結果のキャッシュ（一覧を表示するたびに解析し直さないためのもの）。</summary>
internal sealed class WebTargetInfo
{
    /// <summary>解析した URL。項目の URL がこれと別の文字列になったら作り直す。</summary>
    public string? Target;
    public Uri? PageUri;
    public string? Origin;
    public string? FaviconFileBase;
}
