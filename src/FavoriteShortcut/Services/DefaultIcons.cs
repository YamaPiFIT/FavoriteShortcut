using System.Windows;
using System.Windows.Media;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

/// <summary>
/// favicon が取得できないときや、種別が判定できないときに使う既定アイコン（§11）。
/// ベクター（Geometry）で描くので、どの表示サイズ・DPI でも滲まない。
/// </summary>
public static class DefaultIcons
{
    private static readonly Dictionary<TargetType, ImageSource> Cache = new();
    private static readonly object Gate = new();

    public static ImageSource For(TargetType type)
    {
        lock (Gate)
        {
            if (Cache.TryGetValue(type, out var cached)) return cached;
            var icon = Build(type);
            Cache[type] = icon;
            return icon;
        }
    }

    public static ImageSource Web => For(TargetType.Web);
    public static ImageSource Folder => For(TargetType.Folder);
    public static ImageSource File => For(TargetType.File);
    public static ImageSource Unknown => For(TargetType.Unknown);

    private static ImageSource Build(TargetType type) => type switch
    {
        TargetType.Web => Compose("#3B82F6", GlobeOutline, GlobeDetail, "#FFFFFF"),
        TargetType.Folder => Compose("#F59E0B", FolderShape, null, null),
        TargetType.Application => Compose("#8B5CF6", AppShape, null, null),
        TargetType.File => Compose("#64748B", DocumentShape, DocumentFold, "#FFFFFF"),
        _ => Compose("#94A3B8", QuestionShape, null, null),
    };

    /// <summary>24x24 のビューポート上に、背景色つきの形 + 任意の白い装飾を描く。</summary>
    private static ImageSource Compose(string color, string mainPath, string? detailPath, string? detailColor)
    {
        var group = new DrawingGroup();
        var brush = new SolidColorBrush((Color)ColorConverter.ConvertFromString(color));
        brush.Freeze();

        group.Children.Add(new GeometryDrawing(brush, null, Geometry.Parse(mainPath)));

        if (detailPath is not null && detailColor is not null)
        {
            var accent = new SolidColorBrush((Color)ColorConverter.ConvertFromString(detailColor));
            accent.Freeze();
            var pen = new Pen(accent, 1.4) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round };
            pen.Freeze();
            group.Children.Add(new GeometryDrawing(null, pen, Geometry.Parse(detailPath)));
        }

        // 24x24 の描画領域を固定して、種類ごとに見た目の大きさがぶれないようにする
        group.ClipGeometry = new RectangleGeometry(new Rect(0, 0, 24, 24));
        group.Freeze();

        var image = new DrawingImage(group);
        image.Freeze();
        return image;
    }

    private const string GlobeOutline = "M12,2 A10,10 0 1,0 12,22 A10,10 0 1,0 12,2 Z";
    private const string GlobeDetail =
        "M2.6,9.5 H21.4 M2.6,14.5 H21.4 M12,2.2 C8.6,5.4 8.6,18.6 12,21.8 M12,2.2 C15.4,5.4 15.4,18.6 12,21.8";

    private const string FolderShape =
        "M2.5,5.5 A1.5,1.5 0 0,1 4,4 H9.2 L11.4,6.4 H20 A1.5,1.5 0 0,1 21.5,7.9 V18.5 " +
        "A1.5,1.5 0 0,1 20,20 H4 A1.5,1.5 0 0,1 2.5,18.5 Z";

    private const string DocumentShape =
        "M5.5,3 A1.5,1.5 0 0,1 7,1.5 H14 L19.5,7 V21 A1.5,1.5 0 0,1 18,22.5 H7 A1.5,1.5 0 0,1 5.5,21 Z";
    private const string DocumentFold = "M8.6,12 H16.4 M8.6,15.4 H16.4 M8.6,18.8 H13.6";

    private const string AppShape =
        "M4,5.5 A2,2 0 0,1 6,3.5 H18 A2,2 0 0,1 20,5.5 V18.5 A2,2 0 0,1 18,20.5 H6 " +
        "A2,2 0 0,1 4,18.5 Z M7.6,8.2 H16.4 V10 H7.6 Z";

    private const string QuestionShape =
        "M12,2 A10,10 0 1,0 12,22 A10,10 0 1,0 12,2 Z M11,17.2 H13 V19.2 H11 Z " +
        "M12,5.4 C9.9,5.4 8.4,6.8 8.4,8.9 H10.4 C10.4,7.9 11,7.2 12,7.2 C13,7.2 13.6,7.8 13.6,8.7 " +
        "C13.6,10.2 11,10.4 11,13.2 V15.4 H13 V13.6 C13,11.6 15.6,11.2 15.6,8.6 C15.6,6.7 14.1,5.4 12,5.4 Z";
}
