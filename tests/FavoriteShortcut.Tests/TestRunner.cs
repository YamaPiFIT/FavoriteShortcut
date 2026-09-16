using System.Text;

namespace FavoriteShortcut.Tests;

/// <summary>
/// ごく小さなテスト実行基盤。
/// 外部パッケージを増やさずに `dotnet run` だけで検証できるようにするためのもの。
/// </summary>
public static class TestRunner
{
    private static int _passed;
    private static readonly List<string> Failures = new();
    private static string _currentGroup = string.Empty;

    public static void Group(string name)
    {
        _currentGroup = name;
        Console.WriteLine();
        Console.WriteLine($"── {name} ───────────────────────────────");
    }

    public static void Test(string name, Action body)
    {
        try
        {
            body();
            _passed++;
            Console.WriteLine($"  [OK]   {name}");
        }
        catch (Exception ex)
        {
            var message = $"{_currentGroup} / {name}: {ex.Message}";
            Failures.Add(message);
            Console.WriteLine($"  [NG]   {name}");
            Console.WriteLine($"         {ex.Message}");
        }
    }

    public static void AssertTrue(bool condition, string message)
    {
        if (!condition) throw new Exception($"条件が成立しませんでした: {message}");
    }

    public static void AssertEqual<T>(T expected, T actual, string message)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
            throw new Exception($"{message} — 期待値: {expected}, 実際: {actual}");
    }

    public static int Summarize()
    {
        Console.WriteLine();
        Console.WriteLine("════════════════════════════════════════");
        if (Failures.Count == 0)
        {
            Console.WriteLine($"すべて成功しました（{_passed} 件）");
            return 0;
        }

        var sb = new StringBuilder();
        sb.AppendLine($"失敗 {Failures.Count} 件 / 成功 {_passed} 件");
        foreach (var failure in Failures) sb.AppendLine($"  - {failure}");
        Console.WriteLine(sb.ToString());
        return 1;
    }
}
