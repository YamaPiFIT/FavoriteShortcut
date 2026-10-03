using System.IO;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using Microsoft.Data.Sqlite;
using static FavoriteShortcut.Tests.TestRunner;

namespace FavoriteShortcut.Tests;

/// <summary>v1.3.0 で追加した機能の検証。</summary>
internal static class FeatureTests
{
    public static void Run()
    {
        AutoBackupTests();
        ClipboardTests();
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
