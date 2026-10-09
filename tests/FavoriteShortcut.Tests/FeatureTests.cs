using System.IO;
using System.Windows.Threading;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using FavoriteShortcut.Views;
using Microsoft.Data.Sqlite;
using static FavoriteShortcut.Tests.TestRunner;

namespace FavoriteShortcut.Tests;

/// <summary>v1.3.0 以降で追加・変更した機能の検証。</summary>
internal static class FeatureTests
{
    public static void Run()
    {
        AutoBackupTests();
        ClipboardTests();
        ThemeTests();
        NetworkWaitTests();
        LaunchTests();
    }

    private static (Database Db, AppStore Store) NewStore(string fileName)
    {
        var db = new Database(Path.Combine(AppPaths.DataDirectory, fileName));
        return (db, new AppStore(db));
    }

    private static ShortcutItem Web(string title, string url) => new()
    {
        Title = title,
        Target = url,
        TargetType = TargetType.Web,
    };

    // ------------------------------------------------------- 自動バックアップ

    private static void AutoBackupTests()
    {
        Group("自動バックアップ");

        var (db, store) = NewStore("auto-backup-test.sqlite");
        using (db)
        {
            var settings = new SettingsService(store);
            var service = new AutoBackupService(store, settings);
            var day1 = new DateTime(2026, 10, 1, 9, 0, 0, DateTimeKind.Local);

            // 他の検証が作ったバックアップと混ざらないようにする
            Directory.CreateDirectory(AppPaths.BackupDirectory);
            foreach (var file in Directory.EnumerateFiles(AppPaths.BackupDirectory))
                if (AutoBackupService.IsAutoBackup(file) || file.EndsWith("auto-backup.json", StringComparison.Ordinal))
                    File.Delete(file);

            store.AddShortcut(Web("最初の項目", "https://example.com/first"));

            Test("まだ一度も作っていなければ作る", () =>
            {
                var fingerprint = service.GetDueFingerprint(day1);
                AssertTrue(fingerprint is not null, "作成が必要と判断する");

                var path = AutoBackupService.CreateBackup(db.FilePath, fingerprint!, day1);
                AssertTrue(File.Exists(path), "ファイルができる");
                AssertTrue(AutoBackupService.IsAutoBackup(path), "自動バックアップとして識別できる");
                AssertEqual(day1, AutoBackupService.LastBackupTime, "前回の日時を覚えている");
            });

            Test("同じ日のうちは作らない", () =>
            {
                store.AddShortcut(Web("同じ日に追加", "https://example.com/same-day"));
                AssertTrue(service.GetDueFingerprint(day1.AddHours(8)) is null, "作らない");
            });

            Test("翌日でも、登録内容が変わっていなければ作らない", () =>
            {
                var day2 = day1.AddDays(1);
                var fingerprint = service.GetDueFingerprint(day2);
                AssertTrue(fingerprint is not null, "前日に追加した分があるので作る");
                AutoBackupService.CreateBackup(db.FilePath, fingerprint!, day2);

                AssertTrue(service.GetDueFingerprint(day1.AddDays(3)) is null, "変更がなければ作らない");
            });

            Test("使っただけ（起動回数の記録）では作らない", () =>
            {
                store.RecordUsage(store.Shortcuts[0]);
                AssertTrue(service.GetDueFingerprint(day1.AddDays(4)) is null, "作らない");
            });

            Test("削除だけでも変更として扱う", () =>
            {
                store.DeleteShortcut(store.Shortcuts.First(s => s.Title == "同じ日に追加"));
                AssertTrue(service.GetDueFingerprint(day1.AddDays(5)) is not null, "作る");
            });

            Test("バックアップの中身は、その時点の登録内容と同じ", () =>
            {
                var path = AutoBackupService.CreateBackup(db.FilePath, service.GetDueFingerprint(day1.AddDays(5))!,
                    day1.AddDays(5));
                AssertEqual(store.Shortcuts.Count, CountRows(path, "shortcuts"), "ショートカット数");
            });

            Test("自動の分は最新 14 回分だけ残し、手動のバックアップは消さない", () =>
            {
                var manual = Path.Combine(AppPaths.BackupDirectory, "20260101-000000-manual.sqlite");
                File.Copy(db.FilePath, manual, overwrite: true);

                for (var i = 0; i < 20; i++)
                {
                    var day = day1.AddDays(10 + i);
                    store.AddShortcut(Web($"日々の追加 {i}", $"https://example.com/daily/{i}"));
                    AutoBackupService.CreateBackup(db.FilePath, service.GetDueFingerprint(day)!, day);
                }

                var autos = Directory.EnumerateFiles(AppPaths.BackupDirectory).Count(AutoBackupService.IsAutoBackup);
                AssertEqual(AutoBackupService.KeepCount, autos, "自動バックアップの数");
                AssertTrue(File.Exists(manual), "手動のバックアップは残る");
                AssertTrue(Directory.EnumerateFiles(AppPaths.BackupDirectory, "*.tmp").Any() == false, "作業用のファイルが残らない");
            });

            Test("設定でオフにすると作らない", () =>
            {
                var s = settings.Current.Clone();
                s.AutoBackup = false;
                settings.Save(s);
                store.AddShortcut(Web("オフの後に追加", "https://example.com/off"));

                AssertTrue(service.GetDueFingerprint(day1.AddDays(60)) is null, "作らない");
            });
        }
    }

