using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;

namespace FavoriteShortcut.Models;

/// <summary>フォルダ。parent_id によって階層構造を表す。</summary>
public sealed class FolderItem : INotifyPropertyChanged
{
    private string _name = string.Empty;
    private string? _parentId;
    private int _sortOrder;

    public string Id { get; set; } = Guid.NewGuid().ToString("N");

    public string Name
    {
        get => _name;
        set => Set(ref _name, value);
    }

    public string? ParentId
    {
        get => _parentId;
        set => Set(ref _parentId, value);
    }

    public int SortOrder
    {
        get => _sortOrder;
        set => Set(ref _sortOrder, value);
    }

    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
    public DateTime UpdatedAt { get; set; } = DateTime.UtcNow;

    /// <summary>「仕事 &gt; 開発」のような表示用フルパス。ストア側で再計算される。</summary>
    public string FullPath
    {
        get => _fullPath;
        internal set => Set(ref _fullPath, value);
    }
    private string _fullPath = string.Empty;

    /// <summary>ツリー表示用の子フォルダ。ストア側で構築される。</summary>
    public ObservableCollection<FolderItem> Children { get; } = new();

    public event PropertyChangedEventHandler? PropertyChanged;

    private void Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value)) return;
        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
    }

    internal void RaiseChanged(string name) => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
