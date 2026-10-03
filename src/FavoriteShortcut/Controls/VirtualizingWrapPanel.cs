using System.Collections.Specialized;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;

namespace FavoriteShortcut.Controls;

/// <summary>
/// 同じ大きさの項目を左から右へ並べて折り返す、仮想化対応のパネル（カード表示用）。
///
/// WrapPanel は表示する全件分のカードを作って配置するため、件数が多いと
/// 一覧の更新や検索のたびに 1 秒前後かかっていた。このパネルは画面に見えている行
/// （と、キー操作で隣の行へ移れるよう前後 1 行）のカードだけを作る。
///
/// 並び方、スクロール量（1 行 16px・ホイール 1 回 48px・PageUp/PageDown は 1 画面分）、
/// 選んだカードが見える位置までの最小限のスクロールは、WrapPanel を標準のスクロールで
/// 動かしていたときと同じになるようにしている。
/// </summary>
public sealed class VirtualizingWrapPanel : VirtualizingPanel, IScrollInfo
{
    // 標準のスクロール（ScrollContentPresenter）と同じ移動量
    private const double LineDelta = 16.0;
    private const double WheelDelta = 48.0;

    /// <summary>画面の上下に余分に作っておく行数。</summary>
    private const int BufferRows = 1;

    /// <summary>1 項目の大きさ（実際に作ったカードを測って決める。全カードが同じ大きさの前提）。</summary>
    private Size _itemSize = Size.Empty;
    private int _perRow = 1;

    private Size _extent;
    private Size _viewport;

    /// <summary>
    /// 指定されたスクロール位置。件数が減って表示範囲に収めたあとも覚えておき、
    /// 件数が戻ればこの位置に戻る（標準のスクロールと同じ）。
    /// </summary>
    private Point _offset;

    /// <summary>表示範囲に収めた、実際のスクロール位置。</summary>
    private Point _computedOffset;
    private Point _reportedOffset;

    // ------------------------------------------------------------ レイアウト

    protected override Size MeasureOverride(Size availableSize)
    {
        // 先に InternalChildren に触れておくと、ItemContainerGenerator が確実に用意される
        var children = InternalChildren;
        var generator = ItemContainerGenerator;
        var count = ItemsControl.GetItemsOwner(this)?.Items.Count ?? 0;
        var constraint = new Size(availableSize.Width, double.PositiveInfinity);

        if (count == 0)
        {
            CleanUp(generator, 0, -1);
            UpdateScrollInfo(availableSize, new Size(0, 0));
            return DesiredSizeFor(availableSize);
        }

        if (_itemSize.IsEmpty)
            _itemSize = RealizeRange(generator, children, 0, 0, constraint)?.DesiredSize ?? Size.Empty;

        for (var attempt = 0; attempt < 2; attempt++)
        {
            var rows = ComputeRows(availableSize.Width, count);
            UpdateScrollInfo(availableSize,
                new Size(Math.Min(count, _perRow) * ItemWidth, rows * ItemHeight));

            var (first, last) = RealizedRange(availableSize, count, rows);
            var firstChild = RealizeRange(generator, children, first, last, constraint);
            CleanUp(generator, first, last);

            // カードの大きさが前回と違えば（フォントの変更など）並べ直す
            var actual = firstChild?.DesiredSize ?? _itemSize;
            if (actual == _itemSize || actual.Width <= 0 || actual.Height <= 0) break;
            _itemSize = actual;
        }

        return DesiredSizeFor(availableSize);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        var children = InternalChildren;
        var generator = ItemContainerGenerator;

        for (var i = 0; i < children.Count; i++)
        {
            var itemIndex = generator.IndexFromGeneratorPosition(new GeneratorPosition(i, 0));
            if (itemIndex < 0) continue;

            var child = children[i];
            var x = itemIndex % _perRow * ItemWidth - _computedOffset.X;
            var y = itemIndex / _perRow * ItemHeight - _computedOffset.Y;
            child.Arrange(new Rect(x, y, child.DesiredSize.Width, ItemHeight));
        }

        return finalSize;
    }

    private double ItemWidth => Math.Max(1.0, _itemSize.IsEmpty ? 1.0 : _itemSize.Width);

    private double ItemHeight => Math.Max(1.0, _itemSize.IsEmpty ? 1.0 : _itemSize.Height);

    /// <summary>1 行に並ぶ数と行数を決める（WrapPanel と同じく、幅に収まるだけ並べて折り返す）。</summary>
    private int ComputeRows(double width, int count)
    {
        _perRow = double.IsInfinity(width)
            ? count
            : Math.Max(1, (int)Math.Floor(width / ItemWidth + 1e-9));
        return (count + _perRow - 1) / _perRow;
    }

