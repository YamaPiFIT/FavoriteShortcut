using System.Collections.ObjectModel;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using FavoriteShortcut.Controls;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using static FavoriteShortcut.Tests.TestRunner;

namespace FavoriteShortcut.Tests;

/// <summary>
/// 件数が多いときの処理速度のための仕組みが、結果を変えずに働いているかを確認する。
/// （速さそのものは環境で変わるので測らず、「何回作り直したか」「使い回したか」を見る）
/// </summary>
internal static class PerformanceTests
{
    public static void Run()
    {
        BatchTests();
        SearchIndexTests();
        FolderTests();
        CardPanelTests();
    }

    private static (Database Db, AppStore Store) NewStore(string fileName)
    {
        var db = new Database(Path.Combine(AppPaths.DataDirectory, fileName));
        return (db, new AppStore(db));
    }

    private static ShortcutItem Web(string title, string url, string? folderId = null, params string[] tags) => new()
    {
        Title = title,
        Target = url,
        TargetType = TargetType.Web,
        FolderId = folderId,
        Tags = tags.ToList(),
    };

    // ------------------------------------------------------- まとめて変更

    private static void BatchTests()
    {
        Group("まとめて変更（取り込み・複数削除などの通知を 1 回にまとめる）");

        var (db, store) = NewStore("batch-test.sqlite");
        using (db)
        {
            var notified = 0;
            store.DataChanged += (_, _) => notified++;

            Test("まとめて変更している間の通知は、最後の 1 回にまとまる", () =>
            {
                notified = 0;
                using (store.BeginBatch())
                {
                    var folder = store.CreateFolder("取り込み", null);
                    for (var i = 0; i < 50; i++)
                        store.AddShortcut(Web($"項目{i:D3}", $"https://example.com/{i}", folder.Id));

                    AssertEqual(0, notified, "途中では通知しない");
                }
                AssertEqual(1, notified, "最後に 1 回だけ通知する");
                AssertEqual(50, store.Shortcuts.Count, "登録件数");
            });

            Test("まとめて変更中に作ったフォルダのフルパスは、すぐに使える", () =>
            {
                using (store.BeginBatch())
                {
                    var parent = store.CreateFolder("ブックマーク バー", null);
                    var child = store.CreateFolder("開発", parent.Id);
                    AssertEqual("ブックマーク バー > 開発", child.FullPath, "フォルダのフルパス");

                    var item = Web("Docs", "https://example.com/docs", child.Id);
                    store.AddShortcut(item);
                    AssertEqual("ブックマーク バー > 開発", item.FolderPathText, "ショートカットのフォルダ表示");
                }
            });

            Test("まとめて変更が終わると、フォルダのツリーが組み直される", () =>
            {
                var parent = store.RootFolders.FirstOrDefault(f => f.Name == "ブックマーク バー");
                AssertTrue(parent is not null, "最上位のフォルダとして表示される");
                AssertTrue(parent!.Children.Any(c => c.Name == "開発"), "子フォルダがツリーに入る");
            });

            Test("入れ子にしても、通知は外側を抜けたときの 1 回", () =>
            {
                notified = 0;
                using (store.BeginBatch())
                {
                    using (store.BeginBatch())
                        store.AddShortcut(Web("入れ子", "https://example.com/nested"));

                    AssertEqual(0, notified, "内側を抜けても通知しない");
                }
                AssertEqual(1, notified, "外側を抜けたら通知する");
            });

            Test("途中で失敗しても、それまでの変更は保存され画面にも通知される", () =>
            {
                notified = 0;
                var before = store.Shortcuts.Count;
                try
                {
                    using (store.BeginBatch())
                    {
                        store.AddShortcut(Web("失敗前", "https://example.com/before-error"));
                        throw new InvalidOperationException("テスト用の失敗");
                    }
                }
                catch (InvalidOperationException)
                {
                    // 想定どおり
                }

                AssertEqual(before + 1, store.Shortcuts.Count, "失敗前の登録は残る");
                AssertEqual(1, notified, "通知回数");
            });

            Test("使われなくなったタグは、まとめて変更の最後に削除される", () =>
            {
                var item = Web("タグ付き", "https://example.com/tagged", null, "一時タグ");
                store.AddShortcut(item);

                using (store.BeginBatch())
                    store.DeleteShortcut(item);

                AssertTrue(!store.AllTagNames.Contains("一時タグ"), "未使用タグの削除");
            });

            Test("タグ名の一括変更は、件数が多くても通知 1 回で済む", () =>
            {
                for (var i = 0; i < 20; i++)
                    store.AddShortcut(Web($"タグ{i}", $"https://example.com/tag/{i}", null, "旧名", "共通"));

                notified = 0;
                store.RenameTag("旧名", "新名");

                AssertEqual(1, notified, "通知回数");
                AssertEqual(20, store.Shortcuts.Count(s => s.Tags.Contains("新名")), "変更後のタグ");
                AssertTrue(!store.AllTagNames.Contains("旧名"), "旧タグは消える");
                AssertTrue(store.Shortcuts.Where(s => s.Tags.Contains("新名")).All(s => s.Tags[1] == "共通"),
                    "他のタグと並び順はそのまま");
            });

            Test("タグの一括削除も通知 1 回で済む", () =>
            {
                notified = 0;
                store.DeleteTag("共通");

                AssertEqual(1, notified, "通知回数");
                AssertTrue(store.Shortcuts.All(s => !s.Tags.Contains("共通")), "タグが外れる");
                AssertTrue(!store.AllTagNames.Contains("共通"), "未使用タグの削除");
            });

            Test("まとめて変更した内容は、DB から読み直しても同じ", () =>
            {
                var titles = store.Shortcuts.Select(s => s.Title).OrderBy(t => t, StringComparer.Ordinal).ToList();
                var paths = store.AllFolders.Select(f => f.FullPath).OrderBy(p => p, StringComparer.Ordinal).ToList();

                store.Reload();

                AssertTrue(titles.SequenceEqual(store.Shortcuts.Select(s => s.Title).OrderBy(t => t, StringComparer.Ordinal)),
                    "ショートカット");
                AssertTrue(paths.SequenceEqual(store.AllFolders.Select(f => f.FullPath).OrderBy(p => p, StringComparer.Ordinal)),
                    "フォルダ");
            });
        }
    }