    // ------------------------------------------------------- クリップボードから登録

    private static void ClipboardTests()
    {
        Group("クリップボードから登録");

        Test("URL はそのまま登録候補になる", () =>
            AssertEqual("https://example.com/a?b=c", ClipboardTargets.FromText("https://example.com/a?b=c"), "URL"));

        Test("前後の空白や改行は取り除く", () =>
            AssertEqual("https://example.com/", ClipboardTargets.FromText("  https://example.com/  \r\n"), "URL"));

        Test("複数行のときは 1 行目だけを使う", () =>
            AssertEqual("https://example.com/first",
                ClipboardTargets.FromText("https://example.com/first\r\nhttps://example.com/second"), "URL"));

        Test("スキームのないアドレスも登録候補になる", () =>
            AssertEqual("www.example.co.jp/path", ClipboardTargets.FromText("www.example.co.jp/path"), "アドレス"));

        Test("空白を含むパスも登録候補になる", () =>
            AssertEqual(@"C:\Program Files\App\app.exe", ClipboardTargets.FromText(@"C:\Program Files\App\app.exe"), "パス"));

        Test("エクスプローラーの「パスのコピー」の引用符は取り除く", () =>
            AssertEqual(@"C:\Users\Public\Documents", ClipboardTargets.FromText("\"C:\\Users\\Public\\Documents\""), "パス"));

        Test("共有フォルダのパス（UNC）も登録候補になる", () =>
            AssertEqual(@"\\server\share\folder", ClipboardTargets.FromText(@"\\server\share\folder"), "UNC"));

        Test("普通の文章は登録候補にしない", () =>
        {
            AssertTrue(ClipboardTargets.FromText("今日の会議は 10:30 から") is null, "空白を含む文章");
            AssertTrue(ClipboardTargets.FromText("hello") is null, "単語");
            AssertTrue(ClipboardTargets.FromText("memo:abc") is null, "「〇〇:」で始まるだけの文字列");
        });

        Test("空や長すぎる文字列は登録候補にしない", () =>
        {
            AssertTrue(ClipboardTargets.FromText(null) is null, "null");
            AssertTrue(ClipboardTargets.FromText("   ") is null, "空白だけ");
            AssertTrue(ClipboardTargets.FromText("https://example.com/" + new string('a', 3000)) is null, "長すぎる");
        });

        Test("呼び出しキーは既定では割り当てない", () =>
        {
            var settings = new AppSettings();
            AssertEqual(0, settings.ClipboardHotKeyKey, "キー");
            AssertTrue(new HotKeyService().RegisterClipboard(System.Windows.Input.ModifierKeys.None,
                System.Windows.Input.Key.None), "未設定なら登録処理は成功扱い");
        });

        var (db, store) = NewStore("clipboard-settings-test.sqlite");
        using (db)
        {
            Test("設定した呼び出しキーは保存され、次回も読み込まれる", () =>
            {
                var service = new SettingsService(store);
                var s = service.Current.Clone();
                s.ClipboardHotKeyModifiers = (int)(System.Windows.Input.ModifierKeys.Control | System.Windows.Input.ModifierKeys.Alt);
                s.ClipboardHotKeyKey = (int)System.Windows.Input.Key.R;
                service.Save(s);

                var reloaded = new SettingsService(store).Current;
                AssertEqual(s.ClipboardHotKeyModifiers, reloaded.ClipboardHotKeyModifiers, "修飾キー");
                AssertEqual(s.ClipboardHotKeyKey, reloaded.ClipboardHotKeyKey, "キー");
            });
        }
    }