    /// <summary>カードを作っておく範囲（見えている行 + 前後の予備の行）。</summary>
    private (int First, int Last) RealizedRange(Size available, int count, int rows)
    {
        // スクロールしない置き方をされた場合は、WrapPanel と同じく全件作る
        if (ScrollOwner is null || double.IsInfinity(available.Height)) return (0, count - 1);

        var firstRow = Math.Max(0, (int)Math.Floor(_computedOffset.Y / ItemHeight) - BufferRows);
        var lastRow = Math.Min(rows - 1, (int)Math.Floor((_computedOffset.Y + _viewport.Height) / ItemHeight) + BufferRows);

        return (firstRow * _perRow, Math.Min(count - 1, (lastRow + 1) * _perRow - 1));
    }

    /// <summary>first〜last の項目のカードを用意して測る。戻り値は first のカード。</summary>
    private UIElement? RealizeRange(
        IItemContainerGenerator generator, UIElementCollection children, int first, int last, Size constraint)
    {
        if (first > last) return null;

        UIElement? firstChild = null;
        var start = generator.GeneratorPositionFromIndex(first);
        var childIndex = start.Offset == 0 ? start.Index : start.Index + 1;

        using (generator.StartAt(start, GeneratorDirection.Forward, allowStartAtRealizedItem: true))
        {
            for (var i = first; i <= last; i++, childIndex++)
            {
                if (generator.GenerateNext(out var newlyRealized) is not UIElement child) break;

                if (newlyRealized)
                {
                    if (childIndex >= children.Count) AddInternalChild(child);
                    else InsertInternalChild(childIndex, child);
                    generator.PrepareItemContainer(child);
                }

                child.Measure(constraint);
                firstChild ??= child;
            }
        }

        return firstChild;
    }

    /// <summary>範囲外になったカードを片付ける。</summary>
    private void CleanUp(IItemContainerGenerator generator, int first, int last)
    {
        var children = InternalChildren;
        for (var i = children.Count - 1; i >= 0; i--)
        {
            var position = new GeneratorPosition(i, 0);
            var itemIndex = generator.IndexFromGeneratorPosition(position);
            if (itemIndex >= first && itemIndex <= last) continue;

            // キーボードで選んでいるカードは残す。スクロールして見えなくなっても、
            // WrapPanel のときと同じく、そのカードからキー操作を続けられるようにする。
            if (itemIndex >= 0 && children[i].IsKeyboardFocusWithin) continue;

            generator.Remove(position, 1);
            RemoveInternalChildRange(i, 1);
        }
    }

    protected override void OnItemsChanged(object sender, ItemsChangedEventArgs args)
    {
        switch (args.Action)
        {
            case NotifyCollectionChangedAction.Remove:
            case NotifyCollectionChangedAction.Replace:
                RemoveInternalChildRange(args.Position.Index, args.ItemUICount);
                break;

            case NotifyCollectionChangedAction.Move:
                RemoveInternalChildRange(args.OldPosition.Index, args.ItemUICount);
                break;
        }

        base.OnItemsChanged(sender, args);
        InvalidateMeasure();
    }

    /// <summary>指定した位置の項目まで（必要なら）スクロールして、カードを作る。</summary>
    protected override void BringIndexIntoView(int index)
    {
        var owner = ItemsControl.GetItemsOwner(this);
        if (owner is null || index < 0 || index >= owner.Items.Count) return;

        if (_itemSize.IsEmpty)
        {
            InvalidateMeasure();
            UpdateLayout();
        }

        var top = index / _perRow * ItemHeight;
        SetVerticalOffset(ComputeScrollOffsetWithMinimalScroll(
            _computedOffset.Y, _computedOffset.Y + _viewport.Height, top, top + ItemHeight));
        UpdateLayout();

        if (owner.ItemContainerGenerator.ContainerFromIndex(index) is FrameworkElement container)
            container.BringIntoView();
    }

    private Size DesiredSizeFor(Size available) => new(
        double.IsInfinity(available.Width) ? _extent.Width : Math.Min(available.Width, _extent.Width),
        double.IsInfinity(available.Height) ? _extent.Height : Math.Min(available.Height, _extent.Height));

    // ------------------------------------------------------------ スクロール

    public bool CanVerticallyScroll { get; set; }

    public bool CanHorizontallyScroll { get; set; }

    public double ExtentWidth => _extent.Width;

    public double ExtentHeight => _extent.Height;

    public double ViewportWidth => _viewport.Width;

    public double ViewportHeight => _viewport.Height;

    public double HorizontalOffset => _computedOffset.X;

    public double VerticalOffset => _computedOffset.Y;

    public ScrollViewer? ScrollOwner { get; set; }

    public void LineUp() => SetVerticalOffset(VerticalOffset - LineDelta);

    public void LineDown() => SetVerticalOffset(VerticalOffset + LineDelta);

    public void PageUp() => SetVerticalOffset(VerticalOffset - ViewportHeight);

    public void PageDown() => SetVerticalOffset(VerticalOffset + ViewportHeight);

    public void MouseWheelUp() => SetVerticalOffset(VerticalOffset - WheelDelta);

    public void MouseWheelDown() => SetVerticalOffset(VerticalOffset + WheelDelta);

    public void LineLeft() => SetHorizontalOffset(HorizontalOffset - LineDelta);

    public void LineRight() => SetHorizontalOffset(HorizontalOffset + LineDelta);