    // ------------------------------------------------------- 検索の索引

    private static void SearchIndexTests()
    {
        Group("検索の索引（変わった項目だけ作り直す）");

        var (db, store) = NewStore("index-test.sqlite");
        using (db)
        {
            var folder = store.CreateFolder("資料", null);
            for (var i = 0; i < 30; i++)
                store.AddShortcut(Web($"項目{i:D3}", $"https://example.com/{i}", folder.Id));

            Test("他の項目を追加・変更しても、変わっていない項目の索引は使い回す", () =>
            {
                var item = store.Shortcuts.First(s => s.Title == "項目001");
                SearchService.Search(store.Shortcuts, "項目", store.Revision);
                var index = item.SearchIndex;
                AssertTrue(index is not null, "索引が作られる");

                store.AddShortcut(Web("別の項目", "https://example.com/other"));
                var other = store.Shortcuts.First(s => s.Title == "項目002");
                other.Note = "メモを追加";
                store.UpdateShortcut(other);

                SearchService.Search(store.Shortcuts, "項目", store.Revision);
                AssertTrue(ReferenceEquals(index, item.SearchIndex), "作り直していない");
            });

            Test("タイトルを変えた項目は、新しいタイトルで見つかり古いタイトルでは見つからない", () =>
            {
                var item = store.Shortcuts.First(s => s.Title == "項目003");
                SearchService.Search(store.Shortcuts, "項目003", store.Revision);

                item.Title = "改名済み";
                store.UpdateShortcut(item);

                var hits = SearchService.Search(store.Shortcuts, "改名済み", store.Revision)!;
                AssertTrue(hits.Any(h => ReferenceEquals(h.Item, item)), "新しいタイトルで見つかる");

                var old = SearchService.Search(store.Shortcuts, "項目003", store.Revision)!;
                AssertTrue(old.All(h => !ReferenceEquals(h.Item, item)), "古いタイトルでは見つからない");
            });

            Test("メモ・タグを変えた項目も、変更後の内容で見つかる", () =>
            {
                var item = store.Shortcuts.First(s => s.Title == "項目004");
                SearchService.Search(store.Shortcuts, "項目", store.Revision);

                item.Note = "議事録の置き場所";
                item.Tags = new List<string> { "会議" };
                store.UpdateShortcut(item);

                AssertTrue(SearchService.Search(store.Shortcuts, "議事録", store.Revision)!.Any(h => ReferenceEquals(h.Item, item)),
                    "メモで見つかる");
                AssertTrue(SearchService.Search(store.Shortcuts, "会議", store.Revision)!.Any(h => ReferenceEquals(h.Item, item)),
                    "タグで見つかる");
            });

            Test("フォルダ名を変えると、中の項目は新しいフォルダ名で見つかる", () =>
            {
                SearchService.Search(store.Shortcuts, "資料", store.Revision);
                store.RenameFolder(folder, "共有資料");

                var hits = SearchService.Search(store.Shortcuts, "共有資料", store.Revision)!;
                AssertEqual(store.Shortcuts.Count(s => s.FolderId == folder.Id), hits.Count, "フォルダ内の全件");
            });

            Test("別のフォルダへ移動すると、移動先のフォルダ名で見つかる", () =>
            {
                var other = store.CreateFolder("移動先", null);
                var item = store.Shortcuts.First(s => s.Title == "項目005");
                SearchService.Search(store.Shortcuts, "項目", store.Revision);

                store.MoveShortcut(item, other.Id);

                var hits = SearchService.Search(store.Shortcuts, "移動先", store.Revision)!;
                AssertTrue(hits.Count == 1 && ReferenceEquals(hits[0].Item, item), "移動先で見つかる");
            });

            Test("フォルダを削除して未分類になった項目は、未分類として見つかる", () =>
            {
                var temp = store.CreateFolder("一時フォルダ", null);
                var item = Web("退避される項目", "https://example.com/evacuated", temp.Id);
                store.AddShortcut(item);
                SearchService.Search(store.Shortcuts, "一時フォルダ", store.Revision);

                store.DeleteFolder(temp, deleteShortcuts: false);

                AssertTrue(SearchService.Search(store.Shortcuts, "一時フォルダ", store.Revision)!.Count == 0,
                    "古いフォルダ名では見つからない");
                AssertTrue(SearchService.Search(store.Shortcuts, Loc.T("Str.Folder.Uncategorized"), store.Revision)!
                    .Any(h => ReferenceEquals(h.Item, item)), "未分類で見つかる");
            });
        }
    }