    // ------------------------------------------------------- テーマの「自動」

    private static void ThemeTests()
    {
        Group("テーマの「自動」");

        Test("ライト / ダークは設定どおり", () =>
        {
            AssertTrue(!SystemTheme.IsDark(AppTheme.Light), "ライト");
            AssertTrue(SystemTheme.IsDark(AppTheme.Dark), "ダーク");
        });

        Test("「自動」は Windows のアプリの配色に従う", () =>
            AssertEqual(SystemTheme.WindowsUsesDarkMode(), SystemTheme.IsDark(AppTheme.Auto), "自動"));

        Test("新しく使い始めるときの既定は「自動」", () =>
            AssertEqual(AppTheme.Auto, new AppSettings().Theme, "既定"));

        var (db, store) = NewStore("theme-settings-test.sqlite");
        using (db)
        {
            Test("今まで使っていた人の設定（ライト）はそのまま", () =>
            {
                store.SaveSettingsRaw(new Dictionary<string, string> { ["theme"] = "0" });
                AssertEqual(AppTheme.Light, new SettingsService(store).Current.Theme, "テーマ");
            });

            Test("「自動」を保存して、次回も読み込める", () =>
            {
                var service = new SettingsService(store);
                var s = service.Current.Clone();
                s.Theme = AppTheme.Auto;
                service.Save(s);
                AssertEqual(AppTheme.Auto, new SettingsService(store).Current.Theme, "テーマ");
            });
        }
    }

    // ------------------------------------------- ネットワークを待たない（v1.3.1）