    public void PageLeft() => SetHorizontalOffset(HorizontalOffset - ViewportWidth);

    public void PageRight() => SetHorizontalOffset(HorizontalOffset + ViewportWidth);

    public void MouseWheelLeft() => SetHorizontalOffset(HorizontalOffset - WheelDelta);

    public void MouseWheelRight() => SetHorizontalOffset(HorizontalOffset + WheelDelta);

    public void SetHorizontalOffset(double offset)
    {
        if (double.IsNaN(offset)) throw new ArgumentOutOfRangeException(nameof(offset));

        // 上限は次のレイアウトで表示範囲に収める（標準のスクロールと同じ）
        offset = Math.Max(0, offset);
        if (AreClose(offset, _offset.X)) return;

        _offset.X = offset;
        InvalidateMeasure();
    }

    public void SetVerticalOffset(double offset)
    {
        if (double.IsNaN(offset)) throw new ArgumentOutOfRangeException(nameof(offset));

        offset = Math.Max(0, offset);
        if (AreClose(offset, _offset.Y)) return;

        _offset.Y = offset;
        InvalidateMeasure();
    }

    /// <summary>
    /// visual の rectangle が見えるように最小限スクロールする（キー操作でカードを選んだときなど）。
    /// 計算方法は標準のスクロール（ScrollContentPresenter）と同じ。
    /// </summary>
    public Rect MakeVisible(Visual visual, Rect rectangle)
    {
        if (rectangle.IsEmpty || visual is null || ReferenceEquals(visual, this) || !IsAncestorOf(visual))
            return Rect.Empty;

        rectangle = visual.TransformToAncestor(this).TransformBounds(rectangle);

        var viewport = new Rect(_computedOffset.X, _computedOffset.Y, _viewport.Width, _viewport.Height);
        rectangle.X += viewport.X;
        rectangle.Y += viewport.Y;

        var x = ComputeScrollOffsetWithMinimalScroll(viewport.Left, viewport.Right, rectangle.Left, rectangle.Right);
        var y = ComputeScrollOffsetWithMinimalScroll(viewport.Top, viewport.Bottom, rectangle.Top, rectangle.Bottom);
        SetHorizontalOffset(x);
        SetVerticalOffset(y);

        viewport.X = x;
        viewport.Y = y;
        rectangle.Intersect(viewport);
        if (!rectangle.IsEmpty)
        {
            rectangle.X -= viewport.X;
            rectangle.Y -= viewport.Y;
        }

        return rectangle;
    }

    /// <summary>表示範囲・全体の大きさを更新し、スクロール位置を範囲内に収める。</summary>
    private void UpdateScrollInfo(Size viewport, Size extent)
    {
        if (double.IsInfinity(viewport.Width)) viewport.Width = extent.Width;
        if (double.IsInfinity(viewport.Height)) viewport.Height = extent.Height;

        // 指定された位置（_offset）は残したまま、実際の位置だけを範囲内に収める
        _computedOffset.X = CoerceOffset(_offset.X, extent.Width, viewport.Width);
        _computedOffset.Y = CoerceOffset(_offset.Y, extent.Height, viewport.Height);

        var changed = viewport != _viewport || extent != _extent || _computedOffset != _reportedOffset;
        _viewport = viewport;
        _extent = extent;
        _reportedOffset = _computedOffset;

        if (changed) ScrollOwner?.InvalidateScrollInfo();
    }

    private static double CoerceOffset(double offset, double extent, double viewport)
    {
        if (offset > extent - viewport) offset = extent - viewport;
        if (offset < 0) offset = 0;
        return offset;
    }

    /// <summary>
    /// 子の上端・下端が表示範囲に入るための、最小限のスクロール位置（ScrollContentPresenter と同じ規則）。
    ///   上にはみ出している → 子の上端に合わせる（子が表示範囲より大きければ下端に合わせる）
    ///   下にはみ出している → 子の下端に合わせる（子が表示範囲より大きければ上端に合わせる）
    ///   収まっている / 表示範囲をまたいでいる → 動かさない
    /// </summary>
    private static double ComputeScrollOffsetWithMinimalScroll(
        double topView, double bottomView, double topChild, double bottomChild)
    {
        var above = LessThan(topChild, topView) && LessThan(bottomChild, bottomView);
        var below = GreaterThan(bottomChild, bottomView) && GreaterThan(topChild, topView);
        var larger = bottomChild - topChild > bottomView - topView;

        if ((above && !larger) || (below && larger)) return topChild;
        if (above || below) return bottomChild - (bottomView - topView);
        return topView;
    }

    // WPF の DoubleUtil と同じ誤差の扱い
    private static bool AreClose(double a, double b)
    {
        if (a == b) return true;
        var eps = (Math.Abs(a) + Math.Abs(b) + 10.0) * 2.2204460492503131e-016;
        var delta = a - b;
        return -eps < delta && eps > delta;
    }

    private static bool LessThan(double a, double b) => a < b && !AreClose(a, b);

    private static bool GreaterThan(double a, double b) => a > b && !AreClose(a, b);
}
