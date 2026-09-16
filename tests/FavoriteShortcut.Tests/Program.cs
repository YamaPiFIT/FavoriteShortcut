using System.IO;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using static FavoriteShortcut.Tests.TestRunner;

namespace FavoriteShortcut.Tests;

/// <summary>
/// 開発指示書 §44「テスト項目」を自動で確認するための検証プログラム。
/// UI 操作（IME まわりなど）は自動化できないため、データ層・検索・移植を中心に検証する。
/// </summary>
public static class Program
{
    public static int Main()
    {
        Console.OutputEncoding = System.Text.Encoding.UTF8;

        // 実データを汚さないよう、一時フォルダをデータ保存場所として使う
        var sandbox = Path.Combine(Path.GetTempPath(), "fsc-tests-" + Guid.NewGuid().ToString("N"));
        Environment.SetEnvironmentVariable(AppPaths.DataDirectoryEnvironmentVariable, sandbox);

        Console.WriteLine("お気に入りショートカット 検証");
        Console.WriteLine($"データ保存場所: {AppPaths.DataDirectory}");

        try
        {
            TargetResolverTests();
            TextNormalizerTests();
            StoreTests();
            SearchTests();
            SettingsTests();
            TransferTests();
            RestartTests();
        }
        finally
        {
            TryCleanUp(sandbox);
        }

        return Summarize();
    }

    // ------------------------------------------------------------- 種別判定

    private static void TargetResolverTests()
    {
        Group("URL / パスの判定（§9）");

        Test("https:// は Web", () =>
            AssertEqual(TargetType.Web, TargetResolver.Detect("https://www.google.com/"), "種別"));

        Test("http:// は Web", () =>
            AssertEqual(TargetType.Web, TargetResolver.Detect("http://example.com"), "種別"));

        Test("スキームなしのホスト名も Web", () =>
            AssertEqual(TargetType.Web, TargetResolver.Detect("example.co.jp"), "種別"));

        Test("ドライブレターの実在フォルダはフォルダ", () =>
            AssertEqual(TargetType.Folder, TargetResolver.Detect(@"C:\Windows"), "種別"));

        Test("拡張子つきのパスはファイル", () =>
            AssertEqual(TargetType.File, TargetResolver.Detect(@"D:\Work\Project\test.pdf"), "種別"));

        Test("EXE はアプリケーション", () =>
            AssertEqual(TargetType.Application, TargetResolver.Detect(@"C:\Windows\System32\notepad.exe"), "種別"));

        Test("UNC パスはフォルダ", () =>
            AssertEqual(TargetType.Folder, TargetResolver.Detect(@"\\server\share\folder"), "種別"));

        Test("存在しないファイルでも種別は判定できる", () =>
            AssertEqual(TargetType.File, TargetResolver.Detect(@"C:\test\sample.xlsx"), "種別"));

        Test("スキームなしの Web には https:// を補う", () =>
            AssertEqual("https://example.com", TargetResolver.Normalize("example.com", TargetType.Web), "正規化"));

        Test("パスは書き換えない", () =>
            AssertEqual(@"C:\Windows", TargetResolver.Normalize(@"C:\Windows", TargetType.Folder), "正規化"));

        Test("引用符つきで貼り付けたパスを補正する", () =>
            AssertEqual(@"C:\My Files\a.txt",
                TargetResolver.Normalize("\"C:\\My Files\\a.txt\"", TargetType.File), "正規化"));

        Test("Web のタイトル候補はホスト名（www. を除く）", () =>
            AssertEqual("google.com", TargetResolver.SuggestTitle("https://www.google.com/", TargetType.Web), "タイトル"));

        Test("ファイルのタイトル候補は拡張子なしのファイル名", () =>
            AssertEqual("test", TargetResolver.SuggestTitle(@"D:\Work\test.pdf", TargetType.File), "タイトル"));

        Test("存在しないパスは Exists が false", () =>
            AssertTrue(!TargetResolver.Exists(@"C:\test\sample.xlsx", TargetType.File), "Exists"));
    }

