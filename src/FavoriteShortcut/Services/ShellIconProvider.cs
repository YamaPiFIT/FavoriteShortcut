using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace FavoriteShortcut.Services;

/// <summary>
/// Windows シェルからファイル/フォルダのアイコンを取得する（§11, §16）。
///
/// 高DPIでも粗くならないよう、まず 48px の Extra Large アイコン一覧から取得を試み、
/// 取れない環境では SHGetFileInfo の 32px アイコンにフォールバックする。
/// 同じ拡張子のアイコンは使い回すのでファイル数が増えても負荷は上がらない。
/// </summary>
internal static class ShellIconProvider
{
    private static readonly Dictionary<string, ImageSource?> Cache = new(StringComparer.OrdinalIgnoreCase);
    private static readonly object Gate = new();

    /// <summary>フォルダの標準アイコン。</summary>
    public static ImageSource? GetFolderIcon(string? path)
    {
        var usable = !string.IsNullOrWhiteSpace(path) && Directory.Exists(path);
        // 実在するフォルダは固有アイコン（デスクトップ等）を持つことがあるのでパスをキーにする
        var key = usable ? "dir:" + path!.ToLowerInvariant() : "dir:*";
        return GetCached(key, () => usable
            ? Load(path!, 0, useFileAttributes: false)
            : Load("C:\\", FILE_ATTRIBUTE_DIRECTORY, useFileAttributes: true));
    }

    /// <summary>ファイルのアイコン。EXE/LNK/ICO は実体固有のアイコンを取りに行く。</summary>
    public static ImageSource? GetFileIcon(string? path)
    {
        if (string.IsNullOrWhiteSpace(path)) return null;

        var ext = Path.GetExtension(path);
        var exists = File.Exists(path);
        var perFile = exists && ext is not null &&
                      (ext.Equals(".exe", StringComparison.OrdinalIgnoreCase) ||
                       ext.Equals(".lnk", StringComparison.OrdinalIgnoreCase) ||
                       ext.Equals(".ico", StringComparison.OrdinalIgnoreCase) ||
                       ext.Equals(".msi", StringComparison.OrdinalIgnoreCase) ||
                       ext.Equals(".url", StringComparison.OrdinalIgnoreCase));

        if (perFile)
            return GetCached("file:" + path.ToLowerInvariant(), () => Load(path, 0, useFileAttributes: false));

        var key = "ext:" + (string.IsNullOrEmpty(ext) ? "<none>" : ext.ToLowerInvariant());
        return GetCached(key, () => exists
            ? Load(path, 0, useFileAttributes: false)
            : Load("dummy" + ext, FILE_ATTRIBUTE_NORMAL, useFileAttributes: true));
    }

    public static void ClearCache()
    {
        lock (Gate) Cache.Clear();
    }

    private static ImageSource? GetCached(string key, Func<ImageSource?> factory)
    {
        lock (Gate)
        {
            if (Cache.TryGetValue(key, out var cached)) return cached;
        }

        ImageSource? icon = null;
        try { icon = factory(); }
        catch (Exception ex) { AppLog.Warn($"シェルアイコンの取得に失敗: {key}", ex); }

        lock (Gate) Cache[key] = icon;
        return icon;
    }

    private static ImageSource? Load(string path, uint attributes, bool useFileAttributes)
    {
        var flags = SHGFI_SYSICONINDEX;
        if (useFileAttributes) flags |= SHGFI_USEFILEATTRIBUTES;

        var info = new SHFILEINFO();
        var result = SHGetFileInfo(path, attributes, ref info, (uint)Marshal.SizeOf<SHFILEINFO>(), flags);

        if (result != IntPtr.Zero)
        {
            var fromList = FromImageList(info.iIcon, SHIL_EXTRALARGE) ?? FromImageList(info.iIcon, SHIL_LARGE);
            if (fromList is not null) return fromList;
        }

        // Extra Large が使えない環境向けのフォールバック
        var flags2 = SHGFI_ICON | SHGFI_LARGEICON;
        if (useFileAttributes) flags2 |= SHGFI_USEFILEATTRIBUTES;

        var info2 = new SHFILEINFO();
        if (SHGetFileInfo(path, attributes, ref info2, (uint)Marshal.SizeOf<SHFILEINFO>(), flags2) == IntPtr.Zero)
            return null;

        try
        {
            return ToImageSource(info2.hIcon);
        }
        finally
        {
            if (info2.hIcon != IntPtr.Zero) DestroyIcon(info2.hIcon);
        }
    }

    private static ImageSource? FromImageList(int iconIndex, int size)
    {
        if (SHGetImageList(size, ref IID_IImageList, out var list) != 0 || list is null) return null;

        var hIcon = IntPtr.Zero;
        try
        {
            if (list.GetIcon(iconIndex, ILD_TRANSPARENT, ref hIcon) != 0 || hIcon == IntPtr.Zero) return null;
            return ToImageSource(hIcon);
        }
        catch
        {
            return null;
        }
        finally
        {
            if (hIcon != IntPtr.Zero) DestroyIcon(hIcon);
            Marshal.ReleaseComObject(list);
        }
    }

    private static ImageSource? ToImageSource(IntPtr hIcon)
    {
        if (hIcon == IntPtr.Zero) return null;
        var source = Imaging.CreateBitmapSourceFromHIcon(
            hIcon, Int32Rect.Empty, BitmapSizeOptions.FromEmptyOptions());
        source.Freeze();
        return source;
    }

    // ------------------------------------------------------------- interop

    private const uint SHGFI_ICON = 0x000000100;
    private const uint SHGFI_LARGEICON = 0x000000000;
    private const uint SHGFI_USEFILEATTRIBUTES = 0x000000010;
    private const uint SHGFI_SYSICONINDEX = 0x000004000;

    private const uint FILE_ATTRIBUTE_DIRECTORY = 0x00000010;
    private const uint FILE_ATTRIBUTE_NORMAL = 0x00000080;

    private const int SHIL_LARGE = 0;
    private const int SHIL_EXTRALARGE = 2;
    private const int ILD_TRANSPARENT = 1;

    private static Guid IID_IImageList = new("46EB5926-582E-4017-9FDF-E8998DAA0950");

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct SHFILEINFO
    {
        public IntPtr hIcon;
        public int iIcon;
        public uint dwAttributes;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string szDisplayName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 80)] public string szTypeName;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr SHGetFileInfo(
        string pszPath, uint dwFileAttributes, ref SHFILEINFO psfi, uint cbFileInfo, uint uFlags);

    [DllImport("shell32.dll", EntryPoint = "#727")]
    private static extern int SHGetImageList(int iImageList, ref Guid riid,
        [MarshalAs(UnmanagedType.Interface)] out IImageList ppv);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyIcon(IntPtr hIcon);

    [ComImport]
    [Guid("46EB5926-582E-4017-9FDF-E8998DAA0950")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IImageList
    {
        [PreserveSig] int Add(IntPtr hbmImage, IntPtr hbmMask, ref int pi);
        [PreserveSig] int ReplaceIcon(int i, IntPtr hicon, ref int pi);
        [PreserveSig] int SetOverlayImage(int iImage, int iOverlay);
        [PreserveSig] int Replace(int i, IntPtr hbmImage, IntPtr hbmMask);
        [PreserveSig] int AddMasked(IntPtr hbmImage, int crMask, ref int pi);
        [PreserveSig] int Draw(IntPtr pimldp);
        [PreserveSig] int Remove(int i);
        [PreserveSig] int GetIcon(int i, int flags, ref IntPtr picon);
    }
}