    /// <summary>
    /// 会社の PC などで、プロキシの自動検出や共有フォルダの応答を待って画面が固まらないことの確認。
    /// 実際の通信は行わない（共有フォルダのパスは文字列として扱うだけで、サーバーには接続しない）。
    /// </summary>
    private static void NetworkWaitTests()
    {
        Group("ネットワークを待たない（v1.3.1）");

        Test("アプリはどこにも通信しない（通信用の部品を参照していない）", () =>
        {
            var network = typeof(IconService).Assembly.GetReferencedAssemblies()
                .Select(a => a.Name ?? string.Empty)
                .Where(name => name.StartsWith("System.Net", StringComparison.OrdinalIgnoreCase))
                .ToList();
            AssertTrue(network.Count == 0, "参照している部品: " + string.Join(", ", network));
        });

        Test("共有フォルダのパスを見分けられる", () =>
        {
            AssertTrue(TargetResolver.IsNetworkPath(@"\\server\share\資料"), "UNC");
            AssertTrue(TargetResolver.IsNetworkPath(@"\\?\UNC\server\share\a.txt"), "長いパスの書き方の UNC");
            AssertTrue(!TargetResolver.IsNetworkPath(@"\\?\C:\Windows"), "長いパスの書き方のローカル");
            AssertTrue(!TargetResolver.IsNetworkPath(Environment.GetFolderPath(Environment.SpecialFolder.Windows)), "ローカル");
            AssertTrue(!TargetResolver.IsNetworkPath("https://example.com/"), "Web");
            AssertTrue(!TargetResolver.IsNetworkPath(string.Empty), "空");
        });

        Test("ブラウザのアイコンの一時コピーが残っていたら片付ける（使用中の新しいものは残す）", () =>
        {
            var stale = Path.Combine(Path.GetTempPath(), "fsc-icons-" + Guid.NewGuid().ToString("N"));
            var fresh = Path.Combine(Path.GetTempPath(), "fsc-icons-" + Guid.NewGuid().ToString("N"));
            try
            {
                Directory.CreateDirectory(stale);
                File.WriteAllText(Path.Combine(stale, "Favicons"), "dummy");
                Directory.SetCreationTimeUtc(stale, DateTime.UtcNow.AddHours(-1));
                Directory.CreateDirectory(fresh);

                BrowserFaviconCache.DeleteStaleSnapshots();

                AssertTrue(!Directory.Exists(stale), "古いものは消える");
                AssertTrue(Directory.Exists(fresh), "新しいものは残る");
            }
            finally
            {
                foreach (var dir in new[] { stale, fresh })
                    if (Directory.Exists(dir)) Directory.Delete(dir, recursive: true);
            }
        });

        // サーバーには接続しない。パスは文字列として判定に使うだけ
        const string share = @"\\fsc-test-server\share";

        RunOnSta(() =>
        {
            Test("共有フォルダ上のフォルダは、いったん標準のアイコンを出して実物は裏で読む", () =>
            {
                AssertTrue(IconService.NeedsNetworkRead(share + @"\資料", folder: true, out var placeholder), "裏で読む");
                AssertTrue(placeholder is not null, "すぐ出すアイコン");
            });

            Test("共有フォルダ上の EXE は、いったん標準のアイコンを出して実物は裏で読む", () =>
            {
                AssertTrue(IconService.NeedsNetworkRead(share + @"\tool.exe", folder: false, out var placeholder), "裏で読む");
                AssertTrue(placeholder is not null, "すぐ出すアイコン");
            });

            Test("拡張子で決まるアイコンは、共有フォルダ上のファイルを読まずに済む", () =>
            {
                AssertTrue(!IconService.NeedsNetworkRead(share + @"\見積.fsctest", folder: false, out var placeholder), "読まない");
                AssertTrue(placeholder is not null, "アイコン");
                AssertTrue(ShellIconProvider.IsLoaded(share + @"\別の見積.fsctest", folder: false), "同じ拡張子は取得済み");
            });

            Test("ローカルのフォルダは今までどおりその場で取得する", () =>
                AssertTrue(!IconService.NeedsNetworkRead(
                    Environment.GetFolderPath(Environment.SpecialFolder.Windows), folder: true, out _), "ローカル"));
        });

        var (db, store) = NewStore("network-wait-test.sqlite");
        using (db)
        {
            Test("ブラウザのアイコンを使わない設定では、サイトのアイコンを探さない", () =>
            {
                var settings = new SettingsService(store);
                var s = settings.Current.Clone();
                s.UseBrowserIconCache = false; // 利用者のブラウザのデータには触れない
                settings.Save(s);

                using var icons = new IconService(store, settings);
                var item = Web("例", "https://example.com/");

                var found = icons.EnsureFaviconAsync(item, force: true).GetAwaiter().GetResult();
                AssertTrue(!found, "見つからない扱い");
                AssertTrue(item.IconPath is null, "アイコンは変わらない");
            });

            Test("裏のスレッドで取得したアイコンを、画面のスレッドで表示できる", () =>
            {
                using var icons = new IconService(store, new SettingsService(store));

                // まだ取得していないフォルダで、裏のスレッドに実物を読ませる（ローカルなので通信はしない）
                ShellIconProvider.ClearCache();
                var folder = Environment.GetFolderPath(Environment.SpecialFolder.System);
                var task = icons.RunOnShellThread(() => ShellIconProvider.GetFolderIcon(folder, exists: true));
                AssertTrue(task.Wait(TimeSpan.FromSeconds(30)), "取得が終わる");

                // アイコンを出せない環境（サーバーの検証環境など）では、取得できないこともある
                var icon = task.Result;
                if (icon is null) return;

                AssertTrue(icon.IsFrozen, "どのスレッドからも使えるよう凍結されている");
                RunOnSta(() =>
                {
                    var image = new System.Windows.Controls.Image { Source = icon, Width = 32, Height = 32 };
                    image.Measure(new System.Windows.Size(32, 32));
                    image.Arrange(new System.Windows.Rect(0, 0, 32, 32));
                    var bitmap = new System.Windows.Media.Imaging.RenderTargetBitmap(
                        32, 32, 96, 96, System.Windows.Media.PixelFormats.Pbgra32);
                    bitmap.Render(image); // 別のスレッドのものなら、ここで例外になる
                });
            });
        }
    }

    // ------------------------------------------------------- 開く処理（v1.3.2）