    // ----------------------------------------------------------- 文字列正規化

    private static void TextNormalizerTests()
    {
        Group("検索用の文字列正規化（§14）");

        Test("大文字小文字を同一視する", () =>
            AssertEqual(TextNormalizer.Normalize("Analytics"), TextNormalizer.Normalize("ANALYTICS"), "正規化"));

        Test("全角英数を半角に揃える", () =>
            AssertEqual("google", TextNormalizer.Normalize("Ｇｏｏｇｌｅ"), "正規化"));

        Test("ひらがなとカタカナを同一視する", () =>
            AssertEqual(TextNormalizer.Normalize("かいはつ"), TextNormalizer.Normalize("カイハツ"), "正規化"));

        Test("全角スペースも区切りとして扱う", () =>
            AssertEqual(2, TextNormalizer.SplitTerms("AI　開発").Length, "語数"));

        Test("連続スペースで空の語を作らない", () =>
            AssertEqual(2, TextNormalizer.SplitTerms("  AI    開発  ").Length, "語数"));
    }

    // ------------------------------------------------------------ ストア

    private static (Database Db, AppStore Store) NewStore(string fileName)
    {
        var db = new Database(Path.Combine(AppPaths.DataDirectory, fileName));
        return (db, new AppStore(db));
    }

    private static void StoreTests()
    {
        Group("フォルダ / ショートカット / タグ（§21）");

        var (db, store) = NewStore("store-test.sqlite");
        using (db)
        {
            var work = store.CreateFolder("仕事", null);
            var dev = store.CreateFolder("開発", null);
            var project = store.CreateFolder("プロジェクト", dev.Id);
            var projectA = store.CreateFolder("Project A", project.Id);

            Test("フォルダを作成できる", () => AssertEqual(4, store.AllFolders.Count, "フォルダ数"));

            Test("階層のフルパスが作られる", () =>
                AssertEqual("開発 > プロジェクト > Project A", projectA.FullPath, "フルパス"));

            Test("フォルダ名を変更するとフルパスも追随する", () =>
            {
                store.RenameFolder(project, "案件");
                AssertEqual("開発 > 案件 > Project A", projectA.FullPath, "フルパス");
            });

            Test("自分の配下へは移動できない", () =>
                AssertTrue(!store.MoveFolder(dev, projectA.Id), "循環参照の防止"));

            var chatgpt = new ShortcutItem
            {
                Title = "ChatGPT",
                Target = "https://chatgpt.com/",
                TargetType = TargetType.Web,
                FolderId = work.Id,
                Tags = new List<string> { "AI", "仕事", "開発" },
            };
            store.AddShortcut(chatgpt);

            store.AddShortcut(new ShortcutItem
            {
                Title = "GitHub",
                Target = "https://github.com/",
                TargetType = TargetType.Web,
                FolderId = dev.Id,
                Tags = new List<string> { "開発", "Git" },
            });

            store.AddShortcut(new ShortcutItem
            {
                Title = "開発フォルダ",
                Target = @"C:\Windows",
                TargetType = TargetType.Folder,
                FolderId = projectA.Id,
                Tags = new List<string> { "開発" },
            });

            Test("ショートカットを登録できる", () => AssertEqual(3, store.Shortcuts.Count, "件数"));

            Test("日本語タグと英語タグを混在できる", () =>
                AssertEqual(4, store.AllTagNames.Count(), "タグ数"));

            Test("フォルダのフルパスがショートカットに反映される", () =>
                AssertEqual("仕事", chatgpt.FolderPathText, "フォルダ表示"));

            Test("タグを編集すると使われなくなったタグが消える", () =>
            {
                chatgpt.Tags = new List<string> { "AI", "開発" };
                store.UpdateShortcut(chatgpt);
                AssertTrue(!store.AllTagNames.Contains("仕事"), "未使用タグの削除");
            });

            Test("タグ名を一括変更できる", () =>
            {
                store.RenameTag("開発", "Development");
                AssertTrue(store.Shortcuts.All(s => !s.Tags.Contains("開発")), "旧タグが残っていない");
                AssertTrue(store.Shortcuts.Count(s => s.Tags.Contains("Development")) == 3, "新タグが付いている");
            });

            Test("ショートカットを別フォルダへ移動できる", () =>
            {
                store.MoveShortcut(chatgpt, projectA.Id);
                AssertEqual("開発 > 案件 > Project A", chatgpt.FolderPathText, "移動後のフォルダ");
            });

            Test("使用履歴を記録できる", () =>
            {
                store.RecordUsage(chatgpt);
                store.RecordUsage(chatgpt);
                AssertEqual(2, chatgpt.UsageCount, "使用回数");
                AssertTrue(chatgpt.LastUsedAt is not null, "最終使用日時");
            });

            Test("フォルダ削除でショートカットを未分類へ退避できる", () =>
            {
                store.DeleteFolder(project, deleteShortcuts: false);
                AssertEqual(2, store.AllFolders.Count, "残ったフォルダ数");
                AssertEqual(3, store.Shortcuts.Count, "ショートカットは消えない");
                AssertTrue(store.Shortcuts.Any(s => s.FolderId is null), "未分類へ移動している");
            });

            Test("フォルダ削除で中身も一緒に削除できる", () =>
            {
                store.DeleteFolder(dev, deleteShortcuts: true);
                AssertEqual(2, store.Shortcuts.Count, "GitHub が削除されている");
            });

            Test("ショートカットを削除できる", () =>
            {
                var target = store.Shortcuts.First();
                store.DeleteShortcut(target);
                AssertEqual(1, store.Shortcuts.Count, "削除後の件数");
            });
        }
    }

