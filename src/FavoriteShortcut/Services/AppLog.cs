using System.IO;
using System.Text;
using FavoriteShortcut.Data;

namespace FavoriteShortcut.Services;

/// <summary>
/// 最低限のファイルログ。落ちた原因を後から追えるようにするためのもので、
/// ログ出力自体が例外を投げてアプリを巻き込まないよう、すべて握りつぶす。
/// </summary>
public static class AppLog
{
    private static readonly object Gate = new();
    private const long MaxBytes = 1024 * 1024; // 1MB を超えたら 1 世代だけ退避

    public static void Info(string message) => Write("INFO", message, null);

    public static void Warn(string message, Exception? ex = null) => Write("WARN", message, ex);

    public static void Error(string message, Exception? ex = null) => Write("ERROR", message, ex);

    private static void Write(string level, string message, Exception? ex)
    {
        try
        {
            lock (Gate)
            {
                var path = AppPaths.LogFile;
                Rotate(path);

                var sb = new StringBuilder();
                sb.Append(DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss.fff"))
                  .Append(" [").Append(level).Append("] ").Append(message);
                if (ex is not null) sb.AppendLine().Append(ex);
                sb.AppendLine();

                File.AppendAllText(path, sb.ToString(), Encoding.UTF8);
            }
        }
        catch
        {
            // ログに失敗してもアプリは動かし続ける
        }
    }

    private static void Rotate(string path)
    {
        try
        {
            var info = new FileInfo(path);
            if (!info.Exists || info.Length < MaxBytes) return;
            var old = path + ".1";
            if (File.Exists(old)) File.Delete(old);
            File.Move(path, old);
        }
        catch
        {
            // ローテーションに失敗しても続行
        }
    }
}