    // ------------------------------------------------------- フォルダ

    private static void FolderTests()
    {
        Group("フォルダの階層（多数のフォルダ）");

        var (db, store) = NewStore("folder-test.sqlite");
        using (db)
        {
            Test("フォルダと配下のフォルダを漏れなく集める（まとめて変更中も）", () =>
            {
                using (store.BeginBatch())
                {
                    var root = store.CreateFolder("階層", null);
                    var expected = new HashSet<string> { root.Id };
                    var level = new List<FolderItem> { root };
                    for (var depth = 0; depth < 4; depth++)
                    {
                        var next = new List<FolderItem>();
                        foreach (var parent in level)
                        {
                            for (var i = 0; i < 3; i++)
                            {
                                var folder = store.CreateFolder($"{parent.Name}-{i}", parent.Id);
                                expected.Add(folder.Id);
                                next.Add(folder);
                            }
                        }
                        level = next;
                    }

                    store.CreateFolder("無関係", null);

                    var collected = store.CollectFolderAndDescendants(root);
                    AssertEqual(expected.Count, collected.Count, "件数");
                    AssertTrue(collected.All(f => expected.Contains(f.Id)), "配下のフォルダだけ");
                    AssertTrue(ReferenceEquals(collected[0], root), "先頭は指定したフォルダ");
                }
            });
        }
    }

    // ------------------------------------------------------- カード表示