    // ------------------------------------------------------------- 検索

    private static void SearchTests()
    {
        Group("検索（§12〜§15, §55）");

        var (db, store) = NewStore("search-test.sqlite");
        using (db)
        {
            var work = store.CreateFolder("仕事", null);
            var ai = store.CreateFolder("AI", work.Id);
            var dev = store.CreateFolder("開発", null);

            void Add(string title, string target, TargetType type, string? folderId, params string[] tags) =>
                store.AddShortcut(new ShortcutItem
                {
                    Title = title,
                    Target = target,
                    TargetType = type,
                    FolderId = folderId,
                    Tags = tags.ToList(),
                });

            Add("ChatGPT", "https://chatgpt.com/", TargetType.Web, ai.Id, "AI", "仕事", "開発");
            Add("Claude", "https://claude.ai/", TargetType.Web, ai.Id, "AI", "開発");
            Add("Google Analytics", "https://analytics.google.com/", TargetType.Web, work.Id, "Google", "解析");
            Add("GitHub", "https://github.com/", TargetType.Web, dev.Id, "開発", "Git");
            Add("開発資料", @"C:\Windows", TargetType.Folder, dev.Id, "資料");
            Add("開発環境", @"C:\Windows\System32", TargetType.Folder, dev.Id, "環境");

            List<ShortcutItem> Search(string query) =>
                SearchService.Search(store.Shortcuts, query, store.Revision)!
                    .Select(h => h.Item).ToList();

            Test("タイトルで検索できる", () =>
                AssertTrue(Search("ChatGPT").Any(s => s.Title == "ChatGPT"), "ヒット"));

            Test("タグで検索できる", () =>
                AssertEqual(2, Search("AI").Count, "AI タグのヒット数"));

            Test("フォルダ名で検索できる", () =>
                AssertTrue(Search("仕事").Any(s => s.Title == "ChatGPT"), "フォルダ名ヒット"));

            Test("URL の一部で検索できる", () =>
                AssertTrue(Search("chatgpt.com").Any(s => s.Title == "ChatGPT"), "URL ヒット"));

            Test("日本語の部分一致で検索できる", () =>
            {
                // 「開発」で「開発資料」「開発環境」が、語の途中でも「資料」で「開発資料」が出る
                var byPrefix = Search("開発");
                AssertTrue(byPrefix.Any(s => s.Title == "開発資料"), "開発資料");
                AssertTrue(byPrefix.Any(s => s.Title == "開発環境"), "開発環境");

                var byMiddle = Search("資料");
                AssertTrue(byMiddle.Any(s => s.Title == "開発資料"), "語の途中での一致");
            });

            Test("大文字小文字を区別しない", () =>
                AssertTrue(Search("analytics").Any(s => s.Title == "Google Analytics"), "小文字でヒット"));

            Test("複数ワードは AND 検索になる", () =>
            {
                var hits = Search("AI 開発");
                AssertEqual(2, hits.Count, "両方に関係する件数");
                AssertTrue(hits.All(s => s.Title is "ChatGPT" or "Claude"), "内容");
            });

            Test("どちらかしか一致しない語の組み合わせは 0 件", () =>
                AssertEqual(0, Search("AI 解析").Count, "件数"));

            Test("完全一致が先頭に来る", () =>
                AssertEqual("GitHub", Search("github")[0].Title, "順位"));

            Test("タイトル先頭一致がタグ一致より上位に来る", () =>
            {
                var hits = Search("開発");
                AssertTrue(hits[0].Title.StartsWith("開発"), $"先頭は {hits[0].Title}");
            });

            Test("全角で入力してもヒットする", () =>
                AssertTrue(Search("ＧｉｔＨｕｂ").Any(s => s.Title == "GitHub"), "全角検索"));

            Test("空の検索語では null（＝絞り込みなし）を返す", () =>
                AssertTrue(SearchService.Search(store.Shortcuts, "   ", store.Revision) is null, "null"));

            Test("最近使った項目を新しい順に取得できる", () =>
            {
                var github = store.Shortcuts.First(s => s.Title == "GitHub");
                store.RecordUsage(github);
                var recent = SearchService.Recent(store.Shortcuts, 5);
                AssertEqual("GitHub", recent[0].Title, "先頭");
            });

            Test("よく使う項目がわずかに上位へ来る", () =>
            {
                var claude = store.Shortcuts.First(s => s.Title == "Claude");
                for (var i = 0; i < 30; i++) store.RecordUsage(claude);
                var hits = Search("AI");
                AssertEqual("Claude", hits[0].Title, "使用頻度の反映");
            });

            Test("1万件登録しても検索が 1 秒以内に終わる（§36）", () =>
            {
                for (var i = 0; i < 10000; i++)
                {
                    store.Shortcuts.Add(new ShortcutItem
                    {
                        Title = $"ダミー項目 {i}",
                        Target = $"https://example{i}.com/",
                        TargetType = TargetType.Web,
                        Tags = new List<string> { "ダミー", i % 2 == 0 ? "偶数" : "奇数" },
                    });
                }

                var watch = System.Diagnostics.Stopwatch.StartNew();
                var hits = SearchService.Search(store.Shortcuts, "ダミー 偶数", store.Revision)!;
                watch.Stop();

                AssertEqual(5000, hits.Count, "ヒット数");
                AssertTrue(watch.ElapsedMilliseconds < 1000, $"検索時間 {watch.ElapsedMilliseconds}ms");
                Console.WriteLine($"         （10,000 件からの検索: {watch.ElapsedMilliseconds}ms）");
            });
        }
    }

