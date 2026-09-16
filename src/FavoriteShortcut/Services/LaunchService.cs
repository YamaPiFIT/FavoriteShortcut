using System.Diagnostics;
using System.IO;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

public readonly record struct LaunchResult(bool Success, string? ErrorTitle, string? ErrorDetail)
{
    public static LaunchResult Ok() => new(true, null, null);
    public static LaunchResult Fail(string title, string detail) => new(false, title, detail);
}

/// <summary>
/// ショートカットの起動（§10）。
/// Web は既定ブラウザ、フォルダはエクスプローラー、ファイルは既定のアプリで開く。
/// 対象が見つからない場合もDB上のデータには一切手を触れない（§33）。
/// </summary>
public static class LaunchService
{
    public static LaunchResult Launch(ShortcutItem item)
    {
        var target = (item.Target ?? string.Empty).Trim();
        if (target.Length == 0)
            return LaunchResult.Fail("開けませんでした", "URL / パスが設定されていません。");

        var type = item.TargetType == TargetType.Unknown ? TargetResolver.Detect(target) : item.TargetType;
        var expanded = TargetResolver.Expand(target);

        try
        {
            switch (type)
            {
                case TargetType.Folder:
                    if (!Directory.Exists(expanded))
                        return LaunchResult.Fail("フォルダが見つかりません。", expanded);
                    StartShell(expanded);
                    return LaunchResult.Ok();

                case TargetType.File:
                case TargetType.Application:
                    if (!File.Exists(expanded))
                        return LaunchResult.Fail("ファイルが見つかりません。", expanded);
                    StartShell(expanded, workingDirectory: Path.GetDirectoryName(expanded));
                    return LaunchResult.Ok();

                case TargetType.Web:
                    StartShell(TargetResolver.Normalize(expanded, TargetType.Web));
                    return LaunchResult.Ok();

                default:
                    // 種別が判定できないものは、実体があればパスとして、なければそのままシェルに渡す
                    if (Directory.Exists(expanded) || File.Exists(expanded))
                    {
                        StartShell(expanded);
                        return LaunchResult.Ok();
                    }
                    StartShell(expanded);
                    return LaunchResult.Ok();
            }
        }
        catch (Exception ex)
        {
            AppLog.Warn($"起動に失敗: {target}", ex);
            return LaunchResult.Fail("開けませんでした", $"{expanded}\n\n{ex.Message}");
        }
    }

    /// <summary>エクスプローラーで対象の場所を開く（ファイルなら選択状態にする）。</summary>
    public static LaunchResult RevealInExplorer(ShortcutItem item)
    {
        var type = item.TargetType == TargetType.Unknown ? TargetResolver.Detect(item.Target) : item.TargetType;
        if (type == TargetType.Web)
            return LaunchResult.Fail("この項目には対応していません", "Web サイトにはフォルダがありません。");

        var expanded = TargetResolver.Expand(item.Target ?? string.Empty);

        try
        {
            if (Directory.Exists(expanded))
            {
                Process.Start(new ProcessStartInfo("explorer.exe", $"\"{expanded}\"") { UseShellExecute = true });
                return LaunchResult.Ok();
            }

            if (File.Exists(expanded))
            {
                Process.Start(new ProcessStartInfo("explorer.exe", $"/select,\"{expanded}\"") { UseShellExecute = true });
                return LaunchResult.Ok();
            }

            // 実体が無くても、親フォルダが残っていればそこを開く
            var parent = Path.GetDirectoryName(expanded);
            if (!string.IsNullOrEmpty(parent) && Directory.Exists(parent))
            {
                Process.Start(new ProcessStartInfo("explorer.exe", $"\"{parent}\"") { UseShellExecute = true });
                return LaunchResult.Ok();
            }

            return LaunchResult.Fail("場所が見つかりません。", expanded);
        }
        catch (Exception ex)
        {
            AppLog.Warn($"エクスプローラー表示に失敗: {expanded}", ex);
            return LaunchResult.Fail("開けませんでした", $"{expanded}\n\n{ex.Message}");
        }
    }

    public static void OpenPath(string path)
    {
        try
        {
            StartShell(path);
        }
        catch (Exception ex)
        {
            AppLog.Warn($"パスを開けませんでした: {path}", ex);
        }
    }

    private static void StartShell(string target, string? workingDirectory = null)
    {
        var psi = new ProcessStartInfo(target) { UseShellExecute = true };
        if (!string.IsNullOrEmpty(workingDirectory) && Directory.Exists(workingDirectory))
            psi.WorkingDirectory = workingDirectory;
        Process.Start(psi);
    }
}