    private const double CardWidth = 180;
    private const double CardHeight = 132;
    private const double CardMargin = 4;
    private const double SlotWidth = CardWidth + CardMargin * 2;
    private const double SlotHeight = CardHeight + CardMargin * 2;

    private static void CardPanelTests()
    {
        Group("カード表示（見えている分だけカードを作るパネル）");

        // WPF の部品は STA スレッドでしか作れない
        RunOnSta(() =>
        {
            var items = new ObservableCollection<int>(Enumerable.Range(0, 1000));
            var list = CreateCardList(items);
            var panel = FindDescendant<VirtualizingWrapPanel>(list)!;

            Test("パネルが一覧のスクロールを担当している", () =>
                AssertTrue(panel is not null && panel.ScrollOwner is not null, "スクロールの担当"));

            var perRow = (int)Math.Floor(panel.ViewportWidth / SlotWidth);
            var rows = (1000 + perRow - 1) / perRow;

            Test("1 行の並ぶ数と全体の高さは WrapPanel と同じ", () =>
            {
                AssertTrue(perRow >= 2, "1 行に複数並ぶ");
                AssertEqual(rows * SlotHeight, panel.ExtentHeight, "全体の高さ");
            });

            Test("作るカードは見えている行と前後 1 行の分だけ", () =>
            {
                var visibleRows = (int)Math.Ceiling(panel.ViewportHeight / SlotHeight) + 1;
                AssertTrue(VisualTreeHelper.GetChildrenCount(panel) <= visibleRows * perRow + perRow,
                    $"作ったカード {VisualTreeHelper.GetChildrenCount(panel)} 枚");
            });

            Test("カードの位置は WrapPanel と同じ（左から順に並べて折り返す）", () => AssertPositions(list, panel, perRow));

            Test("ホイール 1 回で 48px、1 行で 16px、1 画面分のページ送り", () =>
            {
                panel.MouseWheelDown();
                list.UpdateLayout();
                AssertEqual(48.0, panel.VerticalOffset, "ホイール");

                panel.LineDown();
                list.UpdateLayout();
                AssertEqual(64.0, panel.VerticalOffset, "1 行");

                var before = panel.VerticalOffset;
                panel.PageDown();
                list.UpdateLayout();
                AssertEqual(before + panel.ViewportHeight, panel.VerticalOffset, "ページ送り");

                AssertPositions(list, panel, perRow);
            });

            Test("一番下より先へはスクロールしない", () =>
            {
                panel.SetVerticalOffset(double.PositiveInfinity);
                list.UpdateLayout();
                AssertEqual(panel.ExtentHeight - panel.ViewportHeight, panel.VerticalOffset, "スクロール位置");
                AssertTrue(list.ItemContainerGenerator.ContainerFromIndex(999) is not null, "最後のカードが作られる");
            });

            Test("画面外の項目を指定すると、そこまでスクロールしてカードを作る", () =>
            {
                panel.SetVerticalOffset(0);
                list.UpdateLayout();

                list.ScrollIntoView(items[600]);
                list.UpdateLayout();

                var container = list.ItemContainerGenerator.ContainerFromIndex(600) as FrameworkElement;
                AssertTrue(container is not null, "カードが作られる");

                var top = container!.TranslatePoint(new Point(0, 0), panel).Y;
                AssertTrue(top >= -0.5 && top + SlotHeight <= panel.ViewportHeight + 0.5, $"見える位置にある（上端 {top}）");

                // 下方向へ最小限だけスクロールする（項目の下端を表示範囲の下端に合わせる）
                var row = 600 / perRow;
                AssertEqual((row + 1) * SlotHeight - panel.ViewportHeight, panel.VerticalOffset, "スクロール位置");
            });

            Test("件数が減ったら、スクロール位置は範囲内に収まる", () =>
            {
                while (items.Count > 10) items.RemoveAt(items.Count - 1);
                list.UpdateLayout();

                AssertEqual(0.0, panel.VerticalOffset, "スクロール位置");
                AssertEqual(10, VisualTreeHelper.GetChildrenCount(panel), "カードの枚数");
                AssertPositions(list, panel, perRow);
            });

            Test("実際の WrapPanel と同じ位置に並び、同じだけスクロールする", () =>
            {
                var itemsA = new ObservableCollection<int>(Enumerable.Range(0, 300));
                var itemsB = new ObservableCollection<int>(Enumerable.Range(0, 300));
                var a = CreateCardList(itemsA);
                var b = CreateCardList(itemsB, useWrapPanel: true);
                var mine = FindDescendant<VirtualizingWrapPanel>(a)!;
                var standard = FindDescendant<ScrollContentPresenter>(b)!;

                void Compare(string what)
                {
                    a.UpdateLayout();
                    b.UpdateLayout();
                    AssertEqual(standard.ExtentHeight, mine.ExtentHeight, what + ": 全体の高さ");
                    AssertEqual(standard.ViewportHeight, mine.ViewportHeight, what + ": 表示範囲の高さ");
                    AssertEqual(standard.VerticalOffset, mine.VerticalOffset, what + ": スクロール位置");

                    for (var i = 0; i < VisualTreeHelper.GetChildrenCount(mine); i++)
                    {
                        var card = (FrameworkElement)VisualTreeHelper.GetChild(mine, i);
                        var index = a.ItemContainerGenerator.IndexFromContainer(card);
                        var other = (FrameworkElement)b.ItemContainerGenerator.ContainerFromIndex(index);
                        var p1 = card.TranslatePoint(new Point(0, 0), mine);
                        var p2 = other.TranslatePoint(new Point(0, 0), standard);
                        AssertTrue(Math.Abs(p1.X - p2.X) < 0.5 && Math.Abs(p1.Y - p2.Y) < 0.5,
                            $"{what}: {index} 番目の位置 {p1} / {p2}");
                    }
                }

                Compare("最初の表示");

                mine.MouseWheelDown();
                standard.MouseWheelDown();
                Compare("ホイール");

                mine.LineDown();
                standard.LineDown();
                Compare("1 行");

                mine.PageDown();
                standard.PageDown();
                Compare("ページ送り");

                // キー操作で選んだカードが画面の外にあるときと同じ「見える位置まで」のスクロール
                var perRowA = (int)Math.Floor(mine.ViewportWidth / SlotWidth);
                var below = (int)((mine.VerticalOffset + mine.ViewportHeight) / SlotHeight + 1) * perRowA;
                ((FrameworkElement)a.ItemContainerGenerator.ContainerFromIndex(below)).BringIntoView();
                ((FrameworkElement)b.ItemContainerGenerator.ContainerFromIndex(below)).BringIntoView();
                Compare("下のカードを表示");

                var above = (int)(mine.VerticalOffset / SlotHeight - 1) * perRowA + 1;
                ((FrameworkElement)a.ItemContainerGenerator.ContainerFromIndex(above)).BringIntoView();
                ((FrameworkElement)b.ItemContainerGenerator.ContainerFromIndex(above)).BringIntoView();
                Compare("上のカードを表示");

                mine.SetVerticalOffset(double.PositiveInfinity);
                standard.SetVerticalOffset(double.PositiveInfinity);
                Compare("一番下");

                mine.MouseWheelUp();
                standard.MouseWheelUp();
                Compare("ホイールで戻る");

                // 一覧を作り直したとき（管理画面と同じく、空にしてから 1 件ずつ追加する）
                void Refill(int count)
                {
                    foreach (var items in new[] { itemsA, itemsB })
                    {
                        items.Clear();
                        foreach (var i in Enumerable.Range(0, count)) items.Add(i);
                    }
                }

                mine.SetVerticalOffset(2000);
                standard.SetVerticalOffset(2000);
                Compare("スクロールした状態");

                Refill(250);
                Compare("作り直して件数が減った");

                Refill(40);
                Compare("作り直して件数が大きく減った");

                Refill(300);
                Compare("作り直して件数が増えた");
            });

            Test("空にしてから追加し直しても正しく並ぶ", () =>
            {
                var items2 = new ObservableCollection<int>();
                var other = CreateCardList(items2);
                var otherPanel = FindDescendant<VirtualizingWrapPanel>(other)!;

                foreach (var i in Enumerable.Range(0, 500)) items2.Add(i);
                other.UpdateLayout();
                AssertPositions(other, otherPanel, perRow);

                items2.Clear();
                foreach (var i in Enumerable.Range(1000, 3)) items2.Add(i);
                other.UpdateLayout();
                AssertEqual(3, VisualTreeHelper.GetChildrenCount(otherPanel), "カードの枚数");
                AssertEqual(1000, (int)((FrameworkElement)VisualTreeHelper.GetChild(otherPanel, 0)).DataContext, "先頭の項目");
            });
        });
    }