    // ------------------------------------------------------------- 設定

    private static void SettingsTests()
    {
        Group("設定の保存（§39）");

        var path = Path.Combine(AppPaths.DataDirectory, "settings-test.sqlite");

        using (var db = new Database(path))
        {
            var store = new AppStore(db);
            var settings = new SettingsService(store);

            Test("初期値が読める", () =>
                AssertEqual(ShortcutViewMode.Card, settings.Current.ViewMode, "既定の表示形式"));

            var updated = settings.Current.Clone();
            updated.ViewMode = ShortcutViewMode.List;
            updated.Theme = AppTheme.Dark;
            updated.IconSize = 48;
            updated.LauncherMaxResults = 20;
            settings.Save(updated);
        }

        using (var db = new Database(path))
        {
            var store = new AppStore(db);
            var settings = new SettingsService(store);

            Test("再起動しても設定が残る", () =>
            {
                AssertEqual(ShortcutViewMode.List, settings.Current.ViewMode, "表示形式");
                AssertEqual(AppTheme.Dark, settings.Current.Theme, "テーマ");
                AssertEqual(48, settings.Current.IconSize, "アイコンサイズ");
                AssertEqual(20, settings.Current.LauncherMaxResults, "ランチャー件数");
            });
        }
    }

    // --------------------------------------------------- エクスポート/インポート