    /// <summary>
    /// ショートカットを開く処理（Windows の ShellExecute）は、ネットワークの状態が悪いと
    /// Windows 側の確認で待たされることがあるので、画面とは別のスレッドで行うことの確認。
    /// 実際には何も開かない（存在しない場所を使うので、Windows に渡す前に止まる）。
    /// </summary>
    private static void LaunchTests()
    {
        Group("開く処理は画面を止めない（v1.3.2）");

        Test("開く処理は専用のスレッド（STA）で行い、呼び出し元を待たせない", () =>
        {
            using var gate = new ManualResetEventSlim(false);
            var caller = Environment.CurrentManagedThreadId;

            // 呼び出し元が先に戻ってくることを確かめるため、合図があるまで処理を終わらせない
            var task = LaunchService.RunOnStaThread(() =>
            {
                gate.Wait(TimeSpan.FromSeconds(30));
                return (Thread: Environment.CurrentManagedThreadId, Apartment: Thread.CurrentThread.GetApartmentState());
            });

            AssertTrue(!task.IsCompleted, "呼び出し元はすぐに戻る");
            gate.Set();
            AssertTrue(task.Wait(TimeSpan.FromSeconds(30)), "処理が終わる");
            AssertTrue(task.Result.Thread != caller, "別のスレッドで行う");
            AssertEqual(ApartmentState.STA, task.Result.Apartment, "STA で行う");
        });

        var missing = Path.Combine(Path.GetTempPath(), "fsc-test-no-such-folder-" + Guid.NewGuid().ToString("N"));

        Test("見つからないフォルダは開かずに理由を返す", () =>
        {
            var result = LaunchService.LaunchAsync(
                new ShortcutItem { Title = "無い", Target = missing, TargetType = TargetType.Folder }).Result;
            AssertTrue(!result.Success, "開かない");
            AssertEqual(Loc.T("Str.Launch.FolderNotFound"), result.ErrorTitle, "理由");
        });

        Test("見つからないファイルは開かずに理由を返す", () =>
        {
            var result = LaunchService.LaunchAsync(
                new ShortcutItem { Title = "無い", Target = Path.Combine(missing, "memo.txt"), TargetType = TargetType.File }).Result;
            AssertTrue(!result.Success, "開かない");
            AssertEqual(Loc.T("Str.Launch.FileNotFound"), result.ErrorTitle, "理由");
        });

        Test("場所が無いものはエクスプローラーで開かずに理由を返す", () =>
        {
            var result = LaunchService.RevealInExplorerAsync(
                new ShortcutItem { Title = "無い", Target = Path.Combine(missing, "memo.txt"), TargetType = TargetType.File }).Result;
            AssertTrue(!result.Success, "開かない");
            AssertEqual(Loc.T("Str.Launch.LocationNotFound"), result.ErrorTitle, "理由");
        });

        Test("Web はエクスプローラーで開けないことを返す", () =>
        {
            var result = LaunchService.RevealInExplorerAsync(Web("例", "https://example.com/")).Result;
            AssertTrue(!result.Success, "開かない");
            AssertEqual(Loc.T("Str.Launch.NotSupported"), result.ErrorTitle, "理由");
        });
    }

    // ------------------------------------------------------- 登録画面（v1.3.1）