    /// <summary>作られているカードが、すべて WrapPanel と同じ位置にあるか確かめる。</summary>
    private static void AssertPositions(ListBox list, VirtualizingWrapPanel panel, int perRow)
    {
        var count = VisualTreeHelper.GetChildrenCount(panel);
        AssertTrue(count > 0, "カードがある");

        for (var i = 0; i < count; i++)
        {
            var child = (FrameworkElement)VisualTreeHelper.GetChild(panel, i);
            var index = list.ItemContainerGenerator.IndexFromContainer(child);
            var expected = new Point(index % perRow * SlotWidth, index / perRow * SlotHeight - panel.VerticalOffset);
            var actual = child.TranslatePoint(new Point(0, 0), panel);

            AssertTrue(Math.Abs(expected.X - actual.X) < 0.5 && Math.Abs(expected.Y - actual.Y) < 0.5,
                $"{index} 番目の位置 期待 {expected} 実際 {actual}");
        }
    }

    /// <summary>
    /// アプリのカード表示と同じ大きさ（180×132、余白 4）のカードを並べる一覧を作る。
    /// useWrapPanel を指定すると、比較用に従来の WrapPanel で並べる。
    /// </summary>
    private static ListBox CreateCardList(System.Collections.IEnumerable items, bool useWrapPanel = false)
    {
        var card = new FrameworkElementFactory(typeof(Border));
        card.SetValue(FrameworkElement.WidthProperty, CardWidth);
        card.SetValue(FrameworkElement.HeightProperty, CardHeight);

        var container = new FrameworkElementFactory(typeof(Border));
        container.SetValue(FrameworkElement.MarginProperty, new Thickness(CardMargin));
        container.AppendChild(new FrameworkElementFactory(typeof(ContentPresenter)));

        var containerStyle = new Style(typeof(ListBoxItem));
        containerStyle.Setters.Add(new Setter(Control.TemplateProperty,
            new ControlTemplate(typeof(ListBoxItem)) { VisualTree = container }));

        var list = new ListBox
        {
            Width = 820,
            Height = 600,
            ItemsSource = items,
            ItemTemplate = new DataTemplate { VisualTree = card },
            ItemContainerStyle = containerStyle,
            ItemsPanel = new ItemsPanelTemplate(new FrameworkElementFactory(
                useWrapPanel ? typeof(WrapPanel) : typeof(VirtualizingWrapPanel))),
        };
        ScrollViewer.SetHorizontalScrollBarVisibility(list, ScrollBarVisibility.Disabled);
        VirtualizingPanel.SetIsVirtualizing(list, true);

        // 画面に置かずに使うので、初期化の完了を明示して標準の見た目（テンプレート）を適用させる
        list.BeginInit();
        list.EndInit();

        list.Measure(new Size(820, 600));
        list.Arrange(new Rect(0, 0, 820, 600));
        list.UpdateLayout();
        return list;
    }

    private static T? FindDescendant<T>(DependencyObject root) where T : DependencyObject
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
        {
            var child = VisualTreeHelper.GetChild(root, i);
            if (child is T found) return found;
            if (FindDescendant<T>(child) is { } nested) return nested;
        }
        return null;
    }

    private static void RunOnSta(Action body)
    {
        Exception? error = null;
        var thread = new Thread(() =>
        {
            try { body(); }
            catch (Exception ex) { error = ex; }
        });
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        thread.Join();

        if (error is not null)
            Test("カード表示の検証を実行できる", () => throw error);
    }
}