    private static void TransferTests()
    {
        Group("エクスポート / インポート（§24〜§28）");

        var zipPath = Path.Combine(AppPaths.DataDirectory, "export-test.zip");
        var iconName = "custom_testicon.png";

        // --- 送り出す側（PC A に相当）---
        using (var db = new Database(Path.Combine(AppPaths.DataDirectory, "pc-a.sqlite")))
        {
            var store = new AppStore(db);
            var settings = new SettingsService(store);
            var transfer = new ExportImportService(store, settings);

            var work = store.CreateFolder("仕事", null);
            var ai = store.CreateFolder("AI", work.Id);
            var dev = store.CreateFolder("開発", null);

            // アイコンファイルが ZIP に含まれ、取り込み先で復元されることを確認する
            Directory.CreateDirectory(AppPaths.IconDirectory);
            File.WriteAllBytes(Path.Combine(AppPaths.IconDirectory, iconName), MinimalPng());

            var chatgpt = new ShortcutItem
            {
                Title = "ChatGPT",
                Target = "https://chatgpt.com/",
                TargetType = TargetType.Web,
                FolderId = ai.Id,
                IconPath = iconName,
                Tags = new List<string> { "AI", "仕事" },
            };
            store.AddShortcut(chatgpt);
            store.RecordUsage(chatgpt);
            store.RecordUsage(chatgpt);

            store.AddShortcut(new ShortcutItem
            {
                Title = "GitHub",
                Target = "https://github.com/",
                TargetType = TargetType.Web,
                FolderId = dev.Id,
                Tags = new List<string> { "開発", "Git" },
            });

            var s = settings.Current.Clone();
            s.IconSize = 64;
            settings.Save(s);

            transfer.Export(zipPath);

            Test("ZIP が作成される", () => AssertTrue(File.Exists(zipPath), "ファイルの存在"));

            Test("ZIP に必要なファイルが入っている", () =>
            {
                using var archive = System.IO.Compression.ZipFile.OpenRead(zipPath);
                var names = archive.Entries.Select(entry => entry.FullName.Replace('\\', '/')).ToList();
                AssertTrue(names.Contains("database.sqlite"), "database.sqlite");
                AssertTrue(names.Contains("manifest.json"), "manifest.json");
                AssertTrue(names.Contains("settings.json"), "settings.json");
                AssertTrue(names.Contains("icons/" + iconName), "アイコン");
            });
        }

        // --- 受け取る側（PC B に相当）: 置き換えインポート ---
        using (var db = new Database(Path.Combine(AppPaths.DataDirectory, "pc-b.sqlite")))
        {
            var store = new AppStore(db);
            var settings = new SettingsService(store);
            var transfer = new ExportImportService(store, settings);

            store.CreateFolder("取り込み前のフォルダ", null);
            store.AddShortcut(new ShortcutItem
            {
                Title = "取り込み前のショートカット",
                Target = "https://example.com/",
                TargetType = TargetType.Web,
            });

            var manifest = transfer.PeekManifest(zipPath);
            Test("マニフェストを読める", () =>
            {
                AssertTrue(manifest is not null, "マニフェスト");
                AssertEqual(2, manifest!.ShortcutCount, "ショートカット件数");
                AssertEqual(3, manifest.FolderCount, "フォルダ件数");
            });

            var summary = transfer.Import(zipPath, ImportMode.Replace);

            Test("置き換えインポートで元データが入れ替わる", () =>
            {
                AssertEqual(2, store.Shortcuts.Count, "ショートカット件数");
                AssertEqual(3, store.AllFolders.Count, "フォルダ件数");
                AssertTrue(!store.Shortcuts.Any(x => x.Title == "取り込み前のショートカット"), "旧データが消えている");
            });

            Test("フォルダ階層が保たれる", () =>
                AssertTrue(store.Shortcuts.Any(x => x.FolderPathText == "仕事 > AI"), "階層"));

            Test("タグが保たれる", () =>
            {
                var chatgpt = store.Shortcuts.First(x => x.Title == "ChatGPT");
                AssertTrue(chatgpt.Tags.Contains("AI") && chatgpt.Tags.Contains("仕事"), "タグ");
            });

            Test("使用履歴が保たれる", () =>
                AssertEqual(2, store.Shortcuts.First(x => x.Title == "ChatGPT").UsageCount, "使用回数"));

            Test("アイコンファイルが復元される", () =>
                AssertTrue(File.Exists(Path.Combine(AppPaths.IconDirectory, iconName)), "アイコン"));

            Test("設定も引き継がれる", () => AssertEqual(64, settings.Current.IconSize, "アイコンサイズ"));

            Test("取り込み前のバックアップが作られる", () =>
                AssertTrue(summary.BackupPath is not null && File.Exists(summary.BackupPath), "バックアップ"));

            // --- 同じ ZIP をもう一度、今度は追加モードで ---
            var second = transfer.Import(zipPath, ImportMode.Merge);

            Test("追加モードで同じ内容は二重登録されない", () =>
            {
                AssertEqual(2, store.Shortcuts.Count, "ショートカット件数");
                AssertEqual(2, second.SkippedShortcuts, "スキップ件数");
            });

            Test("追加モードで同名フォルダは作り直されない", () =>
                AssertEqual(3, store.AllFolders.Count, "フォルダ件数"));
        }

        // --- 追加モードで、別内容を足せることの確認 ---
        using (var db = new Database(Path.Combine(AppPaths.DataDirectory, "pc-c.sqlite")))
        {
            var store = new AppStore(db);
            var settings = new SettingsService(store);
            var transfer = new ExportImportService(store, settings);

            var local = store.CreateFolder("このPCだけのフォルダ", null);
            store.AddShortcut(new ShortcutItem
            {
                Title = "このPCだけの項目",
                Target = @"C:\Windows",
                TargetType = TargetType.Folder,
                FolderId = local.Id,
            });

            transfer.Import(zipPath, ImportMode.Merge);

            Test("追加モードでは既存データが残る", () =>
            {
                AssertEqual(3, store.Shortcuts.Count, "ショートカット件数");
                AssertTrue(store.Shortcuts.Any(x => x.Title == "このPCだけの項目"), "既存データ");
                AssertTrue(store.Shortcuts.Any(x => x.Title == "ChatGPT"), "取り込んだデータ");
            });

            Test("追加モードでもフォルダ階層が作られる", () =>
                AssertTrue(store.Shortcuts.Any(x => x.FolderPathText == "仕事 > AI"), "階層"));
        }

        Test("ZIP 外へ書き出そうとするエントリは無視される", () =>
        {
            var malicious = Path.Combine(AppPaths.DataDirectory, "malicious.zip");
            if (File.Exists(malicious)) File.Delete(malicious);

            using (var archive = System.IO.Compression.ZipFile.Open(
                       malicious, System.IO.Compression.ZipArchiveMode.Create))
            {
                var entry = archive.CreateEntry("../../escaped.txt");
                using var writer = new StreamWriter(entry.Open());
                writer.Write("should not be extracted");
            }

            using var db = new Database(Path.Combine(AppPaths.DataDirectory, "pc-d.sqlite"));
            var store = new AppStore(db);
            var transfer = new ExportImportService(store, new SettingsService(store));

            try { transfer.Import(malicious, ImportMode.Merge); }
            catch (InvalidDataException) { /* database.sqlite が無いので想定どおり失敗する */ }

            AssertTrue(!File.Exists(Path.Combine(Path.GetTempPath(), "escaped.txt")), "展開されていない");
        });
    }