    /// <summary>
    /// 登録・編集画面で、共有フォルダのパスの確認を裏で行うことの確認（画面は表示しない）。
    /// 画面の部品を作るためにアプリ本体（Application）を用意するので、他の検証に影響しないよう最後に実行する。
    /// </summary>
    public static void RunEditWindowTests()
    {
        Group("登録画面は共有フォルダを待たない（v1.3.1）");

        var (db, store) = NewStore("edit-window-test.sqlite");
        using (db)
        {
            RunOnSta(() =>
            {
                // 起動処理（二重起動の確認・トレイ・ホットキー）は行わず、画面に必要な部品だけ用意する
                var app = new TestApp { ShutdownMode = System.Windows.ShutdownMode.OnExplicitShutdown };
                foreach (var theme in new[] { "Themes/Light.xaml", "Themes/Controls.xaml" }) // App.xaml と同じもの
                    app.Resources.MergedDictionaries.Add(new System.Windows.ResourceDictionary
                    {
                        Source = new Uri("pack://application:,,,/FavoriteShortcut;component/" + theme),
                    });
                SynchronizationContext.SetSynchronizationContext(
                    new DispatcherSynchronizationContext(Dispatcher.CurrentDispatcher));

                var settings = new SettingsService(store);
                var s = settings.Current.Clone();
                s.UseBrowserIconCache = false; // 利用者のブラウザのデータには触れない
                settings.Save(s);
                using var icons = new IconService(store, settings);

                SetService(app, nameof(App.Db), db);
                SetService(app, nameof(App.Store), store);
                SetService(app, nameof(App.Settings), settings);
                SetService(app, nameof(App.Icons), icons);
                Loc.Apply(AppLanguage.Japanese);

                try
                {
                    Test("共有フォルダのパスは、画面を止めずに裏で確かめる", () =>
                    {
                        // 自分の PC（127.0.0.1）宛てなので、外への通信は起きない
                        var item = new ShortcutItem
                        {
                            Title = "共有", Target = @"\\127.0.0.1\fsc-test-no-such-share\資料", TargetType = TargetType.Folder,
                        };
                        var window = new ShortcutEditWindow(item);

                        AssertEqual(Loc.T("Str.Edit.CheckingPath"), window.TargetInfoText.Text, "確認中の表示");
                        AssertTrue(PumpUntil(() => window.TargetInfoText.Text != Loc.T("Str.Edit.CheckingPath"),
                            TimeSpan.FromSeconds(60)), "確認が終わる");
                        AssertTrue(window.TargetInfoText.Text.StartsWith(
                            Loc.T("Str.Edit.TypeInfo", Loc.T("Str.Type.Folder")), StringComparison.Ordinal),
                            "種類: " + window.TargetInfoText.Text);
                        AssertTrue(window.TargetInfoText.Text.Contains(Loc.T("Str.Edit.PathMissing")), "見つからない表示");
                        AssertTrue(window.IconPreview.Source is not null, "アイコン");
                        window.Close();
                    });

                    Test("ローカルのパスは今までどおりその場で確かめる", () =>
                    {
                        var folder = Environment.GetFolderPath(Environment.SpecialFolder.Windows);
                        var item = new ShortcutItem { Title = "Windows", Target = folder, TargetType = TargetType.Folder };
                        var window = new ShortcutEditWindow(item);

                        AssertEqual(Loc.T("Str.Edit.TypeInfo", Loc.T("Str.Type.Folder")), window.TargetInfoText.Text, "すぐに結果");
                        AssertTrue(window.IconPreview.Source is not null, "アイコン");
                        window.Close();
                    });

                    Test("Web は「ブラウザから取得」ボタンが使える", () =>
                    {
                        var window = new ShortcutEditWindow(Web("例", "https://example.com/"));

                        AssertTrue(window.FetchFaviconButton.IsEnabled, "ボタンが使える");
                        AssertEqual(Loc.T("Str.Edit.FetchFavicon"), window.FetchFaviconButton.Content as string, "ボタン名");
                        AssertEqual(Loc.T("Str.Edit.IconAutoWeb"), window.IconSourceText.Text, "アイコンの説明");
                        window.Close();
                    });
                }
                finally
                {
                    Dispatcher.CurrentDispatcher.InvokeShutdown();
                }
            });
        }
    }

    /// <summary>
    /// 検証用のアプリ本体。WPF は Application を作った時点で起動処理（OnStartup）の呼び出しを予約するので、
    /// 何もしないものに差し替える（二重起動の確認で、動いている本物のアプリに通知を送ったりしないように）。
    /// </summary>
    private sealed class TestApp : App
    {
        protected override void OnStartup(System.Windows.StartupEventArgs e) { }

        protected override void OnExit(System.Windows.ExitEventArgs e) { }
    }

    private static void SetService(App app, string name, object value) =>
        typeof(App).GetProperty(name)!.SetValue(app, value);

    /// <summary>画面のスレッドの処理を進めながら、条件が成り立つまで待つ。</summary>
    private static bool PumpUntil(Func<bool> condition, TimeSpan timeout)
    {
        var deadline = DateTime.UtcNow + timeout;
        while (!condition())
        {
            if (DateTime.UtcNow > deadline) return false;

            var frame = new DispatcherFrame();
            Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.Background,
                new Action(() => frame.Continue = false));
            Dispatcher.PushFrame(frame);
            Thread.Sleep(20);
        }
        return true;
    }

    /// <summary>シェルのアイコン取得や画面の部品は、画面と同じ STA のスレッドで扱う。例外は呼び出し元へ伝える。</summary>
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

        if (error is not null) System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(error).Throw();
    }

    private static long CountRows(string databasePath, string table)
    {
        var builder = new SqliteConnectionStringBuilder
        {
            DataSource = databasePath,
            Mode = SqliteOpenMode.ReadOnly,
            Pooling = false,
        };
        using var connection = new SqliteConnection(builder.ToString());
        connection.Open();
        using var cmd = connection.CreateCommand();
        cmd.CommandText = $"SELECT count(*) FROM {table}";
        return (long)cmd.ExecuteScalar()!;
    }
}