    // --------------------------------------------------------- 再起動/移行

    private static void RestartTests()
    {
        Group("再起動後のデータ保持とスキーマ（§20, §47）");

        var path = Path.Combine(AppPaths.DataDirectory, "restart-test.sqlite");
        string shortcutId;

        using (var db = new Database(path))
        {
            var store = new AppStore(db);
            var folder = store.CreateFolder("テスト", null);
            var item = new ShortcutItem
            {
                Title = "再起動テスト",
                Target = "https://example.com/",
                TargetType = TargetType.Web,
                FolderId = folder.Id,
                Tags = new List<string> { "タグA", "TagB" },
                Note = "メモ本文",
            };
            store.AddShortcut(item);
            store.RecordUsage(item);
            shortcutId = item.Id;

            Test("スキーマバージョンが設定される", () =>
                AssertEqual(Database.CurrentSchemaVersion, db.GetSchemaVersion(), "user_version"));
        }

        using (var db = new Database(path))
        {
            var store = new AppStore(db);

            Test("再起動後もショートカットが残る", () =>
            {
                var item = store.FindShortcut(shortcutId);
                AssertTrue(item is not null, "ショートカット");
                AssertEqual("再起動テスト", item!.Title, "タイトル");
                AssertEqual("テスト", item.FolderPathText, "フォルダ");
                AssertEqual("メモ本文", item.Note, "メモ");
                AssertEqual(1, item.UsageCount, "使用回数");
            });

            Test("再起動後もタグが残る", () =>
            {
                var item = store.FindShortcut(shortcutId)!;
                AssertEqual(2, item.Tags.Count, "タグ数");
                AssertTrue(item.Tags.Contains("タグA"), "日本語タグ");
                AssertTrue(item.Tags.Contains("TagB"), "英語タグ");
            });

            Test("同じスキーマなら再マイグレーションしない", () =>
                AssertEqual(Database.CurrentSchemaVersion, db.GetSchemaVersion(), "user_version"));
        }

        Test("将来のスキーマのDBは開かずにエラーにする", () =>
        {
            var futurePath = Path.Combine(AppPaths.DataDirectory, "future.sqlite");
            using (var db = new Database(futurePath))
                db.Exec($"PRAGMA user_version={Database.CurrentSchemaVersion + 1};");

            var threw = false;
            try { using var db = new Database(futurePath); }
            catch (InvalidOperationException) { threw = true; }

            AssertTrue(threw, "例外が投げられる");
        });
    }

    // ------------------------------------------------------------- ヘルパー

    /// <summary>1x1 の PNG（アイコンの持ち運び確認用）。</summary>
    private static byte[] MinimalPng() => Convert.FromBase64String(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==");

    private static void TryCleanUp(string directory)
    {
        try
        {
            Microsoft.Data.Sqlite.SqliteConnection.ClearAllPools();
            GC.Collect();
            GC.WaitForPendingFinalizers();
            if (Directory.Exists(directory)) Directory.Delete(directory, recursive: true);
        }
        catch (Exception ex)
        {
            Console.WriteLine($"（一時フォルダを削除できませんでした: {ex.Message}）");
        }
    }
}
